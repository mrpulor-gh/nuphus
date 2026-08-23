// Nuphus API — typed wrappers for all backend Tauri commands
import { invoke } from '../../core/bridge'
import type { RelationConfig } from './relation'
import type {
  ToolSchema,
  MemoryStats,
  TimelineIndexStats,
  DesktopStatus,
  HooksConfigStatus,
  SessionInfo,
  ProcessInputResponse,
  ToolExecuteResult,
  WorkflowItem,
  WorkflowStep,
  Action,
  OnError,
  LoopDef,
  IfDef,
  ScheduleConfig,
  RunRecord,
  ChatAgentConfig,
  InlineChatAgentEntry,
} from '../../core/types'

// ── Tools ──

export function getTools() {
  return invoke<ToolSchema[]>('get_tools')
}

export function executeTool(toolName: string, params: Record<string, unknown>) {
  return invoke<ToolExecuteResult>('execute_tool', { toolName, params })
}

// ── Memory ──

export function getMemoryStats() {
  return invoke<MemoryStats>('get_memory_stats')
}

export function getTenets() {
  return invoke<{
    count: number
    total: number
    items: Array<{
      id: string
      content: string
      priority: string
      enforce: string
      active: boolean
      created_at: string
    }>
  }>('get_tenets')
}

export function deleteTenet(id: string) {
  return invoke<boolean>('delete_tenet', { id })
}

export function addTenet(content: string, priority?: string) {
  return invoke<void>('add_tenet', { content, priority })
}

export function getKnowledgeItems(category: string) {
  return invoke<unknown[]>('get_knowledge_items', { category })
}

export function deleteKnowledgeItem(category: string, id: string) {
  return invoke<boolean>('delete_knowledge_item', { category, id })
}

// ── 新知识库 API ──

import type { KnowledgeHit } from '../../core/types'

export function searchKnowledge(query: string, tags?: string[], maxResults?: number) {
  return invoke<KnowledgeHit[]>('search_knowledge', { query, tags, maxResults })
}

export function listKnowledge() {
  return invoke<KnowledgeHit[]>('list_knowledge')
}

export function listKnowledgeTags() {
  return invoke<string[]>('list_knowledge_tags')
}

export function deleteKnowledge(relPath: string) {
  return invoke<boolean>('delete_knowledge', { relPath })
}

// ── Config ──

export function configureLlm(
  apiKey: string,
  model?: string,
  provider?: string,
  baseUrl?: string,
  contextWindow?: number,
) {
  return invoke<string>('configure_llm', { apiKey, model, provider, baseUrl, contextWindow })
}

/** 清除指定 provider 已存储的 API Key（仅清 key，保留 provider/model 配置） */
export function clearProviderApiKey(provider: string) {
  return invoke<void>('clear_provider_api_key', { provider })
}

export function getSessionInfo() {
  return invoke<SessionInfo>('get_session_info')
}

/** 历史执行过程条目（对齐后端 state::HistoryTraceItem，serde camelCase） */
export interface HistoryTraceItem {
  kind: string // "thinking" | "text" | "tool"
  call_id?: string
  name?: string
  status?: string // "running" | "ok" | "fail"
  params?: string
  text?: string
}

export interface HistoryMessage {
  role: string
  content: string
  images: string[]
  audio: string[]
  /** 消息创建时间（Unix 毫秒）；旧数据可能缺失 */
  timestamp?: number
  /** 执行过程（思考/流式文本/工具调用，按实际顺序）——Session 完整存储 */
  traceItems?: HistoryTraceItem[]
}

export function getChatHistory() {
  return invoke<HistoryMessage[]>('get_chat_history')
}

export interface LLmConfig {
  api_key: string
  has_key: boolean
  model: string
  provider: string
  base_url: string
  configured_providers?: string[]
}

export function getCurrentConfig() {
  return invoke<LLmConfig | null>('get_current_config')
}

export function isLlmConfigured() {
  return invoke<boolean>('is_llm_configured')
}

// ── History ──

export function getSessionHistory() {
  return invoke<import('../../core/types').SessionSummary[]>('get_session_history')
}

export function getSessionDetail(sessionId: string) {
  return invoke<import('../../core/types').SessionDetailEntry[]>('get_session_detail', {
    sessionId,
  })
}

