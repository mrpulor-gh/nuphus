/**
 * 移动端 REST API：/history 与 /message
 * 鉴权统一走 X-Mobile-Token Header（WS 才用 query）。
 */
import type { TraceItem } from './store'
import { t } from './i18n'
import { resolveTunnelDeviceId } from './connection'

/**
 * 当前 REST 通道地址（null=当前页面 origin；非 null=桌面局域网直连地址）。
 * 中继页面探测到局域网可达时由 App 调用 setApiBase 切换——页面不刷新，后续请求直接走局域网。
 */
let apiBase: string | null = null

/** 切换 REST 通道：传入桌面局域网地址（http://ip:port）或 null 恢复当前 origin */
export function setApiBase(base: string | null): void {
  apiBase = base
}

/** 读当前 REST 通道地址（WS 创建等需要同步读取的场合） */
export function getApiBase(): string | null {
  return apiBase
}

/** 解析 API 路径：无通道时原样相对路径（当前 origin）；有通道时拼到局域网地址（跨域由 mobile_server CORS 放行） */
function resolveApi(path: string): string {
  if (apiBase) {
    return `${apiBase.replace(/\/+$/, '')}/${path.replace(/^\.?\//, '')}`
  }
  // 中继通道（apiBase=null）：**显式绝对根路径**，不依赖当前页面路径。
  // ⚠️ 2026-08-26 实测：PWA 主屏幕入口打开的是 /d/<device_id>/mobile.html
  // （manifest start_url 相对解析），相对 './pair' 会错位成 /d/<device_id>/pair →
  // 桌面把它当静态资产伺服（404）→ 「网络错误请重试」。根绝对路径命中桌面 API
  // 路由；归属由 X-Tunnel-Device 头（tunnelDeviceHeaders → resolveTunnelDeviceId
  // 已支持从 /d/<id>/ 路径提取）保证中继精确路由。
  return `/${path.replace(/^\.?\//, '')}`
}

/** 与后端 HistoryMessage 对齐（commands/process/session.rs chat_history 返回） */
export interface HistoryMessage {
  role: string // "user" | "assistant" | "system" | "tool"
  content: string
  images?: string[]
  audio?: string[]
  /** 消息创建时间（Unix 毫秒）；旧数据可能缺失 */
  timestamp?: number
  /** 执行过程（思考/流式文本/工具调用，按实际顺序）——后端 Session 存储，历史拉取时下发 */
  traceItems?: TraceItem[]
}

/**
 * 带超时的 fetch：浏览器 fetch 默认无超时，隧道写半死/后端无响应时会永久挂起。
 * 实测 P0：中继入口 boot 阶段 fetchRelayHint 经隧道到桌面，隧道 Data 帧间歇停滞时
 * fetch 永不 resolve → connMode 永远 null → 手机永久白屏。
 * 超时抛 AbortError，调用方 catch 按失败处理（回退缓存/降级路径）。
 * 时长权衡：中继（国际链路）慢时正常往返可达 3-10s，8s 曾误杀 → 默认 20s；
 * 白屏防护由 load-guard（8s 提示）兜底，不依赖本超时。
 */
const DEFAULT_FETCH_TIMEOUT_MS = 20000

/**
 * 多租户隧道归属标记：中继（公网入口）按 ?device= 或本头路由到归属桌面。
 * apiBase 是裸 origin 前缀拼接（query 进 base 会打废后续路径，实测事故），
 * query 无法贯穿页面内全部 API/WS——头是唯一能统一注入的显式归属通道。
 * ⚠️ 仅中继通道（apiBase=null，相对 origin）注入：局域网直连不经中继路由，
 * 无需标记；且该头会让跨域 LAN 请求触发 CORS 预检——老桌面白名单没有它时
 * 会被整批拦下（实测：同 WiFi 自动切直连后历史加载全挂）。
 */
function tunnelDeviceHeaders(): Record<string, string> {
  if (apiBase !== null) return {}
  const id = resolveTunnelDeviceId()
  return id ? { 'X-Tunnel-Device': id } : {}
}

