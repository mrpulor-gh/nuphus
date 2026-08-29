/**
 * 移动端 App 根：鉴权门 → 配对引导 / 聊天主界面
 * 闭环：token → 拉历史 → WS 实时流（就绪帧后重拉历史）→ 发送 → 事件回显
 */

import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import { initToken, clearToken, saveToken } from './auth'
import {
  fetchHistory,
  sendMessage,
  fetchIdentity,
  fetchAgentStatus,
  fetchSessions,
  switchSession,
  triggerRefine,
  refineSkip,
  fetchRelayHint,
  startNewChat,
  wfPause,
  wfResume,
  wfStop,
  stopExecution,
  setApiBase,
  getApiBase,
  AuthError,
  type HistoryMessage,
  type ShelfSessions,
} from './api'
import { MobileWsClient, type WsStatus } from './ws'
import {
  chatReducer,
  initialChatState,
  makeOptimisticMessage,
  rid,
  type ChatMessage,
} from './store'
import ChatScreen from './components/ChatScreen'
import MobileErrorBoundary from './components/MobileErrorBoundary'
import RatingSheet from './components/RatingSheet'
import PairingGuide from './components/PairingGuide'
import {
  isPrivateHost,
  isStandalone,
  probeLanDirect,
  getCachedLanUrl,
  getCachedRelayUrl,
  resolveTunnelDeviceId,
  saveRelayCache,
  type ConnectionMode,
} from './connection'
import { useWakeLock } from './useWakeLock'
import { t } from './i18n'

type AuthPhase = 'loading' | 'paired' | 'guide'

/** 请求通知权限（仅支持且未决定时；执行开始时发起——用户正在交互，合理时机） */
function ensureNotificationPermission(): void {
  if (!('Notification' in window)) return
  if (Notification.permission === 'default') {
    void Notification.requestPermission().catch(() => {})
  }
}

/** 后台任务完成通知（仅页面隐藏时发，避免前台打扰；PWA 前台用 UI 状态即可） */
function notifyExecutionDone(result?: string): void {
  if (!('Notification' in window)) return
  if (Notification.permission !== 'granted') return
  const body = (result || '任务已完成').slice(0, 120)
  try {
    // tag 去重：同一任务只留最新通知
    new Notification('Nuphus 执行完成', { body, tag: 'nuphus-exec-done' })
  } catch {
    /* 部分 WebView 不支持构造通知，静默降级 */
  }
}