// ── Stats ──

export function getTimelineIndexStats() {
  return invoke<TimelineIndexStats>('get_timeline_index_stats')
}

export function getMemoryOverview() {
  return invoke<import('../../core/types-memory').MemoryOverview>('get_memory_overview')
}

// ── Desktop ──

export function getDesktopStatus() {
  return invoke<DesktopStatus>('get_desktop_status')
}

// ── Hooks ──

export function getHooksStatus() {
  return invoke<HooksConfigStatus>('get_hooks_status')
}

// ── Control ──

export async function retryAgent(): Promise<ProcessInputResponse | null> {
  return invoke<ProcessInputResponse>('retry_agent')
}

export async function processInput(
  input: string,
  history?: { role: string; content: string }[],
  relation?: RelationConfig,
  sendId?: string,
  mode?: string,
  images?: string[],
  references?: { type: string; id: string; label: string }[],
): Promise<ProcessInputResponse | null> {
  // 首次尝试通过 Tauri IPC 发送
  const firstResult = await invoke<ProcessInputResponse>('send_message_cmd', {
    message: input,
    history,
    relation,
    sendId,
    mode,
    images,
    references,
  })

  // invoke 返回 null 说明 IPC 调用失败（ERR_CONNECTION_REFUSED 等）
  if (firstResult === null) {
    console.warn('[API] processInput: invoke returned null (IPC may be down)')
    return null
  }

  return firstResult
}

export function interrupt() {
  return invoke<string>('interrupt')
}

export function pauseExecution() {
  return invoke<string>('pause_execution')
}

export function continueExecution(actionId: string) {
  return invoke<string>('continue_execution', { actionId })
}

export function appendInstruction(actionId: string, instruction: string) {
  return invoke<string>('append_instruction', { actionId, instruction })
}

export function terminateExecution(actionId: string) {
  return invoke<string>('terminate_execution', { actionId })
}

export function gracefulStop() {
  return invoke<string>('graceful_stop')
}

export function isBusy() {
  return invoke<boolean>('is_busy')
}

export function forceReset() {
  return invoke<string>('force_reset')
}

export function setMode(mode: string) {
  return invoke<string>('set_mode', { mode })
}

// ── Custom Agents（自定义 Agent，全体用户可用）──

export interface CustomAgentConfig {
  id: string
  name: string
  l2_prompt: string
  tools: string[]
  greeting: string
  knowledge: string[]
  created_at: string
  updated_at: string
}

export function listCustomAgents() {
  return invoke<CustomAgentConfig[]>('list_custom_agents')
}

export function saveCustomAgent(config: CustomAgentConfig) {
  return invoke<CustomAgentConfig>('save_custom_agent', { config })
}

export function deleteCustomAgent(id: string) {
  return invoke<void>('delete_custom_agent', { id })
}

export function getActiveCustomAgent() {
  return invoke<CustomAgentConfig | null>('get_active_custom_agent')
}

export function setActiveCustomAgent(id: string) {
  return invoke<CustomAgentConfig>('set_active_custom_agent', { id })
}

// ── External Agents（外部 Agent 工作台 / handoff 运行时态）──

/** status.json 运行时态（后端字段原样映射，前端只做显示层转换） */
export interface ExternalAgentStatus {
  agent: string
  state: string
  task_id?: string
  last_event?: {
    status?: string
    summary?: string
    report_path?: string | null
    ts?: string
  } | null
  updated_at?: string
}

/** 列出所有已初始化外部 agent 的运行时态（按 agent 名排序） */
export function listAgentStatuses() {
  return invoke<ExternalAgentStatus[]>('list_agent_statuses')
}

/** 单个交付物条目：briefs/*-report.md（report）或 projects/** 产物文件（artifact） */
export interface AgentDeliverable {
  path: string
  name: string
  rel_path: string
  kind: 'report' | 'artifact'
  size: number
  modified: string
}

/** 列出某外部 agent 的交付物（任务报告 + projects/ 产物，按修改时间降序） */
export function listAgentDeliverables(agent: string) {
  return invoke<AgentDeliverable[]>('list_agent_deliverables', { agent })
}

// ── Session Shelf（浅层会话展示台）──

export interface ShelfSessionItem {
  id: string
  mode: string
  title: string
  message_count: number
  updated_at: number
  is_active: boolean
}

