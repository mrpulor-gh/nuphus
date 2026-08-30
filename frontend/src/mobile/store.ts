/**
 * 移动端对话状态机：历史种子 + NuphusEvent 实时应用
 *
 * 设计要点：
 * - 发送走乐观回显（LAN 低延迟但 POST 要等整轮执行完成，不能靠响应驱动 UI）；
 *   user_message_received 广播回来后按内容匹配确认（事件无 send_id，LAN 同用户启发式足够）
 * - 桌面发出的消息经同一广播到达本端（source="desktop"）——消息互见
 * - from_task / from_subtask 事件是子任务内部过程，一律不渲染
 * - 未知事件类型 default 分支原样返回——静默忽略（向前兼容）
 */

import type { NuphusEvent, WorkflowRunStep } from '../core/types'
import { t } from './i18n'

/** 执行过程条目：思考 / agent 流式文本 / 工具调用，按实际发生顺序排列 */
export type TraceItem =
  | { kind: 'thinking'; text: string }
  | { kind: 'text'; text: string }
  | {
      kind: 'tool'
      callId: string
      name: string
      status: 'running' | 'ok' | 'fail'
      durationMs?: number
      /** 工具调用参数（显示执行的路径/命令等关键信息） */
      params?: unknown
    }

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant' | 'system' | 'refine'
  content: string
  /** 图片 data URL 列表（桌面端发的图在手机端可见） */
  images?: string[]
  /** 音频 URL 列表（agent 返回 TTS 语音时播放） */
  audio?: string[]
  /** 思考过程（is_thinking delta 累积），与正文分开渲染 */
  thinking?: string
  /** 执行过程完整记录（思考 + agent 流式文本 + 工具调用），按实际顺序排列 */
  traceItems?: TraceItem[]
  /** 消息时间戳（毫秒），历史消息可能缺失 */
  timestamp?: number
  /** 消息来源："desktop" | "mobile"（互见标记） */
  source?: string
  /** 乐观回显待广播确认 */
  pending?: boolean
  /** 流式输出中 */
  streaming?: boolean
}

export interface ToolActivity {
  callId: string
  name: string
  success?: boolean
  durationMs?: number
}

export interface ActivityState {
  running: boolean
  goal: string
  mode: string
  tools: ToolActivity[]
  /** 执行暂停中（收到 execution_paused 事件） */
  paused: boolean
  /** 当前暂停的 action_id（继续/终止时回传后端） */
  pauseActionId?: string
  /** 本轮执行开始时间（ms），用于顶部实时用时 */
  startedAt?: number
  /** 暂停发生时间（ms），用于冻结用时 */
  pausedAt?: number
}

export interface PendingConfirm {
  actionId: string
  tool: string
  params: string
  risk: string
  reason: string
}

/** 会话提炼提示（refine_prompt 事件）：上下文超阈值，询问用户是否提炼 */
export interface PendingRefine {
  currentTokens: number
  refineLimit: number
  forceLimit: number
  threshold: number
  contextWindow: number
  /** true = 后端已强制决定提炼（前端自动执行，不弹窗） */
  forced: boolean
}

/** request_user_input 请求（user_input_request 事件）：手机端可交互的输入弹窗 */
export interface PendingUserInput {
  actionId: string
  title: string
  prompt: string
  sensitive: boolean
  /** text / screenshot / region / mouse_pos / color / icon_confirm */
  inputType: string
}

export interface ChatState {
  messages: ChatMessage[]
  activity: ActivityState
  /** 危险操作确认请求（security_check 事件），等待本端或桌面端处理 */
  pendingConfirm: PendingConfirm | null
  /** 会话提炼提示（refine_prompt 事件），确认后触发提炼 */
  pendingRefine: PendingRefine | null
  /** request_user_input 输入请求（user_input_request 事件），等待本端或桌面端处理 */
  pendingUserInput: PendingUserInput | null
  /** 提炼执行中（refine_executing 事件，弹窗显示进度） */
  refining: boolean
  /** 当前生效身份显示名（GET /identity 下发，桌面端 soul 配置） */
  identity?: { assistantName: string; userLabel: string }
  /** 当前执行模型（session_info 事件下发，只读展示用） */
  model?: string
  /** 会话累计上下文用量（token_usage 事件：input/output/cache_hit，执行后广播）
   *  outputTokens / cacheHitTokens 可能缺失（后端不可用时 undefined） */
  tokenUsage?: { inputTokens: number; outputTokens?: number; cacheHitTokens?: number }
  /** 工作流执行实时状态（workflow_event 事件驱动，WorkflowRunCard 渲染 + 遥控）。
   *  undefined = 当前无工作流执行活动（run_started 后创建） */
  workflowRun?: {
    steps: WorkflowRunStep[]
    /** 最近一次运行的 workflow id（run 结束后保留，供控制端点复用） */
    lastWorkflowId?: string
    /** step_run_paused 置 true；run_completed / resume 后置 false */
    isPaused: boolean
    /** run_completed 置 true（本轮已结束，卡片显示完成态、隐藏控制按钮）；run_started 重置 false */
    done: boolean
    /** error 事件消息（执行异常提示） */
    message?: string
  }
  /** 用户已移除工作流预览（workflow_clear）：本轮后续事件不再重建，
   *  下一轮 run_started 自动重置并重新出现（胶囊可移除但执行不受影响） */
  workflowDismissed?: boolean
}

