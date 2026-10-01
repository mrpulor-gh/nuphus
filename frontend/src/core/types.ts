// Nuphus Core Types — 前后端对齐

export interface ChatAgentConfig {
  id: string
  name: string
  persona: string
  goal?: string
  constraints: string[]
  requirements: string[]
  knowledge: string[]
  max_iterations: number
}

/** 工作流内联 ChatAgent 条目（带归属信息） */
export interface InlineChatAgentEntry {
  workflow_id: string
  workflow_name: string
  step_id: string
  step_name: string
  config: {
    // 模型参数
    model?: string
    provider?: string
    model_display?: string
    temperature?: number
    max_tokens?: number
    system_prompt?: string
    // Agent 行为参数（对齐 ChatAgentConfig）
    agent_id?: string
    persona?: string
    goal?: string
    constraints?: string[]
    requirements?: string[]
    knowledge?: string[]
    max_iterations?: number
  }
}

/**
 * 一次发送的真实结果（handleSend 返回值）。
 * 供画布等「非输入框发起」的调用方回执使用：ok=false 时 message 为失败原因。
 */
export interface SendOutcome {
  ok: boolean
  message?: string
  /**
   * 后端拒收原因（稳定标识，目前仅 `'finalizing'`）：主循环已退出、后端正在收尾，
   * 追加没有消费方 → 后端**未受理**这条消息（未入队、未记去重基准）。
   *
   * 调用方必须把用户输入**原样退回输入框**并提示稍后重发：
   * 不得静默丢弃，也不得在聊天区留下不会被执行的气泡。
   */
  rejected?: string
  /**
   * 被受理为**追加指令**（执行中发送，与「新回合」区分）：消息已入后端队列，
   * 由当前执行体的迭代边界注入，**不会立即开启新一轮**（工作流类指令是否启动取决于
   * Agent 是否调用对应工具）。
   *
   * 非输入框入口（如「运行工作流」）据此给出「已作为追加指令插入当前任务」的提示，
   * 否则用户会以为自己的工作流/任务已经启动（静默失效）。
   */
  appended?: boolean
}

export interface ChatReference {
  /** quote = 聊天区选中文字引用。label 承载选中原文**全文**（显示侧由
   *  .ref-chip-label 的 ellipsis 截断，后端注入用的是完整 label，两个长度不同但同源）。
   *  刻意不另开 text 字段：mobile_server 复用同一 Rust 结构体，多一个协议位就要双端同步。 */
  type: 'skill' | 'knowledge' | 'workflow' | 'capture' | 'quote'
  id: string // skill name / knowledge rel_path / workflow id / capture file path / quote 内容哈希
  label: string // 显示文本
  meta?: CaptureMeta // extra data for capture type
}

export interface CaptureMeta {
  region: { x: number; y: number; width: number; height: number }
  /** 截图预览图（PNG data URL），渲染优先用此值，绕开 asset 协议 scope 问题 */
  base64?: string
}

export interface PendingFile {
  path: string
  name: string
}

export interface PendingImage {
  dataUrl: string
  name: string
}

export type ApiHealthStatus = 'unknown' | 'connecting' | 'degraded' | 'offline' | 'stable'

export type ApiHealthEventKind =
  'retry' | 'timeout' | 'disconnect' | 'truncated' | 'provider' | 'recovered'

/**
 * 健康事件聚合条目：同 kind 合并为一条（分类说明 + 累计次数 + 首末时间）。
 * 不按 turn 组织——turn 是「用户消息轮次」，跨轮后无对应意义；
 * 时间线以 lastAt 为排序键（最近发生的在前）。
 */
export interface ApiHealthIncident {
  kind: ApiHealthEventKind
  /** 累计发生次数（重复异常合并计数，不产生重复行） */
  count: number
  /** 首次发生时间戳 */
  firstAt: number
  /** 最近发生时间戳（时间线排序键） */
  lastAt: number
  /** 最近一次的说明文案 */
  lastSummary: string
  impact: 'none' | 'partial' | 'failed'
}