export interface ShelfListResponse {
  /** false = busy 或追加队列非空，切换被后端拒绝 */
  can_switch: boolean
  items: ShelfSessionItem[]
}

/** 展示台列表：active 置顶 + newest-first */
export function listShelfSessions() {
  return invoke<ShelfListResponse>('list_shelf_sessions')
}

/** 切换会话（同 backing mode）；失败 reject 稳定错误码字符串 */
export function switchSession(id: string) {
  return invoke<void>('switch_session', { id })
}

/** 新建对话：归档当前 → 安装空白会话，返回新会话 id */
export function newChatSessionCmd() {
  return invoke<string>('new_chat_session_cmd')
}

/** 重命名会话（落 sessions.summary 元数据行） */
export function renameSession(id: string, title: string) {
  return invoke<void>('rename_session_cmd', { id, title })
}

/** 是否存在可恢复的最近会话镜像（欢迎页「继续对话」按钮显示条件） */
export function hasResumeCandidate() {
  return invoke<boolean>('has_resume_candidate')
}

/** 继续对话：最新镜像写入 session_backup，返回完整历史（下条消息即续聊） */
export function resumeLatestSession() {
  return invoke<HistoryMessage[]>('resume_latest_session')
}

/** 初始化外部 agent 工作目录（幂等，返回目录绝对路径） */
export function agentInit(agent: string, description: string) {
  return invoke<string>('agent_init', { agent, description })
}

/** 派发任务：写 brief + 置 in_progress，返回含门铃 URL/token 的契约字符串（token 仅出现在返回值，不落盘） */
export function handoffEnsure(agent: string, taskId: string, brief: string) {
  return invoke<string>('handoff_ensure', { agent, task_id: taskId, brief })
}

// ── External Agents 配置中心（plugin/team.toml CRUD）──

/** 外部 Agent 登记项（team.toml 段 → 扁平字段，含默认值补全） */
export interface ExternalAgentConfig {
  key: string
  display_name: string
  icon: string
  /** 交互协议（由后端按 mode 归并：background/embedded→terminal、standalone→desktop、web→web-ui） */
  type?: string
  mode: string
  open: string
  args?: string
  process?: string
  description?: string
  note?: string
}

/** 列出全部外部 Agent（按 key 排序） */
export function listExternalAgents() {
  return invoke<ExternalAgentConfig[]>('list_external_agents')
}

/** 新增/更新外部 Agent（新 agent 时后端联动 agent_init 生成 handoff 目录），返回 'created'|'updated' */
export function upsertExternalAgent(agent: ExternalAgentConfig) {
  return invoke<string>('upsert_external_agent', { agent })
}

/** 删除外部 Agent 段（不删除 .nuphus/handoff/{key}/ 目录） */
export function deleteExternalAgent(key: string) {
  return invoke<void>('delete_external_agent', { key })
}

/** 提取应用图标为 data URL（图片文件直接编码；exe/dll/ico 提取关联图标转 PNG） */
export function extractAgentIcon(path: string) {
  return invoke<string>('extract_agent_icon', { path })
}

// ── Permissions ──

export interface ToolPermissions {
  file_access: boolean
  web_search: boolean
  system_automation: boolean
}

export function setToolPermissions(
  file_access: boolean,
  web_search: boolean,
  system_automation: boolean,
) {
  return invoke<string>('set_tool_permissions', {
    fileAccess: file_access,
    webSearch: web_search,
    systemAutomation: system_automation,
  })
}

export function getToolPermissions() {
  return invoke<string>('get_tool_permissions')
}

export function getBrowserCdpUrl() {
  return invoke<string>('get_browser_cdp_url')
}

/** Identity of a picked external (fingerprint) browser — persisted alongside
 * the CDP URL so a reopened window (new random debug port) can be re-resolved. */
export interface BrowserIdentity {
  name: string
  exe_path: string
  user_data_dir?: string | null
}

export function setBrowserCdpUrl(url: string, identity?: BrowserIdentity | null) {
  return invoke<string>('set_browser_cdp_url', { url, identity: identity ?? null })
}

/** Current connection as shown on the settings page status card. */
export interface BrowserConnection {
  url: string
  name?: string | null
  exe_path?: string | null
  user_data_dir?: string | null
}