async function fetchWithTimeout(
  input: RequestInfo | URL,
  init?: RequestInit,
  timeoutMs: number = DEFAULT_FETCH_TIMEOUT_MS,
): Promise<Response> {
  const c = new AbortController()
  const t = setTimeout(() => c.abort(), timeoutMs)
  try {
    return await fetch(input, {
      ...init,
      headers: {
        ...tunnelDeviceHeaders(),
        ...(init?.headers as Record<string, string> | undefined),
      },
      signal: c.signal,
    })
  } finally {
    clearTimeout(t)
  }
}

/** GET /identity 返回：手机端当前生效的身份显示名（桌面端 soul 配置经后端下发） */
export interface Identity {
  assistantName: string
  userLabel: string
}

export class AuthError extends Error {
  constructor() {
    super('token rejected (401)')
    this.name = 'AuthError'
  }
}

/** POST /pair 配对失败类别：密码错误 / 触发防破解锁定 / 桌面端未设置密码 / 网络或未知 */
export type PairErrorKind = 'wrong_password' | 'locked' | 'no_password' | 'network'

/** 配对失败（密码换取 token）——kind 供 UI 分支展示，locked 时携带剩余等待秒数 */
export class PairError extends Error {
  kind: PairErrorKind
  /** kind==='locked'：还需等待的秒数（服务端文案解析，缺省 60） */
  retryAfterSec?: number

  constructor(kind: PairErrorKind, message: string, retryAfterSec?: number) {
    super(message)
    this.name = 'PairError'
    this.kind = kind
    this.retryAfterSec = retryAfterSec
  }
}

/**
 * POST /pair：配对密码换取访问 token（本端点无 token 鉴权，防暴力破解）。
 * 200 → {token}；401 密码错误；429 锁定（解析剩余秒数）；503 桌面未设置密码；
 * 其余状态与网络异常统一转 network。密码不落盘，仅返回的 token 由调用方保存。
 */
export async function postPair(password: string): Promise<string> {
  let res: Response
  try {
    res = await fetchWithTimeout(
      resolveApi('./pair'),
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...tunnelDeviceHeaders() },
        body: JSON.stringify({ password }),
      },
      10000,
    )
  } catch {
    throw new PairError('network', t('mobile.pairNetworkError'))
  }
  if (res.status === 200) {
    const data = (await res.json().catch(() => null)) as { token?: string } | null
    if (!data?.token) throw new PairError('network', t('mobile.pairMissingToken'))
    return data.token
  }
  if (res.status === 401) throw new PairError('wrong_password', t('mobile.pairWrongPasswordShort'))
  if (res.status === 429) {
    const retryAfterSec = await extractRetrySec(res).catch(() => undefined)
    throw new PairError('locked', t('mobile.pairTooManyRetry'), retryAfterSec ?? 60)
  }
  if (res.status === 503) throw new PairError('no_password', t('mobile.pairNoPasswordShort'))
  throw new PairError('network', `${t('mobile.pairFailed')} HTTP ${res.status}`)
}

/** 从 429 响应体 {"error":"尝试过多，请 N 秒后重试"} 提取剩余秒数 */
async function extractRetrySec(res: Response): Promise<number | undefined> {
  const body = (await res.json()) as { error?: string }
  const m = body.error?.match(/(\d+)\s*秒/)
  return m ? Number(m[1]) : undefined
}

async function checkAuth(res: Response): Promise<Response> {
  if (res.status === 401) throw new AuthError()
  return res
}

/** 拉取与桌面一致的对话历史（无 agent 时后端返回空列表） */
export async function fetchHistory(token: string): Promise<HistoryMessage[]> {
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi('./history'), {
      headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
    }),
  )
  if (!res.ok) throw new Error(`history failed: ${res.status}`)
  return (await res.json()) as HistoryMessage[]
}

/** 拉取当前生效的身份显示名（assistant_name / user_label，后端 relation_cache 下发） */
export async function fetchIdentity(token: string): Promise<Identity> {
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi('./identity'), {
      headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
    }),
  )
  if (!res.ok) throw new Error(`identity failed: ${res.status}`)
  return (await res.json()) as Identity
}

/** Custom Agent 卡片（仅 id + name，手机端只读展示/切换显示用） */
export interface CustomAgentBrief {
  id: string
  name: string
}

/** GET /custom-agents 返回：全部卡片 + 当前激活卡片 */
export interface CustomAgentsInfo {
  agents: CustomAgentBrief[]
  active: CustomAgentBrief | null
}