export interface ApiHealthState {
  status: ApiHealthStatus
  stableSince: number | null
  lastTransitionAt: number
  currentTurnId: number
  consecutiveFailures: number
  /**
   * 最近一次**重试事件**的结构化进度（直接来自事件，不从 message 反解）：
   * rail 用它渲染「retry 1/3」这类实时数字标签。恢复稳定 / 转 offline 时清空。
   */
  retry: { attempt: number; max: number; at: number } | null
  /** 事件聚合时间线（按 kind 合并，lastAt 倒序展示） */
  incidents: ApiHealthIncident[]
  unreadCount: number
  /** 瞬时事件脉冲（传输截断 / 单次重试）：一次性动效，不改变 status；到期自动清除 */
  pulse: { kind: ApiHealthEventKind; at: number } | null
}

export interface ChatMessage {
  id: string
  kind?: 'progress'
  message_id?: string
  /** Stable presentation container; atomic receipts remain independently addressable. */
  reply_id?: string
  role: 'user' | 'assistant' | 'system' | 'refine'
  content: string
  /** 图片附件（base64 data URL） */
  images?: string[]
  /** 音频附件（base64 data URL 或文件路径） */
  audio?: string[]
  /** skill/knowledge/workflow 引用 + 聊天区选中文字引用（type='quote'）*/
  references?: ChatReference[]
  /** 拖入的文件路径 */
  files?: string[]
  /** 消息来源徽标（插件 id，source="plugin:{id}" 的 user 消息展示用） */
  sourceLabel?: string
  /** live=正在流式写入中, done=已完成(或历史消息) */
  runtime?: 'live' | 'done'
  /** 本次发送首轮 LLM 即失败（无工具执行）——user 气泡 hover 显示「重试」 */
  failed?: boolean
  timestamp: number
  /** ── refine 消息专用 ── */
  messageCount?: number
  sessionId?: string
  refineStatus?: 'streaming' | 'completed'
  /** 执行过程（思考/流式文本/工具调用，按实际顺序）——历史消息从后端
   *  trace_items 还原，气泡执行回溯入口展示用；实时消息由 useEvents 填充 */
  traceItems?: TimelineEntry[]
  /**
   * 本轮执行的元数据（耗时 / token / 步数）——后端 `TurnMeta` 的前端镜像。
   *
   * 唯一数据源：实时来自 `execution_started` / `token_usage` / `execution_progress` /
   * `execution_completed`，历史来自 `HistoryMessage.meta`。执行面板与 ctx 弹窗同源消费，
   * 不允许任何一处自留计时器（原缺陷：起点存组件 ref，刷新即丢）。
   * 旧历史缺省 undefined → 不渲染元数据条（与 traceItems 同一演进策略）。
   */
  meta?: TurnMeta
}

/**
 * 一轮执行的元数据（耗时 / token / 步数）——对齐后端 `agent::turn_meta::TurnMeta`。
 *
 * 全部字段可选：`#[serde(default, skip_serializing_if)]` 使零值 / 缺省字段根本不下发，
 * 前端据此判空（见 `isTurnMetaEmpty`），避免渲染空壳条。
 */
export interface TurnMeta {
  /** 本轮起点（Unix 毫秒，**后端权威**）。执行中前端以 `Date.now() - startedAtMs`
   *  实时推算——刷新 / 重连后仍从真实起点继续，不归零（根治缺陷的根因字段）。 */
  startedAtMs?: number
  /** 本轮总耗时（毫秒）。执行中缺省（前端用起点推算）；完成时由后端给出权威值。 */
  durationMs?: number
  /** 输入 token 累计（本轮所有 LLM 调用之和） */
  inputTokens?: number
  /** 输出 token 累计 */
  outputTokens?: number
  /** 缓存命中 token 累计 */
  cacheHitTokens?: number
  /** 迭代轮次（ReAct 循环走了几轮） */
  iterations?: number
  /** 工具调用次数（本轮实际发生的 call 数） */
  toolCalls?: number
  /**
   * 本轮**上下文增量**（tokens）：轮末上下文占用 − 轮初占用，后端权威
   * （`TurnMeta::context_delta_tokens`，两个占用值都取自 API usage）。
   *
   * 元数据条展示它而不是「累计消耗」：同一段上下文在同轮多次调用里被反复计入
   * inputTokens，累加和随调用次数线性膨胀（130K × 40 次 = 5.2M），是没有信息量
   * 的数字；增量回答「这一轮让上下文长了多少」。
   */
  contextDeltaTokens?: number
  /** 解码速度（output tokens / 解码秒数） */
  genTps?: number
  /** 首 token 延迟（毫秒） */
  ttftMs?: number
}