export function getBrowserConnection() {
  return invoke<BrowserConnection>('get_browser_connection')
}

export function testBrowserCdpUrl(url: string) {
  return invoke<string>('test_browser_cdp_url', { url })
}

export interface DetectedBrowser {
  name: string
  exe_path: string
  port: number
  url: string
  version: string
  pages: string[]
  user_data_dir?: string | null
}

export function detectCdpBrowsers() {
  return invoke<DetectedBrowser[]>('detect_cdp_browsers')
}

export interface ModelInfo {
  id: string
  provider: string
  alias: string[]
  supports_streaming: boolean
  supports_vision: boolean
  supports_audio: boolean
  reasoning_efforts: string[]
  /** 模型默认推理强度（未配置时生效；null = 无声明，UI 显示「默认」） */
  default_effort?: string | null
}

export function listModels() {
  return invoke<ModelInfo[]>('list_models')
}

export function getDefaultModel() {
  return invoke<string>('get_default_model')
}

// ── Model Switch (provider-driven: reads key from config.toml, no key param) ──

/** mode: leader/workflow/exec/custom/global —— 切换写入对应 agent 模型配置（高级设置联动） */
export function switchModel(
  model: string,
  provider: string,
  baseUrl?: string,
  contextWindow?: number,
  mode?: string,
) {
  return invoke<string>('switch_model', { model, provider, baseUrl, contextWindow, mode })
}

// ── Agent 级模型配置（高级设置） ──

export interface AgentModels {
  default: string
  leader: string
  workflow: string
  exec: string
  custom: string
}

/** 读取高级设置：各 agent 的模型（空串 = 跟随 global / 系统默认） */
export function getAgentModels() {
  return invoke<AgentModels>('get_agent_models')
}

/** 计算某 mode 的生效模型（输入框显示用） */
export function getEffectiveModel(mode: string) {
  return invoke<string>('get_effective_model', { mode })
}

/** 设置某个 agent 的模型；model 空串 = 清除（跟随全局 fallback） */
export function setAgentModel(agent: string, model: string) {
  return invoke<string>('set_agent_model', { agent, model })
}

// ── Reasoning Effort ──

/** Read the reasoning-effort configured for a provider (null = provider default). */
export function getReasoningEffort(provider: string) {
  return invoke<string | null>('get_reasoning_effort', { provider })
}

/** Persist reasoning-effort for a provider; null/empty clears the setting. */
export function setReasoningEffort(provider: string, effort: string | null) {
  return invoke<string>('set_reasoning_effort', { provider, effort })
}

// ── Provider & Connection Test ──

/** List all available models for a provider via /v1/models (validates API key) */
export function listProviderModels(apiKey: string, provider: string, baseUrl?: string) {
  return invoke<string[]>('list_provider_models', { apiKey, provider, baseUrl })
}

/** 刷新服务商最新模型列表：用 config.toml 已存 API key（不暴露 key），模型列表页刷新按钮使用 */
export function refreshProviderModels(provider: string, baseUrl?: string) {
  return invoke<string[]>('refresh_provider_models', { provider, baseUrl })
}

export interface ProviderInfo {
  id: string
  name: string
  base_url: string
  default_model: string
  auth_header: string
  auth_prefix: string
}

export function getSupportedProviders() {
  return invoke<ProviderInfo[]>('get_supported_providers')
}

export function testLlmConnection(
  apiKey: string,
  model: string,
  provider: string,
  baseUrl: string,
) {
  return invoke<string>('test_llm_connection', { apiKey, model, provider, baseUrl })
}

// ── Capabilities ──

export interface Capabilities {
  model: string
  vision: string
  stt: string
  tts: string
  chat_agent_max_iterations: number | null
}

export function getCapabilities() {
  return invoke<Capabilities>('get_capabilities')
}

export function setCapability(key: string, value: string) {
  return invoke('set_capability', { key, value })
}

export function getContextLimit() {
  return invoke<number>('get_context_limit')
}

// ── Language ──

export function getLanguage() {
  return invoke<string>('get_language')
}

export function setLanguage(lang: string) {
  return invoke<string>('set_language', { lang })
}

// ── Project ──

export function setProjectDir(path: string) {
  return invoke<string>('set_project_dir', { path })
}