/** 拉取 Custom Agent 列表 + 激活卡片（卡片管理在桌面端，手机端只读展示） */
export async function fetchCustomAgents(token: string): Promise<CustomAgentsInfo> {
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi('./custom-agents'), {
      headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
    }),
  )
  if (!res.ok) throw new Error(`custom-agents failed: ${res.status}`)
  return (await res.json()) as CustomAgentsInfo
}

/** GET /agent-status 返回：桌面端当前执行状态 */
export interface AgentStatus {
  running: boolean
  /** 桌面端提炼进行中（refine_active 原子锁）：重连/刷新后恢复提炼状态。
   *  可选——旧后端无此字段（视为 false） */
  refine_active?: boolean
}

/** 查询桌面端执行状态（刷新/重连后恢复 running，补齐 broadcast 不重传的间隙事件） */
export async function fetchAgentStatus(token: string): Promise<AgentStatus> {
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi('./agent-status'), {
      headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
    }),
  )
  if (!res.ok) throw new Error(`agent-status failed: ${res.status}`)
  return (await res.json()) as AgentStatus
}

/** 与桌面端 list_models 对齐的模型信息（不含密钥） */
export interface ModelInfo {
  id: string
  provider: string
  alias: string[]
  supports_streaming: boolean
  supports_vision: boolean
  supports_audio: boolean
  supports_image_generation: boolean
  reasoning_efforts: string[]
  default_effort?: string | null
  /** 上下文窗口（tokens）；undefined = 未知 */
  context_window?: number
  /** 成本（USD / 百万输入 tokens）；undefined = 未知 */
  cost_per_million_in?: number
  /** 成本（USD / 百万输出 tokens）；undefined = 未知 */
  cost_per_million_out?: number
}

/** GET /model-config 返回：桌面端模型配置（主模型 + 全部已配置模型，与 /models 页同源） */
export interface ModelConfig {
  current: string
  models: ModelInfo[]
  /** 当前模型上下文窗口（token；旧后端无此字段时前端 fallback 128000） */
  contextWindow?: number
}

/** 拉取桌面端模型配置（config.toml → ModelRegistry，与桌面端 /models 页同源）。
 *  mode 可选：传当前模式时后端按 agent_models 解析该 mode 的生效模型（current 跟随模式）。
 *  fetchWithTimeout 20s 兜底：隧道写半死时裸 fetch 永不 resolve，模型卡会卡死在
 *  「读取配置中」（实测反馈）——超时抛 AbortError 让 UI 落入可重试的错误态。 */
export async function fetchModelConfig(token: string, mode?: string): Promise<ModelConfig> {
  const qs = mode ? `?mode=${encodeURIComponent(mode)}` : ''
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi(`./model-config${qs}`), {
      headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
    }),
  )
  if (!res.ok) throw new Error(`model-config failed: ${res.status}`)
  return (await res.json()) as ModelConfig
}

export interface SwitchModelResult {
  ok: boolean
  message?: string
  error?: string
}

/** 切换当前模型（provider-driven：后端从 config.toml 读 API key，前端不传任何密钥）
 *  mode：当前选择的模式（leader/workflow/custom）——切换写入对应 agent 模型配置 */
export async function switchMobileModel(
  token: string,
  model: string,
  provider: string,
  mode?: string,
): Promise<void> {
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi('./switch-model'), {
      method: 'POST',
      headers: {
        'X-Mobile-Token': token,
        'Content-Type': 'application/json',
        ...tunnelDeviceHeaders(),
      },
      body: JSON.stringify({ model, provider, mode }),
    }),
  )
  if (!res.ok) throw new Error(`switch-model failed: ${res.status}`)
  const data = (await res.json()) as SwitchModelResult
  if (!data.ok) throw new Error(data.error || t('mobile.switchModelFailed'))
}

/** 切换运行模式（leader/workflow/custom）——走后端 set_mode，广播 ModeChanged 双端同步。 */
export async function switchMobileMode(token: string, mode: string): Promise<void> {
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi('./switch-mode'), {
      method: 'POST',
      headers: {
        'X-Mobile-Token': token,
        'Content-Type': 'application/json',
        ...tunnelDeviceHeaders(),
      },
      body: JSON.stringify({ mode }),
    }),
  )
  if (!res.ok) throw new Error(`switch-mode failed: ${res.status}`)
  const data = (await res.json()) as SwitchModelResult
  if (!data.ok) throw new Error(data.error || t('mobile.switchModeFailed'))
}