export default function App() {
  const [phase, setPhase] = useState<AuthPhase>('loading')
  const [authInvalid, setAuthInvalid] = useState(false)
  const [token, setToken] = useState<string | null>(null)
  /**
   * 连接模式：lan=局域网直连 / wan=外网经中继（2026-08 起远程访问免费，无套餐门禁）。
   * null=判定中——中继（公网）页面需先探测局域网直连可达性，期间停留 boot 动画；
   * 探测可达则自动切局域网直连，不可达才落 wan。
   */
  const [connMode, setConnMode] = useState<ConnectionMode | null>(null)
  /** 通道判定完成即可聊（lan/wan 均可） */
  const canChat = connMode !== null
  const [state, dispatch] = useReducer(chatReducer, initialChatState)
  // 任务执行中保持屏幕常亮（仅中继 HTTPS 下生效，局域网 HTTP 静默降级）
  useWakeLock(state.activity.running)
  const [wsStatus, setWsStatus] = useState<WsStatus>('connecting')
  const [historyError, setHistoryError] = useState<string | null>(null)
  /** 轻量弹窗提示（追加指令受理等一句话提醒，不生成消息气泡） */
  const [toast, setToast] = useState<string | null>(null)
  const toastTimerRef = useRef<number | null>(null)
  /** 点评弹窗：assistant 消息「点评」按钮触发，提交记忆评分 */
  const [ratingMsg, setRatingMsg] = useState<ChatMessage | null>(null)
  /** 工作流遥控请求进行中（防重复提交；失败 toast 提示） */
  const [wfControlBusy, setWfControlBusy] = useState(false)
  /** 桌面展示台会话清单镜像（null = 未加载/不可用，隐藏「会话」入口） */
  const [sessions, setSessions] = useState<ShelfSessions | null>(null)
  const [switchBusy, setSwitchBusy] = useState(false)
  /** boot 超时兜底：连接模式判定卡住（隧道半死/慢）时不再无限白屏——
   *  超时后显示错误界面（重试/重新配对），给用户可见出口（2026-08-25 白屏根治）。 */
  const [bootTimeout, setBootTimeout] = useState(false)
  /** 历史拉取失败自动重试：timer + 退避计数（指数退避 3s→30s 上限，中继慢/半死时自愈） */
  const historyRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const historyRetryAttemptRef = useRef(0)

  /** connMode 镜像：发送分流等回调内同步读取，避免 state 变化引发 handleSend 重建 */
  const connModeRef = useRef<ConnectionMode | null>(null)
  useEffect(() => {
    connModeRef.current = connMode
  }, [connMode])

  /** 通道切换冷却（防振荡）：切换后 20s 内禁止反向切换。
   *  背景（实测）：边缘网络下 8s wan→lan 探测 + 2s lan→wan 巡检可交替触发
   *  switchToLan/switchToWan → WS 反复重建 + toast 弹窗 → 页面间隔几秒轻微闪动。 */
  const SWITCH_COOLDOWN_MS = 20000
  const lastSwitchAtRef = useRef(0)
  const markSwitchTime = useCallback(() => {
    lastSwitchAtRef.current = Date.now()
  }, [])
  const withinSwitchCooldown = useCallback(
    () => Date.now() - lastSwitchAtRef.current < SWITCH_COOLDOWN_MS,
    [],
  )

  /** 获取最新桌面局域网地址：优先 /relay-hint（桌面实时下发，IP 变化可感知），缓存兜底。 */
  const resolveLanUrl = useCallback(async (): Promise<string | null> => {
    try {
      const hint = await fetchRelayHint(token ?? '')
      if (hint) {
        // 只缓存非凭据字段（lan_url + relay_url 隧道入口）：hint 里的 caller_token 已无
        // 消费方（POST /task 通道已移除），长期缓存高权限凭据只会扩大 XSS 泄漏面（审计 P3-5 修复）
        saveRelayCache({
          lan_url: hint.lan_url,
          relay_url: hint.tunnel_url,
          device_id: hint.device_id,
        })
        if (hint.lan_url) return hint.lan_url
      }
    } catch {
      /* ignore */
    }
    return getCachedLanUrl()
  }, [token])

  /** 页面内切换到局域网直连通道：更新 REST 基址 + 连接模式。
   *  connMode 变化触发 WS 创建 effect 重建（读 apiBase 已指向局域网）——页面不刷新，
   *  本地消息与执行状态完整保留（刷新会丢执行中气泡、触发重发，绝不在切通道时刷新）。 */
  const switchToLan = useCallback(
    (lanUrl: string) => {
      markSwitchTime()
      setApiBase(lanUrl)
      setConnMode('lan')
    },
    [markSwitchTime],
  )

  /** 明文门导航（v0.1.5 切换逻辑，2026-08-26 大王确认）：探测到局域网（ok/blocked）→
   *  导航到中继明文口（http origin）。PWA 桌面快捷方式无地址栏，导航视觉无感（界面不变）；
   *  http origin 页面 WS/REST 到局域网均不被混合内容拦 → 完整直连（HTTPS 页 WS 被拦导致
   *  掉线反向切回的根治）。明文门探测失败不回跳 → 无循环。 */
  const navigateToLanProbe = useCallback(
    (tok: string) => {
      if (withinSwitchCooldown()) return
      markSwitchTime()
      const host = window.location.hostname
      const dev = resolveTunnelDeviceId()
      const lanProbeUrl = `http://${host}:18081/?token=${encodeURIComponent(tok)}${
        dev ? `&device=${encodeURIComponent(dev)}` : ''
      }`
      window.location.replace(lanProbeUrl)
    },
    [withinSwitchCooldown, markSwitchTime],
  )

  /** 页面内回退到中继通道：REST 基址 + 连接模式置 wan。
   *  connMode 变化触发 WS 创建 effect 重建——页面不刷新，本地消息与执行状态完整保留。
   *  relayBase 缺省 null = 中继 origin 页面回退（基址恢复相对路径，连当前 origin）；
   *  局域网 origin 页面故障转移时传缓存的隧道公网入口（REST/WS 指向绝对地址）。 */
  const switchToWan = useCallback(
    (relayBase?: string) => {
      markSwitchTime()
      setApiBase(relayBase ?? null)
      setConnMode('wan')
    },
    [markSwitchTime],
  )

  // toast 自动消失（4s）
  const showToast = useCallback((text: string) => {
    setToast(text)
    if (toastTimerRef.current) window.clearTimeout(toastTimerRef.current)
    toastTimerRef.current = window.setTimeout(() => setToast(null), 4000)
  }, [])

  /** 刷新会话清单镜像（失败静默 → 入口隐藏，不打扰主流程） */
  const refreshSessions = useCallback((tk: string) => {
    void fetchSessions(tk)
      .then(s => setSessions(s))
      .catch(() => {})
  }, [])

  const wsRef = useRef<MobileWsClient | null>(null)
  // 状态镜像：前台恢复 hook 用 ref 读取，避免 wsStatus 变化触发监听器重注册
  const wsStatusRef = useRef<WsStatus>('connecting')

  /** 历史消息稳定 ID：role+content+timestamp 哈希。
   *  重拉历史时相同消息 id 不变 → React key 稳定 → 不触发消息列表整体重新挂载。
   *  修复（实测）：中继场景隧道抖动 → WS 断线重连 onReady → loadHistory 重拉，
   *  此前每次 rid() 随机 id → 全部 key 变化 → 整列表重渲染 + 滚动丢失 → 页面
   *  间隔几秒轻微闪动（大王 2026-08-14 反馈）。 */
  function stableHistoryId(m: HistoryMessage): string {
    let h = 5381
    const s = `${m.role}\u0000${m.content ?? ''}`
    for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0
    return `h-${h.toString(36)}-${m.timestamp ?? 0}`
  }

  /** 历史粒度对齐（P0 修复）：实时流式把一次执行（多轮 agent 循环）累积为一条
   *  assistant 气泡（store.ts llm_text_delta 无条件累积到 last.streaming，工具/文本
   *  全进 traceItems，execution_completed 才置 streaming=false + 填最终回复）；
   *  但后端 session 每轮 push_assistant 一条（react_loop.rs:757）→ 历史按轮次返回
   *  多条 → 刷新后一个 agent 回复被拆成多个气泡（每个 text 段一个，大王实测）。
   *  折叠连续 assistant（中间无 user 间隔）为一条，content 取组内最后非空
   *  content（最终回复）；追加指令段 [APPEND] 已由后端 extract_history 过滤
   *  （session.rs:61），不会出现在历史里打断折叠。
   *  按大王指示「只拉取最终回复」：历史 assistant 统一不带执行过程
   *  （traceItems 清空）——刷新后显示对话记录（user 问题 + agent 最终回复），
   *  执行状态行/过程弹窗仅实时可见。仅影响手机端，桌面端不受影响。 */
  function foldHistoryAssistants(msgs: HistoryMessage[]): HistoryMessage[] {
    const out: HistoryMessage[] = []
    for (const m of msgs) {
      const prev = out[out.length - 1]
      if (m.role === 'assistant' && prev && prev.role === 'assistant') {
        out[out.length - 1] = {
          ...prev,
          content: m.content && m.content.trim() ? m.content : prev.content,
        }
      } else {
        out.push(m)
      }
    }
    // 只拉取最终回复：历史 assistant 不带执行过程（traceItems 清空）
    return out.map(m => (m.role === 'assistant' ? { ...m, traceItems: [] } : m))
  }

  // ── 历史拉取（首载与每次 WS 就绪后，补齐断线间隙）──
  // 失败自动重试：中继慢/半死时首次失败不卡死——指数退避 3s→6s→12s→24s→30s 重拉，
  // 成功即重置；timer 存在则不重复调度（WS onReady / 前台恢复 / 手动重试会各自触发，
  // 互相去重避免并发风暴）。
  const loadHistory = useCallback(
    async (token: string, opts?: { manual?: boolean }): Promise<boolean> => {
      try {
        const history = await fetchHistory(token)
        // 粒度对齐：折叠连续 assistant（多轮 agent 循环）为一条，与实时气泡一致
        const folded = foldHistoryAssistants(history)
        const messages: ChatMessage[] = folded
          .filter(
            (m: HistoryMessage) =>
              m.role === 'user' || m.role === 'assistant' || m.role === 'refine',
          )
          .map((m: HistoryMessage) => ({
            id: stableHistoryId(m),
            role: m.role as 'user' | 'assistant' | 'refine',
            content: m.content,
            images: m.images && m.images.length > 0 ? m.images : undefined,
            audio: m.audio && m.audio.length > 0 ? m.audio : undefined,
            timestamp: m.timestamp,
            // 后端 Session 存储的完整执行过程（工具/文本/思考）——历史显示完成状态
            traceItems: m.traceItems,
          }))
        dispatch({ type: 'history_merge', messages, manual: opts?.manual })
        setHistoryError(null)
        // 成功：重置退避计数，取消未决重试
        historyRetryAttemptRef.current = 0
        if (historyRetryTimerRef.current) {
          clearTimeout(historyRetryTimerRef.current)
          historyRetryTimerRef.current = null
        }
        return true
      } catch (e) {
        if (e instanceof AuthError) {
          clearToken()
          setAuthInvalid(true)
          setPhase('guide')
          wsRef.current?.dispose()
        } else {
          setHistoryError(t('mobile.historyLoadFailed'))
          // 失败自动重试（指数退避，上限 30s）
          if (!historyRetryTimerRef.current) {
            const attempt = historyRetryAttemptRef.current
            const delay = Math.min(3000 * Math.pow(2, attempt), 30000)
            historyRetryAttemptRef.current = attempt + 1
            historyRetryTimerRef.current = setTimeout(() => {
              historyRetryTimerRef.current = null
              void loadHistory(token)
            }, delay)
          }
        }
        return false
      }
    },
    [],
  )

  /** 手动重新拉取历史（+ 弹窗胶囊触发）：取消未决自动重试，立即重拉并 toast 反馈。
   *  场景：网络抖动/应用切换后历史偶发不显示——无需重启应用，一键刷新到正确历史。 */
  const reloadHistory = useCallback(() => {
    if (historyRetryTimerRef.current) {
      clearTimeout(historyRetryTimerRef.current)
      historyRetryTimerRef.current = null
    }
    historyRetryAttemptRef.current = 0
    // manual=true：跳过 history_merge 的 stalePending 对账——手动重拉是用户主动
    // 恢复历史的操作，WS 断开（running 失真）+ 后端执行中旧快照叠加时会误删
    // 本地 pending 消息（实测：重新拉取反而丢消息，退出重进才正常）。
    void loadHistory(token ?? '', { manual: true }).then(ok => {
      showToast(ok ? t('mobile.historyReloaded') : t('mobile.historyReloadFailed'))
    })
  }, [token, loadHistory, showToast])

  // 卸载清理历史重试 timer（App 根组件几乎不卸载，防御性）
  useEffect(() => {
    return () => {
      if (historyRetryTimerRef.current) clearTimeout(historyRetryTimerRef.current)
    }
  }, [])

  /** 遥控切换桌面当前会话：切的就是电脑端正显示的视图（桌面 rail 同步跟随）。
   *  成功后后端已广播 SessionChanged（本机也会收到一次），此处立即重拉双份数据。
   *  位置约束：依赖 loadHistory/refreshSessions，必须定义于二者之后。 */
  const handleSwitchSession = useCallback(
    (id: string) => {
      if (switchBusy) return
      setSwitchBusy(true)
      switchSession(token ?? '', id)
        .then(res => {
          if (res.ok) {
            showToast('已切换会话')
            void loadHistory(token ?? '')
            refreshSessions(token ?? '')
          } else {
            showToast(res.error)
          }
        })
        .catch(() => showToast('切换失败'))
        .finally(() => setSwitchBusy(false))
    },
    [switchBusy, token, loadHistory, refreshSessions, showToast],
  )

  // 会话清单镜像自愈：WS 未就绪/瞬断期 fetchSessions 失败会留下 null（弹层显示空），
  // 周期补拉 + 回前台即拉，保证入口自愈；成功前「会话」弹层显示暂无记录而非报错
  useEffect(() => {
    if (!token) return
    const tick = () => {
      if (document.visibilityState === 'visible') refreshSessions(token)
    }
    tick()
    const iv = window.setInterval(tick, 30000)
    document.addEventListener('visibilitychange', tick)
    return () => {
      window.clearInterval(iv)
      document.removeEventListener('visibilitychange', tick)
    }
  }, [token, refreshSessions])

  // ── 启动：解析 token → 首拉历史 → 建 WS ──
  useEffect(() => {
    const tokenStr = initToken()
    if (!tokenStr) {
      setPhase('guide')
      return
    }
    setToken(tokenStr)
    setPhase('paired')
  }, [])

  // ── boot 超时兜底（白屏根治）：进入 boot 态后 12s 连接模式仍未就绪 → 显示错误界面 ──
  // 场景：中继隧道半死/极慢时 connMode 判定依赖 fetchRelayHint 往返，可能长时间卡 null；
  // 此前纯空白 mobile-boot 无任何出口，用户误以为应用死了（2026-08-25 实测白屏）。
  useEffect(() => {
    const inBoot = phase === 'loading' || (phase === 'paired' && connMode === null)
    if (!inBoot) {
      setBootTimeout(false)
      return
    }
    setBootTimeout(false)
    const id = setTimeout(() => setBootTimeout(true), 12000)
    return () => clearTimeout(id)
  }, [phase, connMode])

  // ── 连接模式判定：私有 hostname → lan；公网（中继入口）→ 统一先按中继打开界面 ──
  // 设计意图（2026-08 用户确认）：所有中继入口先显示中继界面——全屏原生、启动零等待、
  // 无跨 origin 跳转；局域网探测不阻塞启动，由 WS/历史 effect 在拉取历史前并行发起
  // （本地网络 → 页面内切回直连，历史走局域网更快；异地 → 保持中继）。历史拉取逻辑不变，
  // 探测只是路线选择的加速。
  useEffect(() => {
    if (phase !== 'paired' || !token) return
    let cancelled = false
    const resolve = async () => {
      // 页面本身来自局域网/本机 → lan，无需探测。
      // 后台顺手拉一次 hint：缓存隧道公网入口（relay_url）+ 同步套餐——纯局域网用户
      // 此前无任何缓存，离开 WiFi 后无法页面内故障转移到中继（只能重开中继 URL）。
      if (isPrivateHost(window.location.hostname)) {
        if (cancelled) return
        setConnMode('lan')
        void fetchRelayHint(token).then(hint => {
          if (!hint) return
          saveRelayCache({
            lan_url: hint.lan_url,
            relay_url: hint.tunnel_url,
            device_id: hint.device_id,
          })
        })
        return
      }
      // 公网（中继入口）：统一先按中继打开——界面立即出、全屏稳定（standalone 无跨
      // origin 跳转；浏览器/扫码入口同样先中继界面，视觉原生统一）。
      if (cancelled) return
      setConnMode('wan')
      // 后台顺手拉一次 hint：缓存隧道入口 + 局域网地址（不阻塞启动；切回局域网用）
      void fetchRelayHint(token).then(hint => {
        if (hint)
          saveRelayCache({
            lan_url: hint.lan_url,
            relay_url: hint.tunnel_url,
            device_id: hint.device_id,
          })
      })
    }
    void resolve()
    return () => {
      cancelled = true
    }
  }, [phase, token])

  useEffect(() => {
    // connMode===null：连接模式判定中（中继页探测局域网），不建 WS——探测完成切通道后
    // （connMode 变化）本 effect 重跑，用最新 apiBase（已指向局域网）重建，避免先连中继 origin
    if (phase !== 'paired' || !token || !canChat || connMode === null) return

    // 中继入口：拉取历史前并行发起一次局域网探测（路线选择的加速，不阻塞历史拉取、
    // 不改历史拉取逻辑——历史照常走当前通道发起）。本地网络探测 ok → 页面内切回直连，
    // connMode 变化触发本 effect 重跑 → WS/历史走局域网（更快）；异地/被拦 → 保持中继，
    // 交由下方 8s 巡检持续探测（blocked 导航等完整逻辑在那里，对齐 v0.1.5）。
    // 快路径：缓存 lan_url 直接探测（本地网络 <100ms，不依赖隧道）；缓存缺失/失败
    // 才走 resolveLanUrl（hint 实时地址，IP 变化可感知）→ 探测。
    if (connMode === 'wan') {
      void (async () => {
        try {
          const cached = getCachedLanUrl()
          if (cached) {
            const r0 = await probeLanDirect(cached, token)
            if (r0 === 'ok' && !withinSwitchCooldown()) {
              switchToLan(cached)
              return
            }
          }
          const lanUrl = await resolveLanUrl()
          if (!lanUrl) return
          const r = await probeLanDirect(lanUrl, token)
          if (r === 'ok' && !withinSwitchCooldown()) {
            switchToLan(lanUrl)
          }
        } catch {
          /* 探测失败不阻塞历史拉取 */
        }
      })()
    }

    // 历史首拉：WS 建立前直接拉一次（v0.1.5 对齐）——onReady 每次重连也会重拉；
    // 首拉在此保证界面立即有内容，不依赖 WS 握手成功。
    void loadHistory(token)
    // 拉取身份显示名（桌面端 soul 配置经后端下发）；失败静默（默认 Nuphus）
    void fetchIdentity(token)
      .then(id => dispatch({ type: 'identity', identity: id }))
      .catch(() => {})

    // 通道：切到局域网后 apiBase 指向桌面 → WS 直连（页面不刷新，本地状态保留）；
    // 未切（apiBase=null）→ 当前 origin。
    const client = new MobileWsClient(
      token,
      {
        onEvent: event => {
          // ⚠️ 不使用轻量快照（session_snapshot）：2026-08-26 大王裁定「轻量快照方案不可取」——
          // 只含状态不含数据，执行中（running）时手机端无内容可跟随，事件丢失就永远卡执行中、
          // 无法跟随电脑端完成。同步完全回归 v0.1.5 逻辑：onReady 每次连接 loadHistory 拉完整
          // 历史 + WS 实时事件累积（delta/tool_call/completed）；执行状态由 fetchAgentStatus 恢复。
          // 同 WiFi（局域网直连）带宽充足，完整历史拉取瞬时完成，无需任何快照。
          // 后台任务完成通知：执行完成时若页面在后台，发系统通知提醒
          if (event.type === 'execution_started') {
            ensureNotificationPermission()
          }
          // 会话镜像跟随：桌面 rail（或本机遥控）切换了当前会话 → 重拉历史呈现
          if (event.type === 'session_changed') {
            void loadHistory(token ?? '')
            refreshSessions(token ?? '')
            showToast('桌面已切换会话')
          }
          if (event.type === 'execution_completed' && document.visibilityState === 'hidden') {
            notifyExecutionDone(event.output?.result_message)
          }
          dispatch({ type: 'event', event })
        },
        onReady: () => {
          // 订阅激活（含每次重连）：立即拉取历史补齐断线间隙（v0.1.5 逻辑）。
          // ⚠️ 不做「等 snapshot 再拉」——snapshot 只负责 welcome/running 快速呈现，
          // 历史以 loadHistory 全量拉取为准；onReady 每次连接都拉 → 与桌面永远一致，
          // 不同步根治（2026-08-26 实测：快照清空 + 依赖拉取时机 = 断线重连丢历史）。
          refreshSessions(token)
          void loadHistory(token)
          // 恢复执行状态：broadcast 事件不为迟到订阅者补发，刷新/断线间隙的
          // execution_started/completed 会丢失——据此恢复 running，让后续 delta 正常
          // 累积气泡、完成结果经历史拉取落地（修复刷新后回复看不到）。
          // refine_active 同理：恢复「正在提炼」态，结束事件照常复位。
          void fetchAgentStatus(token)
            .then(s => {
              dispatch({ type: 'sync_running', running: s.running })
              dispatch({ type: 'refine_state', refining: s.refine_active === true })
            })
            .catch(e => console.warn('[mobile] fetchAgentStatus failed:', e))
        },
        onStatus: s => {
          wsStatusRef.current = s
          setWsStatus(s)
        },
      },
      getApiBase() ?? undefined,
    )
    client.start()
    wsRef.current = client
    return () => {
      client.dispose()
    }
  }, [
    phase,
    token,
    loadHistory,
    canChat,
    connMode,
    resolveLanUrl,
    switchToLan,
    withinSwitchCooldown,
  ])

  // ── LAN 断连自动回退中继：局域网直连模式下 WS 离线（离开 WiFi）持续一段时间
  // 且局域网探测不可达 → 页面内切回中继通道（WS 重建），继续可用。
  // 中继 origin 页面：基址回退相对路径（连当前 origin）；局域网 origin 页面：
  // 基址切到缓存的隧道公网入口（无缓存 relay_url 时保持原行为：页面内无路可退，
  // 需重开中继 URL）。
  // 注意：离线起始用 ref 记录，不能以 wsStatus 为依赖做 setTimeout——WS 指数退避
  // 重连会让状态在 offline/connecting 间振荡，timer 每次都被 cleanup 重置永不触发。
  const offlineSinceRef = useRef<number | null>(null)
  useEffect(() => {
    // 只有真正恢复 online 才清空离线起始；connecting 是重连尝试中，必须保持——
    // 否则 offline→connecting 振荡会让累计时长反复清零，回退永不触发。
    // 非 online（offline / connecting）都记录起始：WS 断线后 offline 窗口极短
    // （onclose 报 offline → 1s 后 connect 报 connecting），仅记 offline 会在
    // 振荡相位对齐时漏记（巡检 2s 间隔可能永远采样不到 offline，回退永不触发）。
    if (wsStatus === 'online') {
      offlineSinceRef.current = null
    } else {
      offlineSinceRef.current ??= Date.now()
    }
  }, [wsStatus])

  useEffect(() => {
    if (phase !== 'paired' || !token || connMode !== 'lan') return
    // 局域网 origin 页面：有缓存的隧道入口才有故障转移目标，
    // 否则离线巡检无意义（保持原「无路可退」行为，不空耗探测）
    const lanOrigin = isPrivateHost(window.location.hostname)
    if (lanOrigin && !getCachedRelayUrl()) return
    const iv = setInterval(async () => {
      // 仅离线累计超 10s 才探测（滞回：原 5s 在边缘网络下过灵敏 → 与 wan→lan 探测
      // 交替触发形成通道振荡，页面反复闪动）。判定条件必须是非 online（offline 或
      // connecting）——WS 断线后 offline/connecting 各约 1s 交替（重连退避），若要求
      // 采样到 offline 才探测，2s 巡检间隔可能与 connecting 相位对齐而永远跳过
      // （实测 P0：切 wan 永不触发，apiBase 停留局域网 → 5G 下 POST 挂起 15s →
      // 灰气泡永久 pending）。
      if (wsStatusRef.current === 'online') return
      const since = offlineSinceRef.current
      if (!since || Date.now() - since < 10000) return
      const lanUrl = getApiBase() ?? getCachedLanUrl()
      if (!lanUrl) return
      const reachable = await probeLanDirect(lanUrl, token ?? '')
      // 仅 ok 视为可达；blocked（Mixed Content 拦截）在本场景按不可达处理——本巡检是
      // 「lan 通道掉线反向切 wan」路径，若页面是 HTTPS 中继入口，blocked 说明局域网探测
      // 被浏览器拦（不代表真不可达），但当前 WS 已 offline，保守切 wan 走中继兜底，
      // 避免原地等待；WAN→LAN 切回巡检（下方 effect）会再处理 blocked 导航回直连。
      if (reachable === 'ok') return
      // 冷却：刚切到 lan 不久（WS 重建期 offline 属正常）不反向切，避免振荡
      if (withinSwitchCooldown()) return
      if (lanOrigin) {
        // 局域网 origin：上面已校验 relay_url 存在，这里再取一次保底
        const relayUrl = getCachedRelayUrl()
        if (!relayUrl) return
        // ⚠️ 基址必须保持裸 origin：apiBase 是字符串前缀拼接（resolveApi），
        // 带 query 会打废后续请求。多设备归属由服务端决策（唯一在线自动跟随/
        // 引导页）兜底，不靠客户端拼标记（实测拼接事故）。
        switchToWan(relayUrl)
      } else {
        switchToWan()
      }
      showToast(t('mobile.lanDisconnectedSwitchWan'))
    }, 2000)
    return () => clearInterval(iv)
  }, [phase, token, connMode, switchToWan, showToast, withinSwitchCooldown])

  // ── 完成轮询兜底（2026-08-26）：执行中定期查后端 busy，释放即收尾 ──
  // 手机端 running 依赖 WS 推送的 execution_completed 事件收尾；中继慢链路下事件
  // 可能丢失（WS 在线不重连，onReady 不触发）→ running 永 true 卡执行态、无法跟随
  // 电脑端完成（大王实测）。此处执行中每 5s 轮询后端状态（fetchAgentStatus = busy），
  // 检测到已完成 → sync_running(false) 收口 + loadHistory 补拉最终结果。
  useEffect(() => {
    if (phase !== 'paired' || !token || !state.activity.running) return
    const iv = setInterval(() => {
      fetchAgentStatus(token)
        .then(s => {
          if (!s.running) {
            dispatch({ type: 'sync_running', running: false })
            void loadHistory(token)
          }
        })
        .catch(() => {})
    }, 5000)
    return () => clearInterval(iv)
  }, [phase, token, state.activity.running, loadHistory])

  // ── WAN → LAN 自动切回：重连 WiFi 后，wan 模式下定时探测局域网直连，可达则页面内切回
  // lan（免费直连，绕过可能半死的中继隧道）。重连 WiFi 白屏的根治：此前 connMode 只在
  // 页面加载时判定一次，重连 WiFi 后页面仍走半死中继，不探测局域网 → 永久白屏。
  // 不限页面 origin：局域网 origin 页面故障转移到中继后（connMode=wan），
  // 回到 WiFi 同样经此 effect 切回直连。
  useEffect(() => {
    if (phase !== 'paired' || !token || connMode !== 'wan') return
    // ⚠️ 不依赖 wsStatus：WS 可能一直 connecting（中继/直连建立中），等它 = 死锁
    // 永不巡检、永不切直连（2026-08-26 实测「电脑不推」根因）。周期性探测独立执行。
    let cancelled = false
    // 滞回：连续 2 次探测成功才切回 lan——避免 WiFi 弱/刚连上时单次抖动触发
    // switchToLan → WS 重建 → 反向回退 → 振荡闪动
    let consecutiveOk = 0
    const probe = async () => {
      // resolveLanUrl 优先 fetchRelayHint（经隧道），隧道半死时回退 getCachedLanUrl
      // （localStorage 缓存的 lan_url）→ 仍能拿到局域网地址去探测。
      const lanUrl = await resolveLanUrl()
      if (cancelled || !lanUrl) return
      const r = await probeLanDirect(lanUrl, token)
      if (r === 'ok') {
        consecutiveOk += 1
        if (consecutiveOk >= 2 && !withinSwitchCooldown()) {
          if (cancelled) return
          switchToLan(lanUrl)
          showToast(t('mobile.switchedBackLan'))
        }
      } else if (r === 'blocked') {
        // Mixed Content 拦截（HTTPS 中继页）：不代表不可达——回到 WiFi 后实际可直连。
        // 立即导航到中继 HTTP 口重载，App 以 http origin 重新 resolve 即切回直连。
        // 仍在异地（外网）时导航后同样走中继，仅多一次跳转，不打断使用。
        // ⚠️ standalone PWA（主屏幕图标）跨 origin 导航丢全屏 → 不导航，继续中继；
        //    iOS 下 HTTPS 页探测 HTTP 局域网恒被拦（blocked），跳过即保持全屏。
        if (cancelled) return
        if (isStandalone()) {
          consecutiveOk = 0
        } else {
          navigateToLanProbe(token)
        }
        return
      } else {
        consecutiveOk = 0
      }
    }
    void probe()
    const iv = setInterval(probe, 8000)
    return () => {
      cancelled = true
      clearInterval(iv)
    }
  }, [
    phase,
    token,
    connMode,
    resolveLanUrl,
    switchToLan,
    showToast,
    withinSwitchCooldown,
    navigateToLanProbe,
  ])

  // ── 前台恢复探测：切后台会静默挂起 TCP（iOS/微信 WebView），回前台必须主动体检 ──
  useEffect(() => {
    if (phase !== 'paired' || !token || !canChat) return
    const onForeground = () => {
      wsRef.current?.poke()
      // 仍在线：WS 静默期间可能有事件缺口，顺手重拉历史补齐
      if (wsStatusRef.current === 'online') {
        void loadHistory(token)
        // 同步执行状态：onReady 的 fetchAgentStatus 只在 WS 重连时触发，
        // 若 WS 假在线（TCP 未断但事件有缺口）回前台不会重连——running 状态
        // 就无法恢复/收口（实测：旧后端 404 → catch 静默 → 执行栏永不恢复）。
        void fetchAgentStatus(token)
          .then(s => dispatch({ type: 'sync_running', running: s.running }))
          .catch(e => console.warn('[mobile] foreground agent-status sync failed:', e))
      }
    }
    const onVisibility = () => {
      if (document.visibilityState === 'visible') onForeground()
    }
    document.addEventListener('visibilitychange', onVisibility)
    window.addEventListener('focus', onForeground)
    window.addEventListener('pageshow', onForeground)
    return () => {
      document.removeEventListener('visibilitychange', onVisibility)
      window.removeEventListener('focus', onForeground)
      window.removeEventListener('pageshow', onForeground)
    }
    // canChat 必须在依赖里：guard 用到它——非 Pro 用户在 ProUpgrade 页重试成功切回
    // lan 后，若漏掉依赖，前台恢复监听从未注册（审计 P3-3 修复）
  }, [phase, token, canChat, loadHistory])

  // ── 发送 ──
  // 按执行状态分流（不绕路、不添加再去除）：
  // - agent 未执行（running=false）→ 创建乐观气泡正常发送
  // - agent 执行中（running=true）→ 不创建气泡，直接发送追加指令，弹窗「成功/失败」
  const handleSend = useCallback(
    async (content: string, opts?: { images?: string[]; mode?: string }) => {
      if (!token) return

      // ── 执行中：追加指令——直接发送，弹窗结果，不生成消息气泡 ──
      if (state.activity.running) {
        try {
          const result = await sendMessage(token, content, rid(), opts)
          if (result.ok) {
            // 暂停态追加：后端写 PauseDecision::Append 后 agent 已继续执行，
            // 本地复位暂停态，避免 UI 停留在暂停菜单。
            if (result.appended) dispatch({ type: 'pause_reset' })
            // 极简状态提示：让用户直接知道消息发送成功
            if (result.imagesDropped) {
              // 追加通道为纯文本，图片被后端丢弃——必须告知，否则用户以为带图成功
              showToast(t('mobile.acceptedImagesNotSent'))
            } else if (result.timeout) {
              // 15s 超时 ≠ 失败：受理状态未知，如实告知等待确认（审计 P3-2 修复）
              showToast(t('mobile.sentAwaitConfirm'))
            } else {
              showToast(t('mobile.messageSent'))
            }
          } else {
            showToast(t('mobile.messageSendFailed'))
          }
        } catch (e) {
          const reason =
            e instanceof Error && e.name === 'AbortError'
              ? t('mobile.sendTimeout')
              : t('mobile.sendFailedCheckNetwork')
          showToast(reason)
        }
        return
      }

      // ── 空闲：乐观回显 → POST（send_id 去重）→ 失败撤销 ──
      const optimistic = makeOptimisticMessage(content, opts?.images)
      dispatch({ type: 'optimistic', message: optimistic })
      // ⚠️ 2026-08-26 移除「发送前探测局域网」：resolveLanUrl 经中继 fetchRelayHint
      // 慢（国际链路 20s 超时）会阻塞发送 → 实测「发消息卡死」（气泡永久 pending）。
      // 发送立即走当前通道（中继/直连），通道切换由 8s 巡检/进入探测负责——发送不等待探测。
      try {
        const result = await sendMessage(token, content, optimistic.id, opts)
        if (result.ok && result.timeout) {
          // 响应超时（后端同步等待整轮执行完成，15s 内大概率未返回）：
          // 保留乐观气泡 pending，等待 WS 事件（user_message_received /
          // execution_started）确认——绝不撤销，撤销曾导致 user 消息消失 +
          // 用户误判重试 + agent 双气泡。
          // ⚠️ 弱网兜底：蜂窝 CGNAT 下 WS 可能反复闪断，推送确认永远等不到 →
          // 气泡永久 pending（实测「发消息卡死」观感）。8s 后主动拉一次历史
          // 对账——消息已被后端受理就会出现在历史里，气泡随之落定。
          window.setTimeout(() => {
            void loadHistory(token ?? '')
          }, 8000)
        } else if (!result.ok) {
          dispatch({
            type: 'send_failed',
            id: optimistic.id,
            reason: result.busy ? t('mobile.taskRunningPleaseRetry') : result.error,
          })
        } else if (result.appended) {
          // 竞态兜底：发送瞬间后端刚转为 busy（极窄窗口），追加被受理。
          // 追加语义 = 不显示独立气泡，仅弹窗提示发送状态。
          dispatch({ type: 'remove_optimistic', id: optimistic.id })
          showToast(t('mobile.messageSent'))
          dispatch({ type: 'pause_reset' })
        }
      } catch (e) {
        // 网络异常/超时：撤销乐观气泡并提示，绝不静默失败（P0：sending 卡死曾导致发送永久无效）
        const isTimeout = e instanceof Error && e.name === 'AbortError'
        // LAN 直连模式下发送网络失败（非超时）：大概率已离开 WiFi → 探测局域网不可达则
        // 页面内切到中继通道（中继 origin 回相对基址；局域网 origin 用缓存隧道入口），
        // 用户重发时走中继（handleSend wan 分支自动接管）
        if (!isTimeout && connModeRef.current === 'lan') {
          const lanUrl = getApiBase() ?? getCachedLanUrl()
          // 探测非 ok（timeout=真不可达 / blocked=被拦但发送已失败）→ 反向切 wan 兜底
          const r = lanUrl ? await probeLanDirect(lanUrl, token ?? '') : 'timeout'
          if (r !== 'ok') {
            const relayUrl = isPrivateHost(window.location.hostname)
              ? getCachedRelayUrl()
              : undefined
            if (!isPrivateHost(window.location.hostname) || relayUrl) {
              switchToWan(relayUrl ?? undefined)
              showToast(t('mobile.lanDisconnectedSwitchWanRetry'))
            }
          }
        }
        const reason = isTimeout ? t('mobile.sendTimeout') : t('mobile.sendFailedCheckNetwork')
        dispatch({ type: 'send_failed', id: optimistic.id, reason })
      }
    },
    [token, showToast, state.activity.running, resolveLanUrl, switchToLan, switchToWan, canChat],
  )

  // ── 配对引导：PairingGuide 已用密码换到 token，这里只落盘 token 并进入聊天（App 不接触密码）──
  const handlePair = useCallback((tok: string) => {
    saveToken(tok)
    setAuthInvalid(false)
    setToken(tok.trim())
    setPhase('paired')
  }, [])

  // ── 工作流遥控：POST /workflow-*（复用引擎控制命令），成功/失败 toast 反馈，busy 防重复提交 ──
  const runWorkflowControl = useCallback(
    async (fn: (workflowId: string) => Promise<void>, okText: string) => {
      const wfId = state.workflowRun?.lastWorkflowId
      if (!wfId || wfControlBusy) return
      setWfControlBusy(true)
      try {
        await fn(wfId)
        showToast(okText)
      } catch {
        showToast(t('mobile.wfControlFailed'))
      } finally {
        setWfControlBusy(false)
      }
    },
    [state.workflowRun?.lastWorkflowId, wfControlBusy, showToast],
  )

  if (phase === 'loading' || (phase === 'paired' && connMode === null)) {
    // boot 超时兜底：不再无限白屏——给「重试」与「重新配对」两个可见出口。
    // 重试 = 重新加载页面（隧道恢复后即正常进入）；重新配对 = 清 token 回配对页。
    if (bootTimeout) {
      return (
        <div className="mobile-boot mobile-boot-error">
          <div className="mobile-boot-card">
            <h2>{t('mobile.bootTimeoutTitle')}</h2>
            <p>{t('mobile.bootTimeoutDesc')}</p>
            <div className="mobile-boot-actions">
              <button
                type="button"
                className="mobile-boot-btn"
                onClick={() => window.location.reload()}
              >
                {t('mobile.retry')}
              </button>
              <button
                type="button"
                className="mobile-boot-btn is-secondary"
                onClick={() => {
                  clearToken()
                  wsRef.current?.dispose()
                  setAuthInvalid(false)
                  setPhase('guide')
                }}
              >
                {t('mobile.repair')}
              </button>
            </div>
          </div>
        </div>
      )
    }
    return <div className="mobile-boot" />
  }
  if (phase === 'guide') {
    return <PairingGuide invalid={authInvalid} onPair={handlePair} />
  }
  return (
    <>
      {/* 渲染错误兜底：历史消息大量加载/低端 WebView 渲染崩溃时，白屏变可恢复提示 */}
      <MobileErrorBoundary>
        <ChatScreen
          messages={state.messages}
          activity={state.activity}
          wsStatus={wsStatus}
          historyError={historyError}
          onRetryHistory={() => {
            // 手动重试：取消未决自动重试，立即重拉（重置退避）
            if (historyRetryTimerRef.current) {
              clearTimeout(historyRetryTimerRef.current)
              historyRetryTimerRef.current = null
            }
            historyRetryAttemptRef.current = 0
            void loadHistory(token ?? '')
          }}
          onReloadHistory={reloadHistory}
          pendingConfirm={state.pendingConfirm}
          pendingRefine={state.pendingRefine}
          pendingUserInput={state.pendingUserInput}
          refining={state.refining}
          token={token ?? ''}
          assistantName={state.identity?.assistantName}
          model={state.model}
          tokenUsage={state.tokenUsage}
          workflowRun={state.workflowRun}
          wfControlBusy={wfControlBusy}
          onWorkflowPause={() =>
            void runWorkflowControl(wfId => wfPause(token ?? '', wfId), t('mobile.wfPausedToast'))
          }
          onWorkflowResume={() =>
            void runWorkflowControl(wfId => wfResume(token ?? '', wfId), t('mobile.wfResumedToast'))
          }
          onWorkflowTerminate={() =>
            void runWorkflowControl(wfId => wfStop(token ?? '', wfId), t('mobile.wfStoppedToast'))
          }
          onWorkflowDismiss={() => dispatch({ type: 'workflow_clear' })}
          onStopExecution={() => {
            // 加确认弹窗：避免手机端误触终止（执行中发送按钮常与终止相邻）
            if (!window.confirm(t('input.forceStopConfirm'))) return
            // 直接终止（POST /stop）：紧急操作，无需暂停 action_id
            void stopExecution(token ?? '')
              .then(res => {
                if (res.status === 'stopping' || res.status === 'terminated') {
                  showToast(t('mobile.statusStopped'))
                }
              })
              .catch(() => showToast(t('mobile.stopFailed')))
          }}
          onNewChat={() => {
            // 单一路径：手机新建 = 遥控桌面执行权威新建（复用桌面 new_chat_session_cmd），
            // 后端经 SessionChanged 事件广播，手机收到后跟随显示欢迎页。
            // HTTP 成功响应兜底清一次本端视图（WS 事件可能瞬时漏收，幂等）。
            if (!token) return
            void startNewChat(token).then(r => {
              if (r.ok) {
                dispatch({ type: 'new_chat' })
                showToast(t('mobile.newChatStarted'))
                // 兜底重拉：即使 SessionChanged 漏收，本端也立即呈现新会话欢迎页
                void loadHistory(token)
                refreshSessions(token)
              } else {
                // busy/append_pending：后端 guard_switch 已拒绝（执行中禁止新建），
                // 前端给明确提示而非裸错误码；其余失败走通用文案
                showToast(
                  r.error === 'busy' || r.error === 'append_pending'
                    ? t('mobile.newChatBusy')
                    : r.error || t('mobile.newChatFailed'),
                )
              }
            })
          }}
          onDisconnect={() => {
            // 断开连接：清除 token + WS，回到配对页
            clearToken()
            wsRef.current?.dispose()
            setAuthInvalid(false)
            setPhase('guide')
          }}
          onSend={handleSend}
          onRateMessage={setRatingMsg}
          sessions={sessions}
          onSwitchSession={handleSwitchSession}
          onUserInputResolved={submitted => {
            dispatch({ type: 'user_input_resolved' })
            showToast(submitted ? t('mobile.submitted') : t('mobile.cancelled'))
          }}
          onModelChanged={m => {
            // 模型卡切换成功后立即同步 store（不等下次 session_info 事件）
            dispatch({ type: 'set_model', model: m })
          }}
          onConfirmResolved={approved => {
            dispatch({ type: 'confirm_resolved' })
            // 安全决策反馈：让用户明确知道点击已生效（已允许/已拒绝）
            showToast(approved ? t('mobile.allowedContinue') : t('mobile.deniedIntercepted'))
          }}
          onRefineConfirm={() => {
            dispatch({ type: 'refine_resolve' })
            dispatch({ type: 'refine_state', refining: true })
            // 触发后端提炼（/refine）；成功/失败经 WS 事件（session_refined / 错误）恢复状态
            triggerRefine(token ?? '').catch(() => {
              dispatch({ type: 'refine_state', refining: false })
              showToast(t('mobile.refineFailed'))
            })
          }}
          onRefineSkip={() => {
            dispatch({ type: 'refine_resolve' })
            showToast(t('mobile.refineSkipped'))
            // 通知后端广播 RefineSkipped——桌面端弹窗同步关闭（双端状态一致）
            refineSkip(token ?? '').catch(() => {})
          }}
          toast={toast}
          onToast={showToast}
        />
        {ratingMsg && (
          <RatingSheet
            message={ratingMsg}
            token={token ?? ''}
            onClose={() => setRatingMsg(null)}
            onSubmitted={() => {
              setRatingMsg(null)
              showToast(t('mobile.ratingSaved'))
            }}
          />
        )}
      </MobileErrorBoundary>
    </>
  )
}