export function setProjectBookmarks(bookmarks: { name: string; path: string }[]) {
  return invoke('set_project_bookmarks', { bookmarks })
}

// ── Session Refine ──

export function executeSessionRefine() {
  return invoke<string>('execute_session_refine')
}

/** 桌面端跳过提炼：通知后端广播 RefineSkipped，双端同步关闭弹窗 */
export function refineSkip() {
  return invoke<string>('refine_skip')
}

export interface SessionRefineConfig {
  threshold: number
}

export function getSessionRefineConfig() {
  return invoke<SessionRefineConfig>('get_session_refine_config')
}

export function setSessionRefineConfig(threshold?: number) {
  return invoke<string>('set_session_refine_config', { threshold })
}

// ── User Input ──

export function submitUserInput(actionId: string, value: string) {
  return invoke<string>('submit_user_input', { actionId, value })
}

export function rejectUserInput(actionId: string) {
  return invoke<string>('reject_user_input', { actionId })
}

// ── Security ──

export function approveOnceSecurity(actionId: string) {
  return invoke<string>('approve_once_security', { actionId })
}

export function approveSessionSecurity(actionId: string, tool: string) {
  return invoke<string>('approve_session_security', { actionId, tool })
}

export function rejectSecurity(actionId: string) {
  return invoke<string>('reject_security', { actionId })
}

// ── Execution Rating ──

export function submitExecutionRating(
  goal: string,
  rating: number,
  comment: string,
  steps: {
    tool: string
    params?: unknown
    result?: string
    durationMs: number
    success: boolean
  }[],
  sessionId: string,
) {
  return invoke<string>('submit_execution_rating', {
    goal,
    rating,
    comment,
    tools_summary: steps.map(s => s.tool).join(', '),
    steps_json: JSON.stringify(steps),
    session_id: sessionId,
  })
}

// ── Workflow ──

/** 递归规范化 WorkflowStep (V2 do/Action 格式) */
function normalizeStep(raw: Record<string, unknown>): WorkflowStep {
  const action = (raw.do ?? {}) as Action
  const norm = (s: unknown) => normalizeStep(s as Record<string, unknown>)

  // 递归规范化嵌套步骤（判别式 + 精确变体断言，替代 as any）
  if ('seq' in action && Array.isArray(action.seq)) {
    action.seq = action.seq.map(norm)
  }
  if ('loop' in action) {
    const def = (action as { loop: LoopDef }).loop
    if (def && Array.isArray(def.do)) {
      def.do = def.do.map(norm)
    }
  }
  if ('if' in action) {
    const def = (action as { if: IfDef }).if
    if (def) {
      if (Array.isArray(def.then)) {
        def.then = def.then.map(norm)
      }
      if (Array.isArray(def.else)) {
        def.else = def.else.map(norm)
      }
    }
  }
  // Wait 变体：嵌套步骤在 action.auto（Rust: Wait { wait: String, auto: Vec<Step> }）。
  // 旧代码误读 action.wait.auto（wait 是 string，恒 undefined）——wait 子步骤从未被规范化。
  if ('wait' in action) {
    const auto = (action as { wait: string; auto?: WorkflowStep[] }).auto
    if (Array.isArray(auto)) {
      ;(action as { wait: string; auto?: WorkflowStep[] }).auto = auto.map(norm)
    }
  }

  return {
    id: String(raw.id ?? ''),
    name: String(raw.name ?? ''),
    description: typeof raw.description === 'string' ? raw.description : '',
    on_error: raw.on_error as OnError | undefined,
    capture: raw.capture as string | undefined,
    timeout_secs: typeof raw.timeout_secs === 'number' ? raw.timeout_secs : undefined,
    do: (Object.keys(action).length ? action : { tool: '', with: {} }) as Action,
  }
}