/** GET /relay-hint 返回：桌面中继配置（外网模式下手机经中继发送） */
export interface RelayChannelState {
  status: 'connected' | 'retrying' | 'fault' | 'disabled'
  since?: number
  attempts?: number
  reason?: string
}

export interface RelayCfg {
  url: string
  device_id: string
  caller_token: string
  /** 桌面局域网直连地址（http://<桌面IP>:<port>）。外网页面用于探测局域网可达性——回到同一 WiFi 自动切回直连 */
  lan_url?: string
  /** 隧道公网入口（https://r.example.com 或 http://host:18081）——局域网 origin 页面故障转移到中继的基址 */
  tunnel_url?: string
  /** 桌面↔中继双回路实时状态（任务通道 + 隧道） */
  state?: { relay: RelayChannelState; tunnel: RelayChannelState }
}

/** 拉取桌面中继配置（局域网内访问，成功后由上层存 localStorage 供外网使用） */
export async function fetchRelayHint(token: string): Promise<RelayCfg | null> {
  try {
    const res = await checkAuth(
      // 10s 超时（boot 白屏敏感，但中继慢链路可达 3-10s）：经隧道到桌面，
      // 隧道 Data 帧半死时 fetch 永久挂起 → resolveLanUrl 永不返回 → connMode 永远 null
      // → 手机永久白屏（实测 P0）。超时后回退 getCachedLanUrl（局域网缓存）继续流程。
      await fetchWithTimeout(
        resolveApi('./relay-hint'),
        { headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() } },
        10000,
      ),
    )
    if (!res.ok) return null
    const data = (await res.json()) as {
      enabled?: boolean
      url?: string
      device_id?: string
      caller_token?: string
      lan_url?: string
      tunnel_url?: string
    }
    if (!data.enabled || !data.url || !data.device_id || !data.caller_token) return null
    return {
      url: data.url,
      device_id: data.device_id,
      caller_token: data.caller_token,
      lan_url: data.lan_url,
      tunnel_url: data.tunnel_url,
    }
  } catch {
    return null
  }
}

/** /boot 聚合返回：identity + agentStatus + relayHint + sessions（会话清单镜像） */
export interface BootPayload {
  identity?: Identity
  agentStatus?: { running?: boolean }
  relayHint?: {
    enabled?: boolean
    url?: string
    device_id?: string
    caller_token?: string
    lan_url?: string
    tunnel_url?: string
  }
  sessions?: ShelfSessions
}

/** 中继模式启动聚合：一次往返拿 identity+agentStatus+relayHint。
 *  仅中继（wan）路径使用——局域网保持原分请求模式（合并不影响局域网即时性）。
 *  超时 15s：载荷小（无 history），比 relay-hint 略宽以容忍丢包重传。 */
export async function fetchBoot(token: string): Promise<BootPayload | null> {
  try {
    const res = await checkAuth(
      await fetchWithTimeout(
        resolveApi('./boot'),
        { headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() } },
        15000,
      ),
    )
    if (!res.ok) return null
    return (await res.json()) as BootPayload
  } catch {
    return null
  }
}

/** 桌面展示台会话条目（/sessions 与 /boot.sessions 同源，只读镜像投影） */
export interface ShelfSessionItem {
  id: string
  mode: string
  title: string
  message_count: number
  updated_at?: string
  is_active: boolean
}

export interface ShelfSessions {
  can_switch: boolean
  items: ShelfSessionItem[]
}

/** GET /sessions —— 会话清单镜像（失败返回 null，调用方降级隐藏入口） */
export async function fetchSessions(token: string): Promise<ShelfSessions | null> {
  try {
    const res = await checkAuth(
      await fetchWithTimeout(
        resolveApi('./sessions'),
        { headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() } },
        10000,
      ),
    )
    if (!res.ok) return null
    return (await res.json()) as ShelfSessions
  } catch {
    return null
  }
}