export const initialChatState: ChatState = {
  messages: [],
  activity: { running: false, goal: '', mode: 'leader', tools: [], paused: false },
  pendingConfirm: null,
  pendingRefine: null,
  pendingUserInput: null,
  refining: false,
}

export type ChatAction =
  | { type: 'history'; messages: ChatMessage[] }
  | { type: 'history_merge'; messages: ChatMessage[]; manual?: boolean }
  /** 连接成功快照：电脑端当前状态权威镜像（WS ws_connected 后由服务端推送）。
   *  messages 为空 = 电脑端欢迎界面（显示欢迎而非拉取失败）；running = 桌面执行中。 */
  | { type: 'session_snapshot'; messages: ChatMessage[]; welcome: boolean; running: boolean }
  | { type: 'new_chat' }
  | { type: 'event'; event: NuphusEvent }
  | { type: 'optimistic'; message: ChatMessage }
  | { type: 'send_failed'; id: string; reason: string }
  | { type: 'confirm_optimistic'; id: string }
  | { type: 'set_model'; model: string }
  /** 追加指令受理：移除乐观气泡（不显示为独立消息，仅弹窗提示） */
  | { type: 'remove_optimistic'; id: string }
  | { type: 'confirm_resolved' }
  /** request_user_input 已处理（提交/取消/超时）：清空输入请求 */
  | { type: 'user_input_resolved' }
  /** 继续/终止成功后本地复位暂停态（不等下一轮事件，保证 UI 即时正确） */
  | { type: 'pause_reset' }
  /** 用户响应提炼提示（确认 → 触发 /refine；跳过 → 关闭弹窗） */
  | { type: 'refine_resolve' }
  /** 提炼状态更新（开始/完成/失败） */
  | { type: 'refine_state'; refining: boolean }
  /** 身份显示名下发（GET /identity） */
  | { type: 'identity'; identity: { assistantName: string; userLabel: string } }
  /** 刷新/重连后同步执行状态（broadcast 不重传间隙事件，以此恢复 running） */
  | { type: 'sync_running'; running: boolean }
  /** 关闭工作流执行卡（WorkflowRunCard 完成态右上角 X）：清除 workflowRun，下次 run_started 重建 */
  | { type: 'workflow_clear' }

let seq = 0
/** 消息 ID 生成：移动端页面经局域网 HTTP 访问（非安全上下文），crypto.randomUUID 不可用，故用时间戳+自增 */
export function rid(): string {
  return `m-${Date.now()}-${++seq}`
}

/** 追加执行过程条目：同 kind 连续文本合并到末尾条目（流式分片累积），工具调用各自成条 */
function appendTraceItem(
  items: TraceItem[] | undefined,
  kind: 'thinking' | 'text',
  text: string,
): TraceItem[] {
  const list = items ? items.slice() : []
  const last = list[list.length - 1]
  if (last && last.kind === kind) {
    list[list.length - 1] = { ...last, text: last.text + text }
  } else {
    list.push({ kind, text })
  }
  return list
}

/** 结束所有流式气泡（轮次终态统一收口） */
function finalizeStreaming(messages: ChatMessage[]): ChatMessage[] {
  let changed = false
  const next = messages.map(m => {
    if (m.streaming) {
      changed = true
      return { ...m, streaming: false }
    }
    return m
  })
  return changed ? next : messages
}