/** Backend Workflow → frontend WorkflowItem (字段名/格式/状态枚举转换) */
function normalizeWorkflow(raw: Record<string, unknown>): WorkflowItem {
  const statusMap: Record<string, 'draft' | 'active' | 'archived'> = {
    Draft: 'draft',
    Ready: 'active',
    Running: 'active',
    Completed: 'archived',
    Error: 'archived',
  }
  const toTs = (v: unknown): number =>
    typeof v === 'string' ? new Date(v).getTime() / 1000 : Number(v) || 0

  const steps = Array.isArray(raw.steps)
    ? (raw.steps as unknown[]).map((s: unknown) => normalizeStep(s as Record<string, unknown>))
    : []
  const runHistory = Array.isArray(raw.run_history) ? (raw.run_history as unknown[]) : []
  const rawTags = Array.isArray(raw.tags) ? (raw.tags as unknown[]) : []
  const tags = rawTags.filter((x): x is string => typeof x === 'string')
  const rawDoc = typeof raw.doc === 'string' ? raw.doc : undefined
  const scheduleRaw = raw.schedule
  const schedule =
    scheduleRaw !== null && typeof scheduleRaw === 'object' ? (scheduleRaw as ScheduleConfig) : null
  const rawTimeout = raw.timeout_secs
  const timeout_secs = typeof rawTimeout === 'number' ? rawTimeout : null

  return {
    id: String(raw.id ?? ''),
    title: String(raw.name ?? ''),
    description: String(rawDoc ?? raw.description ?? ''),
    steps,
    tags,
    created_at: toTs(raw.created_at),
    updated_at: toTs(raw.updated_at),
    run_count: runHistory.length,
    status: statusMap[String(raw.status)] || 'draft',
    schedule,
    run_history: runHistory as RunRecord[],
    timeout_secs,
    dry_run: Boolean(raw.dry_run),
    doc: rawDoc ?? null,
  }
}

export async function listWorkflows() {
  const resp = await invoke<Record<string, unknown>>('wf_list')
  const rawList = (resp?.workflows || []) as Record<string, unknown>[]
  return rawList.map(normalizeWorkflow)
}

export function wfDelete(id: string) {
  return invoke<void>('wf_delete', { id })
}

export function wfStop(id: string) {
  return invoke<string>('wf_stop', { id })
}

export function wfPause(id: string) {
  return invoke<void>('wf_pause', { id })
}

export function wfResume(id: string) {
  return invoke<void>('wf_resume', { id })
}

// ── 画布命令（IR 唯一真源：校验 / 保存 / 运行 / 布局 sidecar）──

/** 画布工具选择器数据源：后端权威过滤（WORKFLOW_TOOL_EXCLUDE），仅含 step 可执行工具 */
export function wfTools() {
  return invoke<ToolSchema[]>('wf_tools')
}

export interface ValidationReport {
  passed: boolean
  warnings: string[]
  errors: string[]
}

export interface WfSaveResponse {
  saved: boolean
  report: ValidationReport
}

/** 后端权威校验（L3）：工具注册表 + call 循环链等环境依赖规则 */
export function wfValidate(workflow: unknown) {
  return invoke<ValidationReport>('wf_validate', { workflow })
}

/** 画布唯一写回路径：保存前后端强制 validate，errors 阻断（saved=false + report） */
export function wfSave(workflow: unknown) {
  return invoke<WfSaveResponse>('wf_save', { workflow })
}

/** 画布确定性触发执行（同 id 自动断点续连）；进度经 workflow-event 推送 */
export function wfRun(id: string) {
  return invoke<string>('wf_run', { id })
}

/** 读取画布布局 sidecar（缺失/损坏返回 null → 全量自动布局） */
export function wfLayoutGet(id: string) {
  return invoke<Record<string, unknown> | null>('wf_layout_get', { id })
}

/** 写入画布布局 sidecar（位置元数据，不污染 IR） */
export function wfLayoutSave(id: string, layout: unknown) {
  return invoke<void>('wf_layout_save', { id, layout })
}

/** 画布取原始 IR（不经过 WorkflowItem 归一化，保证编辑写回无损） */
export async function wfGetRaw(id: string): Promise<Record<string, unknown> | null> {
  const resp = await invoke<Record<string, unknown>>('wf_list')
  const rawList = (resp?.workflows || []) as Record<string, unknown>[]
  return rawList.find(w => String(w.id ?? '') === id) ?? null
}

// ── Chat Agent ──

export function listChatAgents() {
  return invoke<ChatAgentConfig[]>('chat_agent_list')
}

export function saveChatAgent(config: ChatAgentConfig) {
  return invoke<ChatAgentConfig>('chat_agent_save', { config })
}