/** 后端稳定错误码 → 用户文案（switch_session_inner 守卫语义） */
const SWITCH_ERROR_TEXT: Record<string, string> = {
  busy: '后台任务运行中，暂不能切换',
  append_pending: '有追加指令待处理，稍后再试',
  mode_mismatch: '该会话不属于当前模式',
  not_found: '会话不存在或已归档',
}

export interface NewChatResult {
  ok: boolean
  session_id?: string
}

/** POST /new-chat —— 遥控桌面新建对话（单一路径：桌面执行权威创建，手机经
 *  SessionChanged 事件跟随显示欢迎页；本端 HTTP 成功响应兜底清一次视图）。 */
export async function startNewChat(
  token: string,
): Promise<{ ok: true; sessionId: string } | { ok: false; error: string }> {
  try {
    const res = await checkAuth(
      await fetchWithTimeout(
        resolveApi('./new-chat'),
        {
          method: 'POST',
          headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
        },
        SEND_TIMEOUT_MS,
      ),
    )
    if (!res.ok) {
      const body = (await res.json().catch(() => ({}))) as { error?: string }
      return { ok: false, error: body.error ?? `新建会话失败（${res.status}）` }
    }
    const data = (await res.json()) as NewChatResult
    return { ok: true, sessionId: data.session_id ?? '' }
  } catch {
    return { ok: false, error: '网络异常，请重试' }
  }
}

/** POST /session/switch —— 遥控切换桌面当前会话（手机视图经 SessionChanged 事件跟随刷新） */
export async function switchSession(
  token: string,
  id: string,
): Promise<{ ok: true } | { ok: false; error: string }> {
  try {
    const res = await checkAuth(
      await fetchWithTimeout(
        resolveApi('./session/switch'),
        {
          method: 'POST',
          headers: {
            'X-Mobile-Token': token,
            'Content-Type': 'application/json',
            ...tunnelDeviceHeaders(),
          },
          body: JSON.stringify({ id }),
        },
        SEND_TIMEOUT_MS,
      ),
    )
    if (res.ok) return { ok: true }
    const body = (await res.json().catch(() => ({}))) as { error?: string }
    return {
      ok: false,
      error: SWITCH_ERROR_TEXT[body.error ?? ''] ?? `切换失败（${body.error ?? res.status}）`,
    }
  } catch {
    return { ok: false, error: '网络异常，请重试' }
  }
}

export type SendResult =
  | { ok: true; appended?: boolean; message?: string; timeout?: boolean; imagesDropped?: boolean }
  | { ok: false; busy: boolean; error: string }

/**
 * 发送消息到共享入口（submit_user_message，source="mobile"）。
 * send_id 由客户端生成（rid，时间戳+自增，兼容非安全上下文）供后端去重防重发。
 * busy 时不再 409 拒绝：转为追加指令插入当前执行，返回 200 {status:"append"}。
 */
/** 发送请求超时：局域网抖动/后端无响应时 fetch 可能永久挂起，
 *  导致 Composer sending 卡死、发送按钮永久锁死（用户实测 P0）。 */
const SEND_TIMEOUT_MS = 15000

/** 发送附加项：图片（data URL，已压缩）与模式（leader/workflow）——后端 /message 原生支持 */
export interface SendOptions {
  images?: string[]
  mode?: string
}