/**
 * 元数据是否「无内容可展示」——对齐后端 `TurnMeta::is_empty()` 的判据
 * （duration / input / output / iterations / toolCalls 全空即空）。
 *
 * 用途：三处接入点统一据此短路，不渲染空壳元数据条（与 `trace_items` 同策略）。
 */
export function isTurnMetaEmpty(meta: TurnMeta | null | undefined): boolean {
  if (!meta) return true
  return (
    (meta.durationMs == null || meta.durationMs === 0) &&
    !meta.inputTokens &&
    !meta.outputTokens &&
    !meta.iterations &&
    !meta.toolCalls
  )
}

export interface ToolCall {
  id: string
  toolName: string
  params: unknown
  status: 'running' | 'success' | 'error'
  durationMs: number
  output: string
}

// ── 元数据取值：唯一出口 ──
// 所有消费点（消息底部 / ctx 弹窗 / 执行回溯顶栏 / 移动端 / ThinkingIndicator）
// **必须**经此读取，不得各自拼兜底链——否则同一轮指标会出现多个不一致的值。
// `turnMeta` 本身由后端权威字段构成，故这里不再叠加 fallback：
//   · toolCalls ← execution_progress.tool_calls_so_far（执行中实时）
//                 / execution_completed.meta.tool_calls（完成权威）
/** 本轮工具调用步数。无数据返回 0（不可知，不拿别处的值充数）。 */
export function resolveTurnCalls(meta: TurnMeta | null | undefined): number {
  return meta?.toolCalls || 0
}

/**
 * 本轮耗时（毫秒）唯一出口。
 *
 * 规则只有两条，**没有兜底链**：
 *   ① 本轮已结束（`durationMs` 有值）→ 后端权威总耗时
 *   ② 本轮进行中（`startedAtMs` 有值）→ `now - startedAtMs` 实时推算
 *
 * `startedAtMs` 由两条通道保证存在，故刷新 / 重连也不归零、不需 fallback：
 *   - 事件通道：`execution_started.started_at_ms`
 *   - 轮询通道：`get_execution_state` 快照的 `started_at_ms`
 *     （后端 `SignalState::set_execution_stage` 在「空闲 → 执行」那一跳记录）
 *
 * 两者皆无 = 后端确实没给起点（后端不可达 / 旧版本）→ 返回 0（不可知，不伪造）。
 */
export function resolveTurnDuration(
  meta: TurnMeta | null | undefined,
  now: number = Date.now(),
): number {
  const finalMs = meta?.durationMs
  if (finalMs != null && finalMs > 0) return finalMs
  const startedAtMs = meta?.startedAtMs
  if (startedAtMs != null && startedAtMs > 0) return Math.max(0, now - startedAtMs)
  return 0
}

/**
 * 轮询快照（`get_execution_state`）合入本轮 turnMeta —— 步数实时值的唯一入口。
 *
 * 后端 `SignalState::execution_tool_calls` 只在本轮「空闲 → Running」那一跳归零
 * （`set_execution_stage`），之后单调递增，故**快照值即实时权威值，直接落定**。
 * 不得与 prev 做 `prev.toolCalls || 快照` 之类合并：那会把首个非零值冻住，
 * 之后更大的快照全被丢弃。主轮（react_loop）不发 execution_progress，轮询是
 * 普通会话唯一的实时步数通道，该合并缺陷表现为「步数恒久显示为 1」。
 *
 * 起点反之「只在缺省时补」：事件通道（`execution_started.started_at_ms`）已写过的
 * 起点不被快照覆盖，避免混入别的轮次 / 别的窗口的起点。
 *
 * 「当前无执行」（`started_at_ms = null` 且 `tool_calls = 0`）由调用方整体跳过，
 * 本函数不做该判断——保持纯函数、无隐藏分支。同理，提炼（内部执行，见
 * `RefineStreamFilter`）期间调用方也不得把快照写进用户轮的 turnMeta。
 */
export function applyTurnSnapshot(
  prev: TurnMeta | null | undefined,
  snapshot: { startedAtMs: number | null; toolCalls: number },
): TurnMeta {
  return {
    ...(prev ?? {}),
    startedAtMs: prev?.startedAtMs ?? snapshot.startedAtMs ?? undefined,
    toolCalls: snapshot.toolCalls || undefined,
  }
}

export interface UserInputRequest {
  actionId: string
  title: string
  prompt: string
  sensitive: boolean
  inputType: string
  // ── icon_confirm ──
  iconPath?: string | null
  defaultName?: string | null
  defaultShortcut?: string | null
  relX?: number | null
  relY?: number | null
  defaultNote?: string | null
  // ── step_form：预填当前阶段名（WorkflowAgent 自填，用户可改）──
  defaultStage?: string | null
}

export interface SecurityCheck {
  actionId: string
  tool: string
  params: string
  risk: 'low' | 'medium' | 'high' | 'critical'
  reason: string
}

export interface ToolSchema {
  name: string
  description: string
  input_schema: Record<string, unknown>
  permission?: string
  /** 画布工具面板展示分组键（wf_tools 下发；旧缓存/异常缺省时前端归 misc） */
  group?: string
}

// ── Backend command return types ──

export interface MemoryStats {
  total_entries: number
  patterns: number
  skills: number
  principles: number
  templates: number
}

export interface TimelineIndexStats {
  total_entries: number
  total_sessions: number
  successful: number
  failed: number
  by_intent: Record<string, number>
}

export interface DesktopStatus {
  connected: boolean
  python_path: string
  tools_count: number
}

export interface HookScriptInfo {
  path: string
  exists: boolean
  size_bytes: number
}

export interface HooksConfigStatus {
  pre_tool_call: HookScriptInfo | null
  post_tool_call: HookScriptInfo | null
  on_session_start: HookScriptInfo | null
  on_session_end: HookScriptInfo | null
  config_path: string
}

export interface SessionSummary {
  session_id: string
  user_message: string
  intent: string
  last_assistant_message: string
  entry_count: number
  tool_call_count: number
  timestamp: string
  success: boolean
  tags: string[]
}

export interface SessionDetailEntry {
  id: string
  kind: string
  user_message: string
  assistant_message: string
  steps_summary: string[]
  goal_type: string | null
  timestamp: string
  success: boolean
}

// ── Workflow V2 ──

// ── OnError ──

export type OnError =
  | 'abort'
  | 'skip'
  | { retry: { max: number; backoff_ms?: number; backoff_multiplier?: number } }
  | { allow_codes: { codes: number[] } }

// ── Condition（对齐后端 types.rs untagged 12 变体）──

export type VarRef = { var: string } | string

export type Condition =
  | { equals: VarRef[] }
  | { not_equals: VarRef[] }
  | { contains: VarRef[] }
  | { starts_with: VarRef[] }
  | { regex: VarRef[] }
  | { not_empty: VarRef }
  | { empty: VarRef }
  | { gt: VarRef[] }
  | { lt: VarRef[] }
  | { gte: VarRef[] }
  | { lte: VarRef[] }
  | { always: boolean }

// ── ForEachDef ──

export interface ForEachDef {
  items: VarRef
  as?: string
}

// ── LoopDef (V2) ──

export interface LoopDef {
  for_each?: ForEachDef
  repeat?: number
  until?: Condition
  max?: number
  do: WorkflowStep[]
}

// ── IfDef ──

export interface IfDef {
  condition: Condition
  then: WorkflowStep[]
  else?: WorkflowStep[]
}

// ── ScriptDef ──

export interface ScriptDef {
  runtime: string
  code: string
  cwd?: string
}

// ── AssertDef ──

export interface AssertDef {
  condition: Condition
  message?: string
}

// ── McpDef ──

export interface McpDef {
  server: string
  tool: string
  with?: Record<string, unknown>
}

// ── ChatOpts ──

export interface ChatOpts {
  agent_id?: string
  screenshot?: boolean
  max_steps?: number
  tools?: string[]
  knowledge?: string[]
  model?: string
  provider?: string
  model_display?: string
  temperature?: number
  max_tokens?: number
  system_prompt?: string
  persona?: string
  goal?: string
  constraints?: string[]
  requirements?: string[]
  max_iterations?: number
}

// ── Action (13 种 + Custom) ──

export type Action =
  | { tool: string; with?: Record<string, unknown> }
  | { seq: WorkflowStep[] }
  | { loop: LoopDef }
  | { if: IfDef }
  | { call: string; with?: Record<string, unknown> }
  | { wait: string; auto?: WorkflowStep[] }
  | { chat: string; with?: ChatOpts }
  | { script: ScriptDef }
  | { assert: AssertDef }
  | { mcp: McpDef }
  | { sleep: number }
  | { break: boolean }
  | { continue: boolean }
  | { [key: string]: unknown }