export function setActiveChatAgent(name: string) {
  return invoke<ChatAgentConfig>('chat_agent_set_active', { name })
}

export function getActiveChatAgent() {
  return invoke<ChatAgentConfig | null>('chat_agent_get_active')
}

export function deleteChatAgent(name: string) {
  return invoke<void>('chat_agent_delete', { name })
}

/** 查询某 workflow 中内联 ChatAgent 步骤配置 */
export function listChatAgentsInline(workflowId: string) {
  return invoke<InlineChatAgentEntry[]>('chat_agent_list_inline', { workflow_id: workflowId })
}

/** 更新工作流中内联 ChatAgent 配置 */
export function updateChatAgentInline(
  workflowId: string,
  stepId: string,
  config: Record<string, unknown>,
) {
  return invoke<void>('chat_agent_update_inline', {
    workflow_id: workflowId,
    step_id: stepId,
    config,
  })
}
// ── HUD ──

export function hudUpdate(text: string, phase: string) {
  return invoke('hud_update', { text, phase })
}
// ── Speech-to-text (cloud-first: capabilities.stt → /audio/transcriptions;
//    local sherpa-onnx fallback) ──

export interface SttStatus {
  available: boolean
  /** "no_microphone" | "model_missing: ..." when unavailable */
  reason: string | null
  /** "idle" | "recording" | "decoding" */
  phase: string
  model_dir: string | null
  version: string | null
  /** Engine serving the next session: "cloud" | "local"（云端优先路由） */
  engine: string
  /** capabilities.stt 已解析到云端 provider+模型（本地模型缺失时语音输入仍可用） */
  cloud_configured: boolean
}

export interface SttFinalPayload {
  text: string
  start_ms: number
  end_ms: number
}

export function sttStatus() {
  return invoke<SttStatus>('stt_status')
}

export function sttStart() {
  return invoke<void>('stt_start')
}

export function sttStop() {
  return invoke<void>('stt_stop')
}

export function sttCancel() {
  return invoke<void>('stt_cancel')
}

/**
 * stt:download 事件 payload（与 src-tauri/src/speech/download.rs 一致）。
 * progress 每 ~1MiB 节流一次（文件完成时必发）；done/error 为终态，各恰好一次。
 */
export type SttDownloadPayload =
  | {
      kind: 'progress'
      file: string
      downloaded: number
      /** 0 = 服务端未给 Content-Length */
      total: number
      /** 当前文件序号（1-based） */
      index: number
      count: number
    }
  | { kind: 'done' }
  | { kind: 'error'; message: string }

/**
 * 启动 STT 模型后台下载。立即返回；Err("stt_download_busy") 表示已有下载在跑
 * （事件全局广播，可直接跟随其进度）。进度/终态经 stt:download 事件推送。
 */
export function sttDownloadModel() {
  return invoke<void>('stt_download_model')
}
// ── 本地视觉模型（PaddleOCR + YOLO icon_detect）自动下载 ──
// 产品原则：除 STT 外的本地模型全自动获取。运行时首启缺文件时 bootstrap 后台
// 补齐；此命令只读状态，ModelsPage 用它渲染「缺模型 / 下载中 / 已就绪」。

/** vision_models_status 命令返回（camelCase，与 bootstrap.rs serde 一致） */
export interface VisionModelsStatus {
  ocrReady: boolean
  yoloReady: boolean
  /** 低于防投毒下限（缺失或过小）的文件名列表 */
  missing: string[]
  /** 下载落盘目录（data_dir 不可解析时为 null） */
  dir: string | null
  downloading: boolean
}

/**
 * models:download 事件 payload（与 src-tauri/src/models/bootstrap.rs 一致）。
 * progress 每 ~1MiB 节流一次；done/error 为终态，各恰好一次。
 */
export type ModelsDownloadPayload =
  | {
      kind: 'progress'
      file: string
      downloaded: number
      /** 0 = 服务端未给 Content-Length */
      total: number
      index: number
      count: number
    }
  | { kind: 'done'; ocr_ready: boolean; yolo_ready: boolean }
  | { kind: 'error'; message: string }

export function visionModelsStatus() {
  return invoke<VisionModelsStatus>('vision_models_status')
}