export async function sendMessage(
  token: string,
  message: string,
  sendId: string,
  opts?: SendOptions,
): Promise<SendResult> {
  // 兼容极老 WebView（无 AbortController，如部分微信 X5）：超时保护降级为
  // 纯 timer（无法中止 fetch，但保证 sending 状态一定复位，不锁死发送按钮）
  const hasAbort = typeof AbortController !== 'undefined'
  const controller = hasAbort ? new AbortController() : null
  const timer = setTimeout(() => controller?.abort(), SEND_TIMEOUT_MS)
  let res: Response
  try {
    res = await checkAuth(
      await fetch(resolveApi('./message'), {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Mobile-Token': token,
          ...tunnelDeviceHeaders(),
        },
        body: JSON.stringify({
          message,
          send_id: sendId,
          ...(opts?.images && opts.images.length > 0 ? { images: opts.images } : {}),
          ...(opts?.mode ? { mode: opts.mode } : {}),
        }),
        ...(controller ? { signal: controller.signal } : {}),
      }),
    )
  } catch (e) {
    // 超时（AbortError）≠ 失败：后端 submit_user_message 同步等待整轮 agent
    // 执行完成才返回响应（可能几分钟），15s 超时只是「响应未到」，消息大概率已
    // 受理并执行中。返回 timeout 标记，由上层保留乐观气泡等待 WS 事件确认，
    // 绝不按失败撤销——撤销曾导致 user 消息消失 + 用户误判重试 + agent 双气泡。
    if (e instanceof Error && e.name === 'AbortError') {
      clearTimeout(timer)
      return { ok: true, timeout: true }
    }
    throw e
  } finally {
    clearTimeout(timer)
  }
  if (res.ok) {
    // 200 可能是正常提交，也可能是「追加指令」被接受（busy 时不拒绝）
    try {
      const body = (await res.json()) as {
        status?: string
        message?: string
        images_dropped?: boolean
      }
      if (body.status === 'append') {
        return {
          ok: true,
          appended: true,
          message: body.message,
          imagesDropped: body.images_dropped === true,
        }
      }
    } catch {
      /* 无 body，视为正常提交 */
    }
    return { ok: true }
  }
  let error = `发送失败（${res.status}）`
  try {
    const body = (await res.json()) as { error?: string }
    if (body.error) error = body.error
  } catch {
    /* 非 JSON 响应，保留默认错误 */
  }
  return { ok: false, busy: res.status === 409, error }
}

/**
 * 危险操作确认回执（POST /confirm）。
 * approved=true + session=true 时登记对话级授权（对齐桌面 approve_session_security）。
 */
export async function postConfirm(
  token: string,
  payload: {
    action_id: string
    approved: boolean
    session?: boolean
    tool?: string
  },
): Promise<void> {
  const res = await checkAuth(
    await fetch(resolveApi('./confirm'), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Mobile-Token': token,
        ...tunnelDeviceHeaders(),
      },
      body: JSON.stringify(payload),
    }),
  )
  if (!res.ok) throw new Error(`confirm failed: ${res.status}`)
}

/**
 * request_user_input 提交（POST /user-input）：对齐桌面 submit_user_input。
 * agent 侧 poll_response 消费后继续执行；提交成功即广播确认。
 */
export async function postUserInput(token: string, actionId: string, value: string): Promise<void> {
  const res = await checkAuth(
    await fetch(resolveApi('./user-input'), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Mobile-Token': token,
        ...tunnelDeviceHeaders(),
      },
      body: JSON.stringify({ action_id: actionId, value }),
    }),
  )
  if (!res.ok) throw new Error(`user-input failed: ${res.status}`)
}

/**
 * request_user_input 取消（POST /user-input-reject）：对齐桌面 reject_user_input。
 * 后端写入 __CANCELLED__，agent 立即醒来继续（不阻塞等待超时）。
 */
export async function postUserInputReject(token: string, actionId: string): Promise<void> {
  const res = await checkAuth(
    await fetch(resolveApi('./user-input-reject'), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Mobile-Token': token,
        ...tunnelDeviceHeaders(),
      },
      body: JSON.stringify({ action_id: actionId, value: '' }),
    }),
  )
  if (!res.ok) throw new Error(`user-input-reject failed: ${res.status}`)
}

/**
 * 执行点评提交（POST /rating）：对齐桌面 submit_execution_rating（记忆系统评分）。
 */
export async function postRating(
  token: string,
  payload: {
    goal: string
    rating: number
    comment: string
    toolsSummary: string
    stepsJson: string
    sessionId: string
  },
): Promise<void> {
  const res = await checkAuth(
    await fetch(resolveApi('./rating'), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Mobile-Token': token,
        ...tunnelDeviceHeaders(),
      },
      body: JSON.stringify({
        goal: payload.goal,
        rating: payload.rating,
        comment: payload.comment,
        tools_summary: payload.toolsSummary,
        steps_json: payload.stepsJson,
        session_id: payload.sessionId,
      }),
    }),
  )
  if (!res.ok) throw new Error(`rating failed: ${res.status}`)
}

// ── 执行控制（暂停 / 继续 / 终止 / 优雅停止）─────────────────────────

export interface ControlResult {
  status: 'paused' | 'resumed' | 'terminated' | 'stopping'
  action_id?: string
}