function applyEvent(state: ChatState, ev: NuphusEvent): ChatState {
  switch (ev.type) {
    case 'user_message_received': {
      // 乐观回显确认：本端刚发出的消息经广播回来，内容匹配即确认
      const idx = state.messages.findIndex(
        m => m.pending && m.role === 'user' && m.content === ev.content,
      )
      if (idx >= 0) {
        const messages = state.messages.slice()
        messages[idx] = {
          ...messages[idx],
          pending: false,
          source: ev.source,
          images: ev.images && ev.images.length > 0 ? ev.images : messages[idx].images,
        }
        return { ...state, messages }
      }
      // 系统内容/追加段不显示：refine 提示词（旧后端广播兜底）、上下文用量系统提示
      // （[系统提示词] 前缀）与追加指令段只进 LLM 上下文，前端不生成用户气泡
      // （对齐后端 extract_history 过滤规则）。
      const content = ev.content ?? ''
      if (
        content.startsWith('开始进行上下文提炼') ||
        content.startsWith('[系统提示词]') ||
        content.startsWith('[APPEND]')
      ) {
        return state
      }
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: rid(),
            role: 'user',
            content: ev.content,
            images: ev.images && ev.images.length > 0 ? ev.images : undefined,
            source: ev.source,
            timestamp: Date.now(),
          },
        ],
      }
    }

    case 'execution_started':
      return {
        ...state,
        activity: {
          running: true,
          goal: ev.goal,
          mode: ev.mode ?? 'leader',
          tools: [],
          paused: false,
          pauseActionId: undefined,
          startedAt: Date.now(),
          pausedAt: undefined,
        },
      }

    case 'mode_changed':
      // 空闲态切 mode（后端 set_mode 广播，含手机端 /switch-mode）→ 同步「当前模式」。
      // 只更新 mode，不重置 running/goal/tools（与 execution_started 区分）。
      return {
        ...state,
        activity: {
          ...state.activity,
          mode: ev.mode ?? 'leader',
        },
      }

    case 'execution_paused':
      // 暂停：记录 action_id，控制区切换为 继续/终止；冻结用时
      return {
        ...state,
        activity: {
          ...state.activity,
          paused: true,
          pauseActionId: ev.action_id,
          pausedAt: Date.now(),
        },
      }

    case 'llm_text_delta': {
      // 非执行中忽略：息屏/切应用返回后积压的旧轮次 delta 重放时，
      // running 已为 false（execution_completed 先到）→ 忽略，防创建孤立空气泡。
      if (!state.activity.running) return state
      // 不拦截 from_task：子任务（ExecAgent）的思考/文本也是执行过程的一部分，
      // 手机端「执行过程」应展示完整链路（与桌面端 timeline 一致）。
      const messages = state.messages.slice()
      const last = messages[messages.length - 1]
      // 执行过程统一进 traceItems（思考 + agent 流式文本，按实际顺序）；正文 delta
      // 同时实时累积到 content（流式显示，对齐桌面端），execution_completed 时以
      // result_message 覆盖定稿——流式过程可见、最终回复干净（中间过程文本仅流式可见）。
      const kind: 'thinking' | 'text' = ev.is_thinking ? 'thinking' : 'text'
      if (last && last.role === 'assistant' && last.streaming) {
        messages[messages.length - 1] = {
          ...last,
          content: kind === 'text' ? (last.content ?? '') + ev.text : last.content,
          thinking: kind === 'thinking' ? (last.thinking ?? '') + ev.text : last.thinking,
          traceItems: appendTraceItem(last.traceItems, kind, ev.text),
        }
      } else {
        messages.push({
          id: rid(),
          role: 'assistant',
          content: kind === 'text' ? ev.text : '',
          thinking: kind === 'thinking' ? ev.text : undefined,
          traceItems: [{ kind, text: ev.text }],
          streaming: true,
          timestamp: Date.now(),
        })
      }
      return { ...state, messages }
    }

    case 'tool_call_start': {
      if (!state.activity.running) return state
      // 不拦截 from_task：子任务（ExecAgent）的工具调用是执行过程核心——
      // Leader 大部分工作经 dispatch 派发，丢弃后手机端执行过程看不到任何工具。
      const messages = state.messages.slice()
      const last = messages[messages.length - 1]
      // 工具调用按实际顺序追加进 traceItems（running 态，等 end 补状态）。
      const item = {
        kind: 'tool' as const,
        callId: ev.call_id,
        name: ev.tool_name,
        status: 'running' as const,
        params: ev.params,
      }
      if (last && last.role === 'assistant' && last.streaming) {
        const traceItems = last.traceItems ? last.traceItems.slice() : []
        traceItems.push(item)
        messages[messages.length - 1] = { ...last, traceItems }
      } else {
        // 执行中打开/返回（重连 sync_running 恢复 running=true）：streaming 气泡
        // 要等下一条 llm_text_delta 才创建，若 agent 正处于纯工具阶段（dispatch
        // 派发后长工具链，无文本流），此前每个 tool 事件都被静默丢弃——用户
        // 全程看不到执行过程（0ac9979 空气泡守卫造成的回归）。
        // 此处主动创建 trace 气泡；旧轮次重放不会误入——上方 running 守卫已拦截
        // （completed 先到 → running=false），completed 丢失则由 sync_running 收口。
        messages.push({
          id: rid(),
          role: 'assistant',
          content: '',
          traceItems: [item],
          streaming: true,
          timestamp: Date.now(),
        })
      }
      return {
        ...state,
        messages,
        activity: {
          ...state.activity,
          // agent 已恢复执行（越过暂停检查点）→ 复位暂停态，防 UI 卡在暂停菜单
          paused: false,
          pauseActionId: undefined,
          pausedAt: undefined,
          tools: [...state.activity.tools, { callId: ev.call_id, name: ev.tool_name }],
        },
      }
    }

    case 'tool_call_end': {
      if (!state.activity.running) return state
      // 不拦截 from_task：子任务工具调用结果同样进 traceItems（✓/✗ + 耗时）
      const messages = state.messages.slice()
      const last = messages[messages.length - 1]
      // 工具结果标记进 traceItems（✓/✗ + 耗时），按 callId 定位。
      // 同样仅限流式中消息（防旧事件重放污染已完成消息）。
      if (last && last.role === 'assistant' && last.streaming && last.traceItems) {
        const traceItems = last.traceItems.map(t =>
          t.kind === 'tool' && t.callId === ev.call_id
            ? {
                ...t,
                status: ev.success ? ('ok' as const) : ('fail' as const),
                durationMs: ev.duration_ms,
              }
            : t,
        )
        messages[messages.length - 1] = { ...last, traceItems }
      }
      return {
        ...state,
        messages,
        activity: {
          ...state.activity,
          tools: state.activity.tools.map(t =>
            t.callId === ev.call_id ? { ...t, success: ev.success, durationMs: ev.duration_ms } : t,
          ),
        },
      }
    }

    case 'image_generated': {
      // agent 产图（image_generate 工具完成）：追加一条带图 assistant 消息。
      // 无文本时图片独立展示（MessageBubble mediaBlock）；执行中事件才接收。
      if (!state.activity.running) return state
      return {
        ...state,
        messages: [
          ...state.messages,
          {
            id: rid(),
            role: 'assistant',
            content: '',
            images: [ev.url],
            timestamp: Date.now(),
          },
        ],
      }
    }

    case 'execution_completed': {
      const messages = finalizeStreaming(state.messages)
      const result = ev.output?.result_message?.trim()
      // Refine 模式：result_message 是后端 resume 内部生成的提炼摘要，
      // 由 session_refined 消费（本端只清状态不生成气泡）。若覆盖最后一条
      // assistant（真实回复）会把最终回复替换成 refine 内容，故跳过。
      // 最终回复填入最后一条 assistant 消息 content（执行过程 trace 保留，可点击查看）
      if (!state.refining && result) {
        const lastIdx = messages.length - 1
        const last = messages[lastIdx]
        if (last && last.role === 'assistant') {
          messages[lastIdx] = { ...last, content: result, streaming: false }
        } else {
          messages.push({
            id: rid(),
            role: 'assistant',
            content: result,
            timestamp: Date.now(),
          })
        }
      }
      return {
        ...state,
        activity: {
          ...state.activity,
          running: false,
          paused: false,
          pauseActionId: undefined,
          startedAt: undefined,
          pausedAt: undefined,
        },
        pendingConfirm: null,
        messages,
      }
    }

    case 'direct_response': {
      // 提炼等系统反馈（"上下文已提炼"/"提炼失败"）不生成气泡——系统内容
      // 不显示在前端，只清执行态；用户可见的直接响应（如停止反馈）保留气泡。
      const dm = ev.message ?? ''
      const cleared = {
        ...state,
        activity: {
          ...state.activity,
          paused: false,
          pauseActionId: undefined,
          pausedAt: undefined,
          startedAt: undefined,
        },
      }
      if (dm.startsWith('上下文已提炼') || dm.startsWith('提炼失败')) {
        return cleared
      }
      return {
        ...cleared,
        messages: [
          ...finalizeStreaming(state.messages),
          { id: rid(), role: 'assistant', content: ev.message, timestamp: Date.now() },
        ],
      }
    }

    case 'execution_error':
      return {
        ...state,
        activity: {
          ...state.activity,
          running: false,
          paused: false,
          pauseActionId: undefined,
          startedAt: undefined,
          pausedAt: undefined,
        },
        pendingConfirm: null,
        messages: [
          ...finalizeStreaming(state.messages),
          { id: rid(), role: 'system', content: `执行出错：${ev.error}` },
        ],
      }

    case 'error': {
      if (ev.from_subtask) return state
      return {
        ...state,
        activity: {
          ...state.activity,
          running: false,
          paused: false,
          pauseActionId: undefined,
          startedAt: undefined,
          pausedAt: undefined,
        },
        pendingConfirm: null,
        messages: [
          ...finalizeStreaming(state.messages),
          { id: rid(), role: 'system', content: ev.message },
        ],
      }
    }

    case 'warning':
      return {
        ...state,
        messages: [...state.messages, { id: rid(), role: 'system', content: ev.message }],
      }

    case 'security_check':
      // 危险操作确认请求：弹出确认卡（本端与桌面弹窗并存，先响应者生效）
      return {
        ...state,
        pendingConfirm: {
          actionId: ev.action_id,
          tool: ev.tool,
          params: ev.params,
          risk: ev.risk,
          reason: ev.reason,
        },
      }

    case 'refine_prompt':
      // 上下文超阈值：forced=true 自动执行（不弹窗，直接触发 /refine）；
      // 否则弹窗询问用户（本端与桌面弹窗并存，先响应者生效）。
      if (ev.forced) {
        return {
          ...state,
          pendingRefine: null,
          refining: true,
        }
      }
      return {
        ...state,
        pendingRefine: {
          currentTokens: ev.current_tokens ?? 0,
          refineLimit: ev.refine_limit ?? 0,
          forceLimit: ev.force_limit ?? 0,
          threshold: ev.threshold ?? 0,
          contextWindow: ev.context_window ?? 0,
          forced: false,
        },
      }

    case 'refine_skipped':
      // 另一端（桌面/手机）跳过了提炼：本端同步关闭弹窗（双端状态一致）
      return { ...state, pendingRefine: null }

    case 'refine_executing':
      return { ...state, refining: true, pendingRefine: null }

    case 'session_refined':
      // 提炼完成：只清状态（弹窗自然关闭）。提炼结果/提示属系统内容，
      // 不生成消息气泡（对齐后端 extract_history 过滤，前端不显示系统提示）。
      return {
        ...state,
        refining: false,
        pendingRefine: null,
      }

    case 'refine_failed':
      // 提炼失败（LLM key 失效/连不上/超时/空摘要）：与 refine_executing 配对的
      // 结束事件——退出「正在提炼」卡片并明示原因（message 已含完整描述），
      // 缺失此分支时卡片永久显示（假死）。
      return {
        ...state,
        refining: false,
        pendingRefine: null,
        messages: [
          ...finalizeStreaming(state.messages),
          { id: rid(), role: 'system', content: ev.message },
        ],
      }

    case 'user_input_request':
      // request_user_input：手机端渲染输入弹窗（text 类可直接交互；
      // 截图/坐标/取色/图标确认等依赖桌面能力的类型由弹窗内提示去桌面完成）
      return {
        ...state,
        pendingUserInput: {
          actionId: ev.action_id,
          title: ev.title,
          prompt: ev.prompt,
          sensitive: ev.sensitive,
          inputType: ev.input_type || 'text',
        },
      }

    case 'leader_done':
      return {
        ...state,
        activity: {
          ...state.activity,
          running: false,
          paused: false,
          pauseActionId: undefined,
          startedAt: undefined,
          pausedAt: undefined,
        },
        pendingConfirm: null,
      }

    case 'session_info':
      // 当前执行模型（桌面端下发），手机端「模型设置」只读展示
      if (!ev.model) return state
      return { ...state, model: ev.model }

    case 'token_usage': {
      // 会话累计上下文用量（input/output/cache_hit），驱动模型卡「上下文 xx%」与执行弹窗统计
      if (typeof ev.input_tokens !== 'number' || ev.input_tokens < 0) return state
      // cache_hit_tokens 哨兵 0xffffffff = 后端无缓存数据（执行完成的 session 汇总事件，
      // process.rs post-processing 发射）。**沿用上次命中值**（对齐桌面端 useEvents）——
      // 清零会让执行弹窗在「新指令思考中」误显 0%，直到下一个真实命中事件到达。
      const cacheHit =
        typeof ev.cache_hit_tokens === 'number' && ev.cache_hit_tokens !== 0xffffffff
          ? ev.cache_hit_tokens
          : (state.tokenUsage?.cacheHitTokens ?? 0)
      const output =
        typeof ev.output_tokens === 'number' && ev.output_tokens >= 0 ? ev.output_tokens : undefined
      return {
        ...state,
        tokenUsage: {
          inputTokens: ev.input_tokens,
          outputTokens: output,
          cacheHitTokens: cacheHit,
        },
      }
    }

    case 'workflow_event': {
      // 工作流执行实时状态：解析逻辑对齐桌面 useExecutionUI.ts:139-219（同一 payload 结构）
      // 用户已移除预览（workflow_clear）：本轮后续事件静默忽略（不重建卡片），
      // 仅 run_started（新一轮）重置 dismissed 并重新出现——移除不影响后端执行。
      if (state.workflowDismissed && ev.event !== 'run_started') return state
      const str = (v: unknown, d = '') => (typeof v === 'string' ? v : d)
      const cur = state.workflowRun ?? {
        steps: [] as WorkflowRunStep[],
        isPaused: false,
        done: false,
      }
      switch (ev.event) {
        case 'run_started': {
          // 新一轮执行：清空步骤列表，记录 workflow_id（供控制端点复用），重置完成态
          return {
            ...state,
            workflowDismissed: false,
            workflowRun: {
              steps: [],
              lastWorkflowId: str(ev.workflow_id) || undefined,
              isPaused: false,
              done: false,
            },
          }
        }
        case 'step_run_started': {
          const id = str(ev.step_id)
          const steps = cur.steps.some(s => s.id === id)
            ? cur.steps.map(s => (s.id === id ? { ...s, status: 'running' as const } : s))
            : [
                ...cur.steps,
                {
                  id,
                  name: str(ev.step_name, '未知步骤'),
                  status: 'running' as const,
                  depth: typeof ev.depth === 'number' ? ev.depth : 0,
                  kind: str(ev.kind, 'tool'),
                },
              ]
          return { ...state, workflowRun: { ...cur, steps, isPaused: false } }
        }
        case 'step_run_completed': {
          const id = str(ev.step_id)
          // 失败判定：StepRunStatus 是外部标记枚举，Error(String) 序列化为 {"Error": "..."}。
          // 旧实现只比对字符串 'Failed'（枚举中不存在该变体）→ 永假死代码，失败步骤被
          // 误收敛为绿色 completed。现识别对象形状（兼容旧字符串 'Failed' 防御）。
          const raw = ev.status
          const failed =
            (typeof raw === 'string' && raw === 'Failed') ||
            (typeof raw === 'object' && raw !== null && 'Error' in raw)
          const steps = cur.steps.map(s =>
            s.id === id
              ? { ...s, status: (failed ? 'failed' : 'completed') as 'failed' | 'completed' }
              : s,
          )
          return { ...state, workflowRun: { ...cur, steps } }
        }
        case 'step_run_paused': {
          const id = str(ev.step_id)
          const steps = cur.steps.map(s => (s.id === id ? { ...s, status: 'paused' as const } : s))
          return { ...state, workflowRun: { ...cur, steps, isPaused: true } }
        }
        case 'run_completed': {
          // 运行结束：running/pending 步骤统一收敛为 completed，解除暂停态，标记完成（卡片隐藏控制按钮）
          const steps = cur.steps.map(s =>
            s.status === 'running' || s.status === 'pending'
              ? { ...s, status: 'completed' as const }
              : s,
          )
          return { ...state, workflowRun: { ...cur, steps, isPaused: false, done: true } }
        }
        case 'error': {
          return { ...state, workflowRun: { ...cur, isPaused: false, message: str(ev.message) } }
        }
        default:
          // 其它 WorkflowEvent（step_run_output / step_run_retry / status_change 等）不驱动卡片
          return state
      }
    }

    default:
      // 未知/暂不支持的事件（token_usage、refine 等）：
      // 静默忽略，向前兼容。
      return state
  }
}

