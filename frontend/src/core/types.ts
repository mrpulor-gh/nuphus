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

export interface ChatReference {
  type: 'skill' | 'knowledge' | 'workflow' | 'capture'
  id: string // skill name / knowledge rel_path / workflow id / capture file path
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

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant' | 'system' | 'refine'
  content: string
  /** 图片附件（base64 data URL） */
  images?: string[]
  /** 音频附件（base64 data URL 或文件路径） */
  audio?: string[]
  /** skill/knowledge/workflow 引用 */
  references?: ChatReference[]
  /** 拖入的文件路径 */
  files?: string[]
  /** 消息来源徽标（插件 id，source="plugin:{id}" 的 user 消息展示用） */
  sourceLabel?: string
  /** live=正在流式写入中, done=已完成(或历史消息) */
  runtime?: 'live' | 'done'
  timestamp: number
  /** ── refine 消息专用 ── */
  messageCount?: number
  sessionId?: string
  refineStatus?: 'streaming' | 'completed'
  /** 执行过程（思考/流式文本/工具调用，按实际顺序）——历史消息从后端
   *  trace_items 还原，气泡执行回溯入口展示用；实时消息由 useEvents 填充 */
  traceItems?: TimelineEntry[]
}

export interface ToolCall {
  id: string
  toolName: string
  params: unknown
  status: 'running' | 'success' | 'error'
  durationMs: number
  output: string
}

export interface ExecState {
  open: boolean
  stepIndex: number
  goal: string
  tools: string[]
  toolCalls: ToolCall[]
  llmOutput: string
  thinkingText: string
  progress: { iteration: number; maxIterations: number; toolCallsSoFar: number }
  error: string | null
  completed: boolean
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

/** VisualAnchor: 截图标注锚点，对齐后端 types.rs */
export interface VisualAnchor {
  id: string
  screenshot_path: string
  region: { x: number; y: number; width: number; height: number }
  label: string
  ocr_result?: string | null
}

/** 工具调用参数（picker/mouse_pos 捕获的坐标） */
export interface WorkflowStepToolParams {
  /** 选区坐标（picker 模式捕获） */
  region?: {
    x: number
    y: number
    width: number
    height: number
  }
  /** 鼠标点击坐标（mouse_pos 模式捕获） */
  mouse?: {
    x: number
    y: number
  }
}

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
  steps?: unknown[]
  error?: string | null
  variables_snapshot?: Record<string, unknown>
}

// ── ScheduleConfig ──

export interface ScheduleConfig {
  cron: string
  timezone: string
  enabled: boolean
  label?: string
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
  /** 图片降级警告：主模型与 vision 模型都不支持视觉时返回，前端弹窗提示 */
  image_warning?: string
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
  /** Task node fields (used by workflow task_started/task_completed events) */
  taskId?: number
  totalTasks?: number
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
    }
  | { type: 'execution_error'; step_index: number; error: string }
  | {
      type: 'task_started'
      task_id: number
      total_tasks: number
      description: string
    }
  | {
      type: 'task_completed'
      task_id: number
      total_tasks: number
      success: boolean
      description: string
      summary: string
    }
  | {
      type: 'task_list'
      plan_path: string
      tasks: Array<{ id: number; name: string; status: string }>
    }
  | { type: 'seed_generated'; seed_id: string; seed_type: string; summary: string }
  | { type: 'execution_paused'; action_id: string }
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
  | { type: 'warning'; code: string; message: string }
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
  | { type: 'session_changed'; session_id: string; source: 'desktop' | 'mobile' }
  | { type: 'mode_changed'; mode: string }
  | {
      type: 'token_usage'
      input_tokens: number
      output_tokens: number
      cache_hit_tokens: number
      source: string
    }
  | {
      type: 'refine_prompt'
      current_tokens: number
      refine_limit: number
      force_limit: number
      threshold: number
      context_window: number
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
  | { type: 'hud_update'; text: string; phase: string; step_kind?: string | null }
  | { type: 'leader_done'; message: string }
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

export interface ApprovePendingItem {
  id: string
  title: string
  description?: string
  reason?: string
  payload?: unknown
}

export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'cancelled' | 'failed'
export type TaskPriority = 'high' | 'medium' | 'low'

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