/**
 * 执行控制上行统一入口：pause/stop 无请求体；resume/terminate 回传
 * 后端 /pause 广播的 action_id。鉴权统一走 X-Mobile-Token Header。
 */
async function postControl(token: string, path: string, body?: unknown): Promise<ControlResult> {
  const headers: Record<string, string> = {
    'X-Mobile-Token': token,
    ...tunnelDeviceHeaders(),
  }
  const init: RequestInit = { method: 'POST', headers }
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json'
    init.body = JSON.stringify(body)
  }
  const res = await checkAuth(await fetch(resolveApi(path), init))
  if (!res.ok) throw new Error(`${path} failed: ${res.status}`)
  return (await res.json()) as ControlResult
}

/** 暂停执行（后端生成 action_id，事件经 WS 双推回来） */
export function pauseExecution(token: string): Promise<ControlResult> {
  return postControl(token, './pause')
}

/** 继续执行（回传暂停时的 action_id） */
export function resumeExecution(token: string, actionId: string): Promise<ControlResult> {
  return postControl(token, './resume', { action_id: actionId })
}

/** 终止执行（回传暂停时的 action_id） */
export function terminateExecution(token: string, actionId: string): Promise<ControlResult> {
  return postControl(token, './terminate', { action_id: actionId })
}

/** 优雅停止（不等暂停菜单，直接 Terminate） */
export function stopExecution(token: string): Promise<ControlResult> {
  return postControl(token, './stop')
}

/**
 * 工作流遥控（POST /workflow-pause | /workflow-resume | /workflow-stop）：
 * 复用 WorkflowEngine 控制命令（pause_workflow / resume_workflow / cancel_workflow），
 * 请求体 {workflow_id}，鉴权走 X-Mobile-Token（与现有控制端点一致）。
 */
async function postWorkflowControl(token: string, path: string, workflowId: string): Promise<void> {
  const res = await checkAuth(
    await fetchWithTimeout(resolveApi(path), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Mobile-Token': token,
        ...tunnelDeviceHeaders(),
      },
      body: JSON.stringify({ workflow_id: workflowId }),
    }),
  )
  if (!res.ok) {
    let error = `${path} failed: ${res.status}`
    try {
      const body = (await res.json()) as { error?: string }
      if (body.error) error = body.error
    } catch {
      /* 非 JSON 响应，保留默认错误 */
    }
    throw new Error(error)
  }
}

/** 暂停工作流执行（等价桌面 wf_pause） */
export function wfPause(token: string, workflowId: string): Promise<void> {
  return postWorkflowControl(token, './workflow-pause', workflowId)
}

/** 继续工作流执行（等价桌面 wf_resume） */
export function wfResume(token: string, workflowId: string): Promise<void> {
  return postWorkflowControl(token, './workflow-resume', workflowId)
}

/** 终止工作流执行（cancel_workflow + mark_user_cancelled，等价桌面 wf_stop） */
export function wfStop(token: string, workflowId: string): Promise<void> {
  return postWorkflowControl(token, './workflow-stop', workflowId)
}
/** 会话提炼（refine）：手机端触发（对齐桌面 execute_session_refine）。
 *  显式 100s 超时 > 后端 Leader 90s 硬超时——默认 20s 会在提炼正常进行中误杀请求。 */
export async function triggerRefine(token: string): Promise<void> {
  const res = await checkAuth(
    await fetchWithTimeout(
      resolveApi('./refine'),
      {
        method: 'POST',
        headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
      },
      100_000,
    ),
  )
  if (!res.ok) {
    let err = `refine failed: ${res.status}`
    try {
      const body = (await res.json()) as { error?: string }
      if (body.error) err = body.error
    } catch {
      /* 非 JSON 响应，保留默认错误 */
    }
    throw new Error(err)
  }
}

/** 跳过提炼：通知后端广播 RefineSkipped，桌面端弹窗同步关闭（双端状态一致） */
export async function refineSkip(token: string): Promise<void> {
  const res = await checkAuth(
    await fetch(resolveApi('./refine-skip'), {
      method: 'POST',
      headers: { 'X-Mobile-Token': token, ...tunnelDeviceHeaders() },
    }),
  )
  if (!res.ok) throw new Error(`refine-skip failed: ${res.status}`)
}