export function chatReducer(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case 'history':
      // 重拉历史（首载 / 断线重连补齐）：直接替换本地视图
      return { ...state, messages: action.messages }
    case 'new_chat':
      // 新会话：清空前端消息与待处理卡片（历史仍在后端，刷新可恢复）
      return {
        ...state,
        messages: [],
        pendingConfirm: null,
        pendingRefine: null,
        pendingUserInput: null,
        workflowRun: undefined,
      }
    case 'history_merge': {
      // 执行中重连补历史：合并而非替换——保留本地仍在流式中的消息，
      // 历史作为已完成部分补在前面。
      // live = pending/streaming（无条件）+ 执行中后端历史缺失的本地消息。
      // 执行中 leader_agent 被 take 移出 runtime，chat_history 只能读 session_backup
      // （执行前快照）→ 后端历史缺当前轮 user/agent；若本地完成态消息被历史替换
      // 则最后气泡消失（用户实测：息屏重开/切换应用返回后最后消息不见）。
      // 仅执行中启用保护：空闲/完成态历史完整，不保护避免 refine 后旧消息残留。
      const historyKeys = new Set(action.messages.map(h => `${h.role}|${h.content ?? ''}`))
      // 长尾对账（P2 修复）：空闲态下 pending 超 60s 且历史无同 content
      // = 请求从未到达后端（隧道半死/断网瞬间 POST 挂起 15s 超时）→
      // 转失败提示，不再永久挂灰。执行中/历史已含的 pending 不动
      // （超时≠失败：消息大概率已受理，撤销曾导致双气泡——防双发窗口保留）。
      // ⚠️ manual（用户主动点「重新拉取」）跳过对账：手动重拉是用户明确要求
      // 恢复历史的操作，此时 WS 断开会导致 running 失真（false），叠加后端
      // 执行中返回 session_backup 旧快照（缺当前轮）→ 本地 pending 消息会被
      // 误判「未送达」而删除（实测：退出重进才正常，重新拉取反而丢消息）。
      const now = Date.now()
      const stalePending: typeof state.messages = []
      const live = state.messages.filter(m => {
        if (m.pending && m.role === 'user') {
          const stale =
            !action.manual &&
            !state.activity.running &&
            typeof m.timestamp === 'number' &&
            now - m.timestamp > 60_000 &&
            !historyKeys.has(`${m.role}|${m.content ?? ''}`)
          if (stale) {
            stalePending.push(m)
            return false
          }
          return true
        }
        return (
          m.pending ||
          m.streaming ||
          (state.activity.running && !historyKeys.has(`${m.role}|${m.content ?? ''}`))
        )
      })
      const merged = [...action.messages, ...live]
      // 去重：role+content 相同只保留一份。优先级——
      // - assistant 流式中/pending 消息覆盖历史副本（历史同 content 是完成态
      //   副本，保留会丢 streaming 标记 → 后续 delta 新建气泡 → 消息分裂）
      // - user pending 若历史已含同 content = 后端已受理（广播可能丢失，
      //   迟到订阅靠 history_merge 兜底）→ 保留历史版本确认态（清除 pending）
      // - 历史内部重复（重试产生同 content user）保留最先那份即可
      const seen = new Set<string>()
      const out: typeof state.messages = []
      for (const m of merged) {
        const key = `${m.role}|${m.content ?? ''}`
        const idx = out.findIndex(o => `${o.role}|${o.content ?? ''}` === key)
        if (idx < 0) {
          seen.add(key)
          out.push(m)
        } else if (m.role === 'assistant' && (m.streaming || m.pending)) {
          // assistant 流式中覆盖历史副本（保留流式状态与 trace）
          out[idx] = m
        } else if (m.role === 'user' && m.pending) {
          // user pending + 历史已有同 content → 确认（保留历史无 pending 版本）
          // 无需替换；历史版本已在 out 中
        } else {
          // 已存在且非 live → 保留既有（历史优先，避免重复）
        }
      }
      if (stalePending.length > 0) {
        out.push({
          id: rid(),
          role: 'system',
          content: t('mobile.msgNotDelivered'),
          timestamp: now,
        })
      }
      return {
        ...state,
        messages: out
          // 历史（fetchHistory）现已返回真实执行过程 traceItems（后端 Session 存储），
          // 刷新后保留——显示完成状态（执行栏「执行完成 · N 个工具」可点开看详情）。
          // 仅移除「空气泡」：无正文、无执行过程、非执行中的 assistant 残留消息
          // （息屏/切应用期间 WS 事件重放可能创建的孤立流式气泡）。
          .filter(
            m =>
              !(
                m.role === 'assistant' &&
                !m.streaming &&
                !m.pending &&
                !m.content?.trim() &&
                !m.traceItems?.length
              ),
          ),
      }
    }
    case 'session_snapshot': {
      // 连接成功快照（轻量状态帧，welcome/running）——**不再清空本地消息**。
      // 背景（2026-08-26 不同步根治）：快照不带历史（messages=[]），若 reducer 把
      // 现有消息替换为空 → 手机外网 WS 断线重连（频繁）每次新快照都清空已显示历史，
      // 依赖后续 loadHistory 恢复 → 拉取失败/慢 = 不同步。改为：快照 messages 为空时
      // **保留本地全部消息**；历史一致性由 onReady 每次连接的 loadHistory 全量拉取保证
      // （v0.1.5 逻辑）；带历史种子（理论上）时才按 live 保护合并。
      const live =
        action.messages.length > 0
          ? state.messages.filter(
              m =>
                m.pending ||
                m.streaming ||
                (action.running && !action.messages.some(h => h.id === m.id)),
            )
          : state.messages
      const merged = [...action.messages, ...live]
      const seen = new Set<string>()
      const out = merged.filter(m => {
        const k = `${m.role}|${m.content ?? ''}`
        if (seen.has(k)) return false
        seen.add(k)
        return true
      })
      return {
        ...state,
        messages: out,
        activity: {
          ...state.activity,
          running: action.running,
          goal: action.running ? state.activity.goal : '',
        },
      }
    }
    case 'event':
      return applyEvent(state, action.event)
    case 'sync_running': {
      // 刷新/重连后同步执行状态（broadcast 不为迟到订阅者补发，间隙事件会丢失）。
      if (action.running && !state.activity.running) {
        // 后端仍在执行 → 恢复 running，后续 delta 正常累积 streaming 气泡
        return {
          ...state,
          activity: {
            ...state.activity,
            running: true,
            startedAt: state.activity.startedAt ?? Date.now(),
          },
        }
      }
      if (!action.running && state.activity.running) {
        // 后端已完成（间隙错过 execution_completed）→ finalize 收口 + 复位；
        // 最终结果由本次 onReady 的 loadHistory 拉取落地
        return {
          ...state,
          activity: { ...state.activity, running: false, startedAt: undefined },
          messages: finalizeStreaming(state.messages),
        }
      }
      return state
    }
    case 'optimistic':
      return { ...state, messages: [...state.messages, action.message] }
    case 'send_failed':
      // 发送未被接受（busy/错误）：撤掉乐观气泡，以系统消息说明原因
      return {
        ...state,
        messages: [
          ...state.messages.filter(m => m.id !== action.id),
          { id: rid(), role: 'system', content: action.reason },
        ],
      }
    case 'confirm_optimistic':
      // 执行中发送（append）：后端不广播 user_message_received（消息已入队为追加指令），
      // 由本 action 本地确认乐观气泡，避免永久 pending 显示。
      return {
        ...state,
        messages: state.messages.map(m =>
          m.id === action.id ? { ...m, pending: false, source: 'mobile' } : m,
        ),
      }
    case 'remove_optimistic':
      // 追加指令受理：移除乐观气泡——追加消息不显示为独立气泡（避免划开 agent
      // 流式气泡），只弹窗提示一句话，与桌面端「前端不显示追加消息」一致。
      return {
        ...state,
        messages: state.messages.filter(m => m.id !== action.id),
      }
    case 'confirm_resolved':
      return { ...state, pendingConfirm: null }
    case 'user_input_resolved':
      return { ...state, pendingUserInput: null }
    case 'refine_resolve':
      // 用户响应提炼提示：关闭弹窗（确认/跳过由 App 层处理 /refine 调用）
      return { ...state, pendingRefine: null }
    case 'refine_state':
      return { ...state, refining: action.refining }
    case 'pause_reset':
      // 继续/终止成功后本地复位暂停态（不等下一轮事件，保证 UI 即时正确）；
      // 恢复时清除 pausedAt（继续计时），保留 startedAt
      return {
        ...state,
        activity: {
          ...state.activity,
          paused: false,
          pauseActionId: undefined,
          pausedAt: undefined,
        },
      }
    case 'identity':
      return { ...state, identity: action.identity }
    case 'workflow_clear':
      // 移除工作流预览（胶囊 X）：清除实时状态（步骤列表/遥控入口一并移除），
      // 标记 dismissed——本轮后续 workflow_event 不再重建；新一轮 run_started 自动重置。
      // 后端执行不受影响（遥控按钮消失但工作流照常跑完）。
      return { ...state, workflowRun: undefined, workflowDismissed: true }
    case 'set_model':
      // 手机端模型卡切换成功后立即更新（不等下次 session_info 事件）
      return { ...state, model: action.model }
  }
}

export function makeOptimisticMessage(content: string, images?: string[]): ChatMessage {
  return {
    id: rid(),
    role: 'user',
    content,
    images: images && images.length > 0 ? images : undefined,
    source: 'mobile',
    pending: true,
    timestamp: Date.now(),
  }
}