// ── WorkflowStep (V2) ──

export interface WorkflowStep {
  id: string
  name: string
  description?: string
  on_error?: OnError
  capture?: string
  timeout_secs?: number
  do: Action
}

// ── RunRecord（对齐后端 RunStatus 枚举）──

export interface RunRecord {
  run_id: string
  started_at: string
  finished_at?: string | null
  status: 'Running' | 'Success' | 'Cancelled' | 'Paused' | { Error: string }
  steps?: StepRunRecord[]
  error?: string | null
  variables_snapshot?: Record<string, unknown>
}

export interface StepRunRecord {
  step_id: string
  started_at: string
  finished_at?: string | null
  status: 'Running' | 'Success' | 'Skipped' | { Error: string }
  output_summary?: string | null
}

// ── ScheduleConfig ──

export interface ScheduleConfig {
  cron: string
  timezone: string
  enabled: boolean
  label?: string
  interval_minutes?: number
}

// ── WorkflowInputSpec（后端 InputSpec 镜像：workflow.inputs[]）──

/** 输入类型（后端 InputKind，JSON 字段名 type） */
export type WorkflowInputKind = 'string' | 'number' | 'boolean' | 'path' | 'json'

/** 工作流外部输入声明（运行前由 UI 收集，注入变量池 inputs.x / 顶层 x） */
export interface WorkflowInputSpec {
  name: string
  /** 控件类型，缺省 string */
  type?: WorkflowInputKind
  /** 必填：无 default 且未填 → 运行前阻断 */
  required?: boolean
  /** 默认值（未填时后端兜底注入；UI 预填） */
  default?: unknown
  description?: string
  /** 敏感值：密码控件且不写日志/事件；本机运行快照仍会保留值 */
  sensitive?: boolean
}

// ── WorkflowItem (V2) ──

export interface WorkflowItem {
  id: string
  title: string // ← 后端 name 映射
  description?: string // ← 后端 doc 映射
  steps: WorkflowStep[]
  tags: string[]
  created_at: number // Unix timestamp (秒)
  updated_at: number
  run_count: number // ← run_history.length
  status: 'draft' | 'active' | 'archived'
  // V2 新增字段
  schedule?: ScheduleConfig | null
  run_history?: RunRecord[]
  timeout_secs?: number | null
  dry_run?: boolean
  doc?: string | null
  /** 外部输入声明（后端空数组不下发 → undefined） */
  inputs?: WorkflowInputSpec[]
}

// ── Knowledge ──

export interface KnowledgeHit {
  rel_path: string
  title: string
  tags: string[]
  snippet: string
  file_mtime: number
}

export interface SessionInfo {
  version: string
  name: string
  description: string
}

export interface ProcessInputResponse {
  success: boolean
  message: string
  /** 执行中发送被接受为追加指令（不开启新执行） */
  appended?: boolean
  /** 收尾期拒收原因（"finalizing"）：消息未被受理，调用方须把原文退回输入框 */
  rejected?: string
  /** 图片降级警告：主模型与 vision 模型都不支持视觉时返回，前端弹窗提示 */
  image_warning?: string
  /** 本次执行已执行工具步数（后端 output.steps.len()）。
   *  失败分流依据：0 = 首轮 LLM 调用即失败（user 气泡可重试）；
   *  >0 = 已执行工具后失败（优雅停止，不重试） */
  steps_count?: number
}

// ── TimelineEntry (从 App.tsx / ExecutionPanel.tsx 提取，避免重复定义) ──

export interface TimelineEntry {
  id: string
  kind: 'thinking' | 'text' | 'tool_call' | 'reminder' | 'task'
  text?: string
  toolName?: string
  params?: unknown
  status?: 'running' | 'success' | 'error'
  durationMs?: number
  output?: string
  /** 逐行输出（流式追加，终端模式逐行渲染用） */
  outputLines?: string[]
  outputFullSize?: number
  isTruncated?: boolean
  count?: number
  maxCount?: number
  summary?: string
  fromTask?: boolean
  /** 执行面板任务行字段（来自服务端 TaskRun 台账快照，键为 run_id） */
  runId?: string
}

export interface ToolExecuteResult {
  success: boolean
  output: string
  error: string
}

// ── Nuphus Events (serde tagged enum, 对齐后端 src/agent/events.rs) ──

export type NuphusEvent =
  | {
      type: 'execution_started'
      step_index: number
      goal: string
      tools: string[]
      source: string
      mode?: string
      session_id?: string
      turn_id?: string
      /** 本轮起点（Unix 毫秒，**后端权威**）——前端据此实时推算耗时，
       *  刷新 / 重连后依然准确（根治「刷新后计时归零」）。旧后端缺省 = 无起点。 */
      started_at_ms?: number
    }
  | {
      type: 'assistant_progress'
      session_id: string
      turn_id: string
      message_id: string
      text: string
      timestamp: number
      replaces_draft: boolean
    }
  | {
      type: 'tool_call_start'
      call_id: string
      tool_name: string
      params: unknown
      iteration: number
      from_task: boolean
    }
  | { type: 'tool_output_line'; call_id: string; line: string; is_stderr: boolean }
  | {
      type: 'tool_call_end'
      call_id: string
      tool_name: string
      success: boolean
      duration_ms: number
      output_preview: string
      output_full_size: number
      is_truncated: boolean
      error: string | null
      from_task: boolean
    }
  | { type: 'llm_text_delta'; text: string; is_thinking: boolean; from_task: boolean }
  | { type: 'image_generated'; url: string }
  | {
      type: 'execution_progress'
      iteration: number
      max_iterations: number
      tool_calls_so_far: number
    }
  | {
      type: 'execution_completed'
      step_index: number
      output: {
        step_index: number
        result_message: string
        artifacts: string[]
        tool_calls_count: number
      }
      total_duration_ms: number
      total_calls: number
      /** 本轮元数据（耗时 / token / 步数）。旧字段 `total_duration_ms` /
       *  `total_calls` 语义不变；`meta` 承载 token 等新维度，二者同源不漂移。 */
      meta?: TurnMeta
    }
  | { type: 'execution_error'; step_index: number; error: string }
  /**
   * ExecAgent 执行生命周期**全量快照**（后端 NuphusEvent::TaskRuns）。
   * task 面板的唯一数据源：前端只渲染，不做 id 配对/状态推断。
   */
  | {
      type: 'task_runs'
      runs: TaskRun[]
    }
  | { type: 'seed_generated'; seed_id: string; seed_type: string; summary: string }
  | { type: 'execution_paused'; action_id: string }
  | { type: 'append_queue_updated'; messages: string[] }
  | {
      type: 'security_check'
      action_id: string
      tool: string
      params: string
      risk: 'low' | 'medium' | 'high' | 'critical'
      reason: string
    }
  | {
      type: 'user_input_request'
      action_id: string
      title: string
      prompt: string
      sensitive: boolean
      input_type: string
      icon_path?: string | null
      default_name?: string | null
      default_shortcut?: string | null
      rel_x?: number | null
      rel_y?: number | null
      default_note?: string | null
      default_stage?: string | null
    }
  | { type: 'prompt_timeout'; action_id: string }
  | {
      type: 'agent_reminder'
      kind: string
      count: number
      max_count: number
      text: string
    }
  | {
      type: 'user_message_received'
      content: string
      source: string
      /** 图片 data URL 列表（已冻结 PNG），前端 user 消息渲染 */
      images?: string[]
    }
  | { type: 'direct_response'; message: string }
  | {
      type: 'warning'
      code: string
      message: string
      /** 重试类事件（llm_retry / llm_network_retry）的结构化进度；非重试事件省略 */
      attempt?: number
      max_attempts?: number
    }
  | {
      type: 'error'
      code: string
      message: string
      retryable: boolean
      from_subtask?: boolean
    }
  | {
      type: 'goal_type_identified'
      goal_type: string
      label: string
      confidence: number
      max_iterations: number
    }
  | {
      type: 'understanding_complete'
      summary: string
      critiques: string[]
      needs_clarification: boolean
      confidence: number
    }
  | { type: 'session_info'; session_id: string; model: string; timestamp: number }
  /** 后端已受理用户消息（开启新执行 或 进入追加队列）：真实发送成功时点，
   *  与整轮执行完成（send_message_cmd 返回）区分——画布据此立即收起遮罩回对话。
   *  send_id 缺省 = 老调用方（无精确对齐需求）。 */
  | { type: 'message_accepted'; send_id?: string | null; source: string }
  | { type: 'mode_changed'; mode: string }
  | {
      type: 'token_usage'
      input_tokens: number
      output_tokens: number
      cache_hit_tokens: number
      source: string
      /** 解码速度（output tokens / 首 token→结束秒），仅 exec 源携带；缺省 = 无数据 */
      gen_tps?: number
      /** 首 token 延迟（毫秒）：请求发出 → 首个内容 chunk；仅 exec 源携带 */
      ttft_ms?: number
    }
  | {
      type: 'refine_prompt'
      current_tokens: number
      refine_limit: number
      force_limit: number
      threshold: number
      context_window: number
      /** 当前模型上下文窗口的分档（后端 RefineTier）：small 无提示 / medium 固定双线 / large 可调 */
      tier?: string
      /** 当前生效的强制线比例（0~1） */
      force_threshold?: number
      /** 可调范围（0~1）；非 large 档同样下发，UI 按 tier 决定是否渲染 slider */
      force_min?: number
      force_max?: number
      forced: boolean
    }
  | { type: 'refine_executing' }
  | { type: 'refine_skipped' }
  | {
      type: 'session_refined'
      summary: string
      message_count: number
      session_id: string
    }
  /** 提炼失败（LLM 调用失败/超时/空摘要）：与 refine_executing 配对的结束事件，
   *  双端据此退出"提炼中"UI 并展示错误——缺失会导致永久 spinner（假死） */
  | { type: 'refine_failed'; message: string }
  | { type: 'hud_update'; text: string; phase: string; step_kind?: string | null }
  | { type: 'leader_done'; message: string }
  /** 会话切换（桌面 rail 或手机遥控触发）：手机收到后重拉 /history 跟随桌面当前视图 */
  | { type: 'session_changed'; session_id: string }
  /** 展示台列表变化（桌面归档/重命名会话）：手机刷新会话清单，当前会话未变 */
  | { type: 'shelf_updated' }
  /** 新建对话纯意图广播（手机 /new-chat 触发）：后端不创建任何 session，
   *  桌面端收到执行本地 handleNewChat（清聊天区回欢迎页）；手机端本机已先行清视图 */
  | { type: 'new_chat_broadcast' }
  | {
      type: 'workflow_event'
      /** WorkflowEvent 子类型（run_started / step_run_started / step_run_completed /
       *  step_run_paused / run_completed / error 等），与桌面 workflow-event payload 对齐 */
      event: string
      run_id?: string
      workflow_id?: string
      step_id?: string
      step_name?: string
      /** RunStatus / StepRunStatus serde 形状：'Success' | 'Skipped' | 'Running' | { Error: string } */
      status?: unknown
      depth?: number
      kind?: string
      message?: string
    }

// ── Plan / Task types ──

export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'cancelled' | 'failed'
export type TaskPriority = 'high' | 'medium' | 'low'

// ── ExecAgent 执行生命周期（服务端台账快照，见后端 agent/task_run.rs）──
/** 没有 pending：未派发的任务属于计划文档的意图，不属于执行生命周期 */
export type TaskRunState = 'running' | 'completed' | 'failed' | 'interrupted'

export interface TaskRunOrigin {
  plan_path: string
  task_no: number | null
}

export interface TaskRun {
  /** 服务端下发的唯一身份（进程内单调 */
  run_id: string
  title: string
  /** 派发正文全文（给 Exec 的那份：任务定义 / 上下文等） */
  task: string
  goal_type: string
  origin: TaskRunOrigin | null
  /** 同标题/同归属的第几次执行（重试计数，从 1 开始） */
  attempt: number
  state: TaskRunState
  started_at: number
  settled_at: number | null
  duration_ms: number | null
  ok: boolean | null
  summary: string | null
}

export interface PlanTask {
  id: number
  name: string
  understanding: string
  status: TaskStatus
  priority: TaskPriority
}

export interface PlanData {
  project: string
  topic: string
  goalType: string
  requirement: string
  status: string
  context: string
  tasks: PlanTask[]
  planPath: string
}

export interface WorkflowRunStep {
  id: string
  name: string
  status: 'pending' | 'running' | 'completed' | 'failed' | 'paused'
  depth?: number
  kind?: string
}
