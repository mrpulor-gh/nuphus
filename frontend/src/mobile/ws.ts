/**
 * 移动端 WS 客户端：事件推流 + 断线重连 + 订阅就绪竞态防护 + 僵尸连接看门狗
 *
 * 协议（mobile_server.rs）：
 * - 连接后先收 {"type":"ws_connected"} 就绪帧——**必须等此帧后才认为订阅激活**
 *   （broadcast 不为迟到订阅者补发，101 握手完成 ≠ 订阅已就位，P1 修过的真实竞态）
 * - 随后收 NuphusEvent JSON 流（serde tagged enum）；服务端每 15s 发 {"type":"heartbeat"}
 * - 未知事件类型必须静默忽略（向前兼容：retry/refine 辅助流程事件暂不走 WS）
 *
 * 活性保障（iOS/微信 WebView 切后台会静默挂起 TCP，onclose 永不触发）：
 * - 任何帧到达（含 parse 失败）都刷新 lastFrameAt——帧本身即活性证明
 * - 10s 看门狗：静默 >45s（3 个心跳周期）判定僵尸连接，斩杀并立即重连
 * - poke()：页面回前台时主动体检（visibilitychange/focus/pageshow 由 App 层注册）
 */

import type { NuphusEvent } from '../core/types'
import { resolveTunnelDeviceId } from './connection'

export type WsStatus = 'connecting' | 'online' | 'offline'

const BACKOFF_MIN_MS = 1000
const BACKOFF_MAX_MS = 30000
/** 看门狗巡检间隔 */
const WATCHDOG_INTERVAL_MS = 10000
/** 静默阈值：服务端心跳 15s/次，45s（3 个周期）无任何帧即判定僵尸连接 */
const SILENCE_THRESHOLD_MS = 45000
/** 客户端活性 ping 间隔：产生 mobile→relay 方向流量，刷新中继隧道 idle 计时。
 *  背景（审计 R3.2）：中继 idle 30s 双向共享刷新（读写任一方向有流量即可），
 *  但服务端心跳是 relay→mobile 的 Text 帧（浏览器不自动应答）——若手机方向
 *  长时间零流量且事件稀疏，仍可能逼近 idle 阈值。浏览器 API 无法发协议级 Ping，
 *  只能发应用层 Text——mobile_server 读循环已忽略文本帧。
 *  15s：与 idle 30s 保持 2 倍余量（原 20s 余量偏小，事件循环繁忙/后台挂起时
 *  可能延迟触发 idle 掐断 → 对话中偶发重连，2026-08-31 排查）。 */
const CLIENT_PING_INTERVAL_MS = 15000

interface WsCallbacks {
  /** 收到业务事件（NuphusEvent，ws_connected 已被本类消费） */
  onEvent: (event: NuphusEvent) => void
  /** 订阅激活（收到就绪帧）——调用方应在此刻重拉历史补齐断线间隙 */
  onReady: () => void
  /** 连接状态变化（驱动导航条状态点） */
  onStatus: (status: WsStatus) => void
}

export class MobileWsClient {
  private ws: WebSocket | null = null
  private backoffMs = BACKOFF_MIN_MS
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private watchdogTimer: ReturnType<typeof setInterval> | null = null
  private pingTimer: ReturnType<typeof setInterval> | null = null
  private lastFrameAt = 0
  private disposed = false

  constructor(
    private readonly token: string,
    private readonly cb: WsCallbacks,
    /** 桌面局域网直连地址（http://ip:port）；缺省 = 当前页面 origin。中继页面切通道后传入以直连桌面 */
    private readonly baseUrl?: string,
  ) {}

  start(): void {
    this.disposed = false
    this.startWatchdog()
    this.connect()
  }

  dispose(): void {
    this.disposed = true
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
    if (this.watchdogTimer) clearInterval(this.watchdogTimer)
    this.watchdogTimer = null
    this.ws?.close()
    this.ws = null
  }

  /** 页面回前台主动体检：连接异常立即重连，健康则无操作 */
  poke(): void {
    if (this.disposed) return
    const ws = this.ws
    if (!ws || ws.readyState === WebSocket.CLOSED || ws.readyState === WebSocket.CLOSING) {
      // 连接已死但未重连（如退避定时器被系统挂起）→ 重置退避立即重连
      if (this.reconnectTimer) {
        clearTimeout(this.reconnectTimer)
        this.reconnectTimer = null
      }
      this.backoffMs = BACKOFF_MIN_MS
      this.connect()
      return
    }
    if (ws.readyState === WebSocket.OPEN && Date.now() - this.lastFrameAt > SILENCE_THRESHOLD_MS) {
      this.killZombie()
    }
    // OPEN 且静默未超时 / CONNECTING 进行中：健康，无操作
  }

  private startWatchdog(): void {
    if (this.watchdogTimer) clearInterval(this.watchdogTimer)
    this.watchdogTimer = setInterval(() => {
      if (this.disposed) return
      const ws = this.ws
      if (!ws) return
      // CLOSED/CLOSING 走 onclose 退避重连路径，看门狗只处理挂死的活连接
      if (ws.readyState !== WebSocket.OPEN && ws.readyState !== WebSocket.CONNECTING) return
      if (Date.now() - this.lastFrameAt > SILENCE_THRESHOLD_MS) {
        this.killZombie() // OPEN 假死 / CONNECTING 握手挂死 统一斩杀
      }
    }, WATCHDOG_INTERVAL_MS)
  }

  /** 僵尸斩杀：旧连接三个 handler 置 null（防 onclose 重入退避）→ close → 按退避重连。
   *  ⚠️ 2026-08-31 修复：此前立即重连 + 重置 backoff——中继/网络持续异常（如隧道
   *  间歇停滞）时会形成「每 45s 斩杀一次」的高频重连循环。改为与 onclose 一致的
   *  指数退避（1s→2s→…→30s），循环频率随退避自然衰减；连接成功收到 ws_connected
   *  时仍会重置退避（见 onmessage），正常恢复不受影响。 */
  private killZombie(): void {
    const ws = this.ws
    if (ws) {
      ws.onmessage = null
      ws.onclose = null
      ws.onerror = null
      ws.close()
    }
    this.ws = null
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.cb.onStatus('offline')
    // 遵循退避：不立即重连、不重置 backoff——避免持续异常时的高频重连循环
    this.reconnectTimer = setTimeout(() => this.connect(), this.backoffMs)
    this.backoffMs = Math.min(this.backoffMs * 2, BACKOFF_MAX_MS)
  }

  private connect(): void {
    if (this.disposed) return
    this.cb.onStatus('connecting')
    // 活性计时从连接发起开始：新连接有 45s 宽限期产出首帧（就绪帧正常秒到）
    this.lastFrameAt = Date.now()
    // 通道地址：baseUrl 存在（切到局域网直连）→ 用其 host；否则当前页面 origin
    const host = this.baseUrl
      ? this.baseUrl.replace(/^https?:\/\//, '').replace(/\/+$/, '')
      : location.host
    const proto = this.baseUrl
      ? this.baseUrl.startsWith('https')
        ? 'wss'
        : 'ws'
      : location.protocol === 'https:'
        ? 'wss'
        : 'ws'
    // 子协议鉴权通道（中继三通道之一）：浏览器不能自定义 Header，用 protocols 传 token。
    // ⚠️ 仅中继（公网 host）需要：mobile_server（局域网/本机直连）不协商子协议，
    // 浏览器带子协议握手要求服务器回 Sec-WebSocket-Protocol 响应头，服务器不回 →
    // 握手失败 error+close 1006 → WS 永久断开（实测定位：refine 弹窗/实时消息全失效）。
    // 判断依据：目标 host 是否私有/本机网段——私有 = mobile_server 直连，不传子协议；
    // 公网 = 中继隧道（r.nuphus.com 等），传子协议供中继鉴权转发。token 的 query
    // 鉴权两种通道都保留（mobile_server token_valid 读 query）。
    const hostname = (
      host.startsWith('[') ? host.slice(0, host.indexOf(']') + 1) : host.split(':')[0]
    ).toLowerCase()
    const isPrivateHost =
      hostname === 'localhost' ||
      hostname === '127.0.0.1' ||
      hostname === '::1' ||
      hostname === '[::1]' ||
      /^10\./.test(hostname) ||
      /^192\.168\./.test(hostname) ||
      /^172\.(1[6-9]|2\d|3[01])\./.test(hostname) ||
      /^169\.254\./.test(hostname) ||
      // Tailscale CGNAT 段（100.64.0.0/10）——用户自建组网后按局域网直连处理
      /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(hostname)
    const protocols = isPrivateHost ? undefined : [`auth.${this.token}`]
    // 多租户归属标记：WS 无法带自定义 Header，query 是唯一通道（中继请求行解析
    // ?device= 优先于 sole-online 决策）。局域网直连同样携带，mobile_server 忽略未知 query。
    const deviceId = resolveTunnelDeviceId()
    const ws = new WebSocket(
      `${proto}://${host}/ws?token=${encodeURIComponent(this.token)}${
        deviceId ? `&device=${encodeURIComponent(deviceId)}` : ''
      }`,
      protocols,
    )
    this.ws = ws

    // 客户端活性 ping（20s）：产生 mobile→relay 方向流量刷新中继隧道 idle 计时，
    // 防空闲 WS 会话被中继 30s idle 掐断（审计 R3.2——服务端心跳是反方向 Text 帧，
    // 浏览器不自动应答，手机→中继方向空闲零流量）。mobile_server 读循环已忽略文本帧。
    if (this.pingTimer) clearInterval(this.pingTimer)
    this.pingTimer = setInterval(() => {
      if (this.disposed) return
      const w = this.ws
      if (w && w.readyState === WebSocket.OPEN) {
        try {
          w.send(JSON.stringify({ type: 'ping' }))
        } catch {
          /* 连接即将关闭，忽略 */
        }
      }
    }, CLIENT_PING_INTERVAL_MS)

    ws.onmessage = e => {
      // 任何帧到达都是活性证明（含无法 parse 的帧），必须先刷新再处理
      this.lastFrameAt = Date.now()
      let data: { type?: string }
      try {
        data = JSON.parse(e.data as string)
      } catch {
        return // 非 JSON 帧：静默忽略
      }
      if (data.type === 'ws_connected') {
        // 订阅激活：重置退避，通知上层补齐历史
        this.backoffMs = BACKOFF_MIN_MS
        this.cb.onStatus('online')
        this.cb.onReady()
        return
      }
      if (data.type === 'heartbeat') {
        return // 服务端活性帧：lastFrameAt 已在上面刷新；不上抛，避免误重置上层事件停滞检测
      }
      // 业务事件交给上层状态机；未知类型在上层静默忽略
      this.cb.onEvent(data as unknown as NuphusEvent)
    }

    ws.onclose = () => {
      if (this.disposed) return
      if (this.pingTimer) {
        clearInterval(this.pingTimer)
        this.pingTimer = null
      }
      this.cb.onStatus('offline')
      // 指数退避重连：1s/2s/4s/... 上限 30s
      this.reconnectTimer = setTimeout(() => this.connect(), this.backoffMs)
      this.backoffMs = Math.min(this.backoffMs * 2, BACKOFF_MAX_MS)
    }

    ws.onerror = () => {
      ws.close() // 统一走 onclose 重连路径
    }
  }
}