/**
 * 触发 / 重试本地视觉模型下载（后台线程，立即返回；非阻塞，不拖慢启动）。
 * 进度 / 终态经 models:download 事件推送；ModelsPage「重试」按钮复用。
 */
export function retryVisionDownload() {
  return invoke<boolean>('preload_ocr')
}
// ── 移动端局域网 server（mobile_server.rs，P3 设置面板） ──

export interface MobileServerStatus {
  running: boolean
  port: number
  token: string
  lan_url: string | null
  /** 是否已设置配对密码（配对凭证：密码换 token） */
  password_set: boolean
}

export function mobileServerStart(port?: number) {
  return invoke<MobileServerStatus>('mobile_server_start', { port: port ?? null })
}

/** 确保 server 运行但不持久化 enabled（插件宿主专用，不改变用户移动端开关设置） */
export function mobileServerEnsure() {
  return invoke<MobileServerStatus>('mobile_server_ensure')
}

export function mobileServerStop() {
  return invoke<void>('mobile_server_stop')
}

export function mobileServerStatus() {
  return invoke<MobileServerStatus>('mobile_server_status')
}

export function mobileTokenRegenerate() {
  return invoke<string>('mobile_token_regenerate')
}

/** 设置/修改配对密码（≥6 位含字母数字；成功自动重签 token，已配对手机需重新输密码） */
export function mobilePasswordSet(password: string) {
  return invoke<void>('mobile_password_set', { password })
}

// ── 中继连接状态（relay_client.rs 统一状态机，设置页只读展示） ──

export interface RelayChannelState {
  status: 'connected' | 'retrying' | 'fault' | 'disabled'
  /** retrying: 本轮连续失败起始时间（unix 秒） */
  since?: number
  /** retrying: 连续失败次数 */
  attempts?: number
  /** fault: 故障原因摘要 */
  reason?: string
}

export interface RelayClientStatus {
  enabled: boolean
  state: { relay: RelayChannelState; tunnel: RelayChannelState }
  /** 隧道公网入口（http://host:18081），中继未启用时为 null；远程配对链接的 base */
  public_url?: string | null
}

export function relayClientStatus() {
  return invoke<RelayClientStatus>('relay_client_status')
}

/** 中继开关：持久化 enabled + 运行时即时启停（免重启） */
export function relayClientSetEnabled(enabled: boolean) {
  return invoke<string>('relay_client_set_enabled', { enabled })
}

/** 轮换中继调用凭据（caller_token）：服务端热生效，旧凭据即刻失效，已配对手机外网访问需重新扫码 */
export function relayCallerTokenRotate() {
  return invoke<string>('relay_caller_token_rotate')
}
// ── Relation（身份关系） ──

/** 持久化身份配置到后端 relation.json 并更新 relation_cache（手机端 /identity 依赖此缓存） */
export async function setRelation(relation: RelationConfig): Promise<void> {
  await invoke('set_relation', { relation })
}

// ── 文件预览（preview.rs，AI 回复路径点击） ──

/** 读取文件文本内容（后端限制 ≤2MB），供预览覆盖层渲染 */
export function readFile(path: string) {
  return invoke<string>('read_file', { path })
}

/** 读取文件为 base64（后端限制 ≤8MB），供预览覆盖层内联渲染图片 */
export function readFileBase64(path: string) {
  return invoke<string>('read_file_base64', { path })
}

/** 系统默认程序打开文件/文件夹 */
export function openPath(path: string) {
  return invoke<void>('open_path', { path })
}

/** 文件管理器定位（Windows explorer /select,） */
export function revealPath(path: string) {
  return invoke<void>('reveal_path', { path })
}

// ── MCP 管理（只读） ──

export interface McpServerInfo {
  key: string
  command: string
  args: string[]
  timeout_ms: number
  auto_start: boolean
}

export interface McpToolInfo {
  name: string
  description?: string
}

/** 列出所有已配置的 MCP server（不含 env 敏感字段） */
export function listMcpServers() {
  return invoke<{ servers: McpServerInfo[] }>('list_mcp_servers')
}

/** 查询某 MCP server 的工具列表（tools/list 原始响应，解析 .tools 数组） */
export function listMcpTools(server: string) {
  return invoke<{ tools?: McpToolInfo[] }>('list_mcp_tools', { server })
}