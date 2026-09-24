// Nuphus API — typed wrappers for all backend Tauri commands
import { invoke } from '../../core/bridge'
import { showAppFeedbackByHudPhase } from '../../ui/islandChannel'
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
  WorkflowInputKind,
  WorkflowInputSpec,
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

/** 手动设置某 provider 下某模型的 context_window（ModelsPage 模型行内编辑）。
 *  持久化到 providers.toml（真实用户意图，非探测猜测）；若为当前激活模型，
 *  后端同步更新运行时窗口 —— refine 阈值与上下文占用百分比立即按新值计算。 */
export function setModelContextWindow(provider: string, model: string, contextWindow: number) {
  return invoke<string>('set_model_context_window', { provider, model, contextWindow })
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
  kind?: 'progress'
  message_id?: string
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

// ── Jev enhanced workflow mode ──

/** Jev 使用独立配置，不属于聊天模型 Provider。后端永不返回 API Key 原文。 */
export interface JevConfig {
  enabled: boolean
  base_url: string
  model: string
  has_key: boolean
  timeout_ms: number
  max_retries: number
  fallback_to_primary_model: boolean
}

export interface SaveJevConfigInput {
  apiKey?: string
  baseUrl: string
  model: string
  timeoutMs: number
  maxRetries: number
  fallbackToPrimaryModel: boolean
}

/**
 * Laya 配置：Jev 之外的自托管决策后端选项。
 *
 * Laya（github.com/NandhaKishorM/laya）是独立模型，只是恰好兼容 Jev 的
 * /v1/systemone 协议。它与 Jev 各自独立配置，同一时刻只有一个生效
 * （见后端 ModelRegistry::decision_backend：Jev 已配 Key 时优先）。
 *
 * 与 Jev 的关键差异：自托管 Laya 通常不需要 API Key（仅当服务端设置了
 * LAYA_API_KEY 时才要求）。
 */
export interface LayaConfig {
  enabled: boolean
  base_url: string
  model: string
  has_key: boolean
  timeout_ms: number
  max_retries: number
  fallback_to_primary_model: boolean
}

export interface SaveLayaConfigInput {
  apiKey?: string
  baseUrl: string
  model: string
  enabled: boolean
  timeoutMs: number
  maxRetries: number
  fallbackToPrimaryModel: boolean
}

export interface LayaConnectionStatus {
  status: string
  message?: string
  model?: string
}

export interface JevConnectionStatus {
  status: string
  message?: string
  model?: string
}

export interface WorkflowEnhancedMode {
  /** Session preference; may remain enabled while desktop permissions are missing. */
  enabled: boolean
  configured: boolean
  /** Includes unsupported_platform and needs_accessibility; neither is ready. */
  status?: string
}

export function getJevConfig() {
  return invoke<JevConfig>('get_jev_config')
}

export function saveJevConfig(input: SaveJevConfigInput) {
  return invoke<JevConfig>('save_jev_config', { ...input })
}

export function clearJevApiKey() {
  return invoke<void>('clear_jev_api_key')
}

export function getLayaConfig() {
  return invoke<LayaConfig>('get_laya_config')
}

export function saveLayaConfig(input: SaveLayaConfigInput) {
  return invoke<LayaConfig>('save_laya_config', { ...input })
}

export function clearLayaApiKey() {
  return invoke<void>('clear_laya_api_key')
}

export function testLayaConnection() {
  return invoke<LayaConnectionStatus>('test_laya_connection')
}

export function testJevConnection() {
  return invoke<JevConnectionStatus | string>('test_jev_connection')
}

export function getWorkflowEnhancedMode() {
  return invoke<WorkflowEnhancedMode>('get_workflow_enhanced_mode')
}

export function setWorkflowEnhancedMode(enabled: boolean) {
  return invoke<WorkflowEnhancedMode>('set_workflow_enhanced_mode', { enabled })
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

export function getMemoryOverview() {
  return invoke<import('../../core/types-memory').MemoryOverview>('get_memory_overview')
}

// ── Desktop ──

// ── Hooks ──

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
  newSession?: boolean,
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
    new_session: newSession ?? false,
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

/**
 * 后端唯一权威执行态（前端执行态单一来源）。
 *
 * 前端所有「执行中」相关 UI（气泡光标 / 思考条 / mode 锁 / 会话 rail 锁 / 终止按钮）
 * 都从这里派生，禁止再各自订阅 is_busy / can_switch 或自行 OR 派生（曾因此出现
 * 「主循环已退出、界面仍以为在执行」的不一致）。
 */
export interface ExecutionStateSnapshot {
  /** "idle" | "running" | "finalizing"（见 nuphus::state::ExecutionStage） */
  stage: string
  /** stage !== "idle"（= 旧 is_busy 语义） */
  busy: boolean
  /** 当前提交是否会按追加指令受理（仅 running；finalizing 会被拒收） */
  append_accepting: boolean
}

export function getExecutionState() {
  return invoke<ExecutionStateSnapshot>('get_execution_state')
}

export function getAppendQueue() {
  return invoke<string[]>('get_append_queue')
}

export function removeAppendQueueItem(index: number) {
  return invoke<string[]>('remove_append_queue_item', { index })
}

export function forceReset() {
  return invoke<string>('force_reset')
}

// ── 后端资源互斥门（见 nuphus::automation_gate）──

/**
 * 稳定错误码：桌面自动化 / 浏览器控制 / 录制 / 主执行体互斥被拒。
 * 后端 `LeaseBusy` 的 Display 固定为 `automation_busy: <人话>`（Tauri 的 Err 只有
 * 字符串通道，码前缀是前端识别的唯一锚点，前后端契约不可改名）。
 */
export const AUTOMATION_BUSY = 'automation_busy'

function errorText(err: unknown): string {
  if (typeof err === 'string') return err
  if (err instanceof Error) return err.message
  return String(err ?? '')
}

/** 该错误是否来自资源互斥门（执行体/录制占用中） */
export function isAutomationBusy(err: unknown): boolean {
  return errorText(err).startsWith(AUTOMATION_BUSY)
}

/** 后端错误 → 可直接展示的文案（剥掉稳定码前缀，其余原样返回） */
export function backendErrorMessage(err: unknown): string {
  const text = errorText(err)
  if (!text.startsWith(AUTOMATION_BUSY)) return text
  const rest = text.slice(AUTOMATION_BUSY.length).replace(/^:\s*/, '').trim()
  return rest || text
}

export function setMode(mode: string) {
  return invoke<string>('set_mode', { mode })
}

/** 获取当前权威 mode（启动时同步镜像恢复结果） */
export function getCurrentMode() {
  return invoke<string>('get_current_mode')
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

/**
 * 删除某外部 agent 的一个交付物文件。
 * relPath 为相对该 agent handoff 目录的路径（briefs/… 或 projects/…），
 * 后端二次校验范围与 list 严格一致，越界/核心文件一律拒绝。
 */
export function deleteAgentDeliverable(agent: string, relPath: string) {
  return invoke<void>('delete_agent_deliverable', { agent, relPath })
}

/** 外部 Agent 被移出列表栏 → 后端写一句提示，下一轮带进 agent 上下文
 *  （只进上下文、界面不显示）。让 agent 知道后续需用户显式指定才可调用。 */
export function notifyExtAgentRemoved(agent: string) {
  return invoke<void>('notify_ext_agent_removed', { agent })
}

// ── Session Shelf（浅层会话展示台）──

export interface ShelfSessionItem {
  id: string
  mode: string
  title: string
  /** hover 预览：agent 最终回复脱敏截断（≤400 字符），与标题「话题 ↔ 结果」互补 */
  preview?: string
  message_count: number
  updated_at: number
  is_active: boolean
  /**
   * 会话归属目录（诞生时快照，Phase 1 新增）；`null` = 无归属（历史遗留/未配置项目目录）。
   * ⚠️ 无归属会话**不得**用当前工作目录回填——前端只据此归入「未分组」。
   */
  project_path?: string | null
  /**
   * 草稿对话标记（后端 `draft: true`）：新建项目文件夹后立刻出现、尚未开说的空对话。
   * 它**不在** SQLite / session_meta / mirror / snapshot 任何一处（纯内存态），
   * 切换会话或退出进程即消失。前端据此渲染「新建对话」标题，并隐藏重命名/归档
   * （重命名会写 sessions 行，违反「草稿不落库」）。可选：老后端/mock 缺失即按普通会话处理。
   */
  draft?: boolean
  /**
   * 会话创建时刻（Unix 毫秒）——「按时间顺序 → 创建时间」组内排序的**唯一**依据。
   * 来源 sessions.created_at（尚无落盘行的 active 会话取首条消息时间戳）。
   * 可选：老后端 / mock 缺失时前端退化为 updated_at（见 sessionGroups.createdMillis）。
   */
  created_at?: number
}

/**
 * 会话工作台排序偏好（后端 `sort_prefs` 原样透传）。
 * 字段是**线上取值**（后端已归一，非法值会在前端二次归一为默认）。
 */
export interface ShelfSortPrefs {
  /** `bookmark` = 按项目（书签顺序，默认）/ `recent` = 近期项目（组内最近会话倒序） */
  group_order: string
  /** `updated` = 更新时间（默认）/ `created` = 创建时间 */
  sort_key: string
}

/** 项目文件夹（会话工作台分组数据源，对齐后端 ProjectEntry） */
export interface ShelfProjectEntry {
  /** 分组键：与 items[].project_path 精确对应 */
  path: string
  /** 展示名：书签自定义名优先，自动组取目录末段 */
  name: string
  /** 当前工作目录所在组（不上浮；**不驱动组头视觉**——组头不渲染「当前」/ 不着色，
   *  组内会话点击时跳过重复 `set_project_dir` 的幂等判据是它唯一的前端消费点） */
  is_current: boolean
  /** true = 未收藏但有会话的自动组（只读：不可重命名/归档） */
  auto: boolean
}

export interface ShelfListResponse {
  /** false = busy 或追加队列非空，切换被后端拒绝 */
  can_switch: boolean
  items: ShelfSessionItem[]
  /** 可见项目文件夹（书签顺序 → auto），顺序即组顺序 */
  projects: ShelfProjectEntry[]
  /** 已归档项目文件夹（整组隐藏；「已归档文件夹」恢复入口的数据源） */
  archived_projects: ShelfProjectEntry[]
  /** 全局组内折叠上限（后端已把 0 收敛为默认值） */
  collapsed_limit: number
  /** 排序偏好（组序维度 + 组内键）：桌面与移动端共用同一读数 */
  sort_prefs: ShelfSortPrefs
}

/** 展示台列表：active 置顶 + newest-first */
export function listShelfSessions() {
  return invoke<ShelfListResponse>('list_shelf_sessions')
}

/** 切换会话；mode 为可选目标 mode——跨 mode 切换由后端原子完成
 * （归档原槽 → 切 current_mode → 安装目标槽），避免前端先 set_mode 再
 * switch_session 两次 IPC 的竞态。失败 reject 稳定错误码字符串 */
export function switchSession(id: string, mode?: string) {
  return invoke<void>('switch_session', { id, mode: mode ?? null })
}

/** 新建对话：归档当前 → 当前 mode 槽置 None → **回到无会话的欢迎页**，同时记录
 * 弹窗填写的 `title`（只记录，不创建会话）——真实会话在欢迎页直发首条消息那一刻
 * 诞生，后端在诞生点把记录的标题写成该会话的标题（rail + sessions.summary）。
 * 返回值是 SessionChanged 事件 token，**不是会话 id**（此刻还没有会话）。
 * 无标题调用（Ctrl+N 等）同时清掉上一次的残留记录。失败 reject 稳定错误码。 */
export function newChatSessionCmd(title?: string) {
  return invoke<string>('new_chat_session_cmd', { title: title ?? null })
}

/**
 * 「新建项目文件夹」第 ③ 步：在当前项目目录下立刻生成一条**草稿对话**并成为当前对话。
 *
 * 调用方（创建项目弹窗）已按序完成 ① 写书签（`setProjectBookmarks`）② 切当前目录
 * （`setProjectDir`）——本命令只做归属快照 + 内存态登记，**不落库**：
 * 后端无 sessions 行 / session_meta / mirror / snapshot，因此用户未发消息就切换会话会
 * 消失、退出进程不留痕迹；发出首条消息时真实会话在诞生点登记归属（= 该文件夹），
 * 标题走既有派生规则。失败 reject 稳定错误码（busy / append_pending / no_project_dir）。
 */
export function createProjectChat() {
  return invoke<{ id: string; mode: string; project_path: string }>('create_project_chat')
}

/** 重命名会话（落 sessions.summary 元数据行） */
export function renameSession(id: string, title: string) {
  return invoke<void>('rename_session_cmd', { id, title })
}

/** 手动归档：移出展示台并清快照（对话记录保留在记忆页）；失败 reject 稳定错误码 */
export function archiveSession(id: string) {
  return invoke<void>('archive_session', { id })
}

/** 是否存在可恢复的最近会话镜像（欢迎页「继续对话」按钮显示条件） */
export function hasResumeCandidate() {
  return invoke<boolean>('has_resume_candidate')
}

/** 继续对话：最新镜像写入 session_backup，返回完整历史（下条消息即续聊） */
export function resumeLatestSession() {
  return invoke<HistoryMessage[]>('resume_latest_session')
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
  /** Agent 工作目录（用户个性化配置；Leader 查找/定位用） */
  dir?: string
  // ── v8 交互固化字段（终端型推荐配置；agent_dispatch 使用）──
  launch?: string
  window_hint?: string
  cooldown_secs?: number
  dispatch_steps?: Array<{ tool: string; with?: Record<string, unknown> }>
  await_timeout_secs?: number
  timeout_action?: string
  timeout_script?: string
  auto_approve?: string
  auto_approve_script?: string
  confirm_keywords?: string[]
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

export const TOOL_PERMISSIONS_CHANGED_EVENT = 'nuphus:tool-permissions-changed'

export function setToolPermissions(
  file_access: boolean,
  web_search: boolean,
  system_automation: boolean,
) {
  const permissions: ToolPermissions = { file_access, web_search, system_automation }
  return invoke<string>('set_tool_permissions', {
    fileAccess: file_access,
    webSearch: web_search,
    systemAutomation: system_automation,
  }).then(result => {
    window.dispatchEvent(
      new CustomEvent<ToolPermissions>(TOOL_PERMISSIONS_CHANGED_EVENT, { detail: permissions }),
    )
    return result
  })
}

export function getToolPermissions() {
  return invoke<string>('get_tool_permissions')
}

export type MacosPermissionId =
  'screen_recording' | 'accessibility' | 'microphone' | 'files_and_folders' | 'automation'

export interface MacosPermissionItem {
  id: MacosPermissionId
  status: 'granted' | 'missing' | 'on_demand' | 'unsupported'
  requiredForWorkflow: boolean
  title: string
  description: string
  settingsUrl?: string | null
}

export interface MacosPermissionReport {
  platformSupported: boolean
  permissions: MacosPermissionItem[]
}

export function getMacosPermissionStatus() {
  return invoke<MacosPermissionReport>('get_macos_permission_status')
}

export function requestMacosPermission(id: MacosPermissionId) {
  return invoke<MacosPermissionReport>('request_macos_permission', { id })
}

export function openMacosPermissionSettings(id: MacosPermissionId) {
  return invoke<void>('open_macos_permission_settings', { id })
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
  supports_image_generation: boolean
  /** 上下文窗口（tokens）；undefined = 未知 */
  context_window?: number
  /** 推理强度选项（如 ["low","high","max"]）；空 = 无此旋钮 */
  reasoning_efforts: string[]
  /** 模型默认推理强度（未配置时生效；null = 无声明，UI 显示「默认」） */
  default_effort?: string | null
  /** 成本（USD / 百万输入 tokens）；undefined = 未知 */
  cost_per_million_in?: number
  /** 成本（USD / 百万输出 tokens）；undefined = 未知 */
  cost_per_million_out?: number
  /** 条目来源：auto = 官方 /v1/models 同步写入；manual = 用户手动添加，刷新不移除 */
  source?: 'auto' | 'manual'
}

export function listModels() {
  return invoke<ModelInfo[]>('list_models')
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
  leader: string
  leader_provider: string
  workflow: string
  workflow_provider: string
  exec: string
  exec_provider: string
  custom: string
  custom_provider: string
}

/** 读取高级设置：各 agent 的模型（空串 = 跟随 global / 系统默认） */
export function getAgentModels() {
  return invoke<AgentModels>('get_agent_models')
}

/** 计算某 mode 的生效模型（输入框显示用） */
export function getEffectiveModel(mode: string) {
  return invoke<string>('get_effective_model', { mode })
}

/**
 * 设置某个 agent 的模型；model 空串 = 清除（跟随全局 fallback）。
 * provider 必须与被选 model 同源（ModelInfo.provider）：后端按 (provider, model)
 * 成对落盘，只给 model 时由后端消歧（唯一候选自动补全 / 多候选报错）。
 */
export function setAgentModel(agent: string, model: string, provider?: string) {
  return invoke<string>('set_agent_model', { agent, model, provider })
}

/** 生效模型的 provider 归属（mode 感知，后端权威解析）：弹窗勾选/effort 上下文数据源 */
export interface ProviderContext {
  model: string
  provider: string
}

/**
 * 某 mode 生效模型 + 其 provider 归属。同 id 跨 provider（官方 deepseek vs
 * opencode-go）时，get_current_config 的 provider 是 runtime 全局模型（非 mode
 * 感知），勾选/effort 须以本命令为准，否则会串卡。
 */
export function getProviderContext(mode: string) {
  return invoke<ProviderContext | null>('get_provider_context', { mode })
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

/** 服务商 /v1/models 返回的单个模型（id + 能力元数据，用于模型列表能力徽标） */
export interface ProviderModelBrief {
  id: string
  supports_streaming: boolean
  supports_vision: boolean
  supports_audio: boolean
  supports_image_generation: boolean
  /** 上下文窗口（tokens）；undefined = 未知 */
  context_window?: number
}

/** List all available models for a provider via /v1/models (validates API key) */
export function listProviderModels(apiKey: string, provider: string, baseUrl?: string) {
  return invoke<ProviderModelBrief[]>('list_provider_models', { apiKey, provider, baseUrl })
}

/** 刷新同步摘要：新增 / 更新 / 移除，以及被更新与被移除的 id（供用户知情与重加） */
export interface SyncReport {
  added: number
  updated: number
  /** 能力被覆写的模型代号（磁盘顺序） */
  updated_ids: string[]
  removed: number
  removed_ids: string[]
  /** 官方清单外但按 source=manual 保留下来的条目数 */
  kept_manual: number
}

/** 刷新结果：当次拉取的模型 brief（列表显示）+ 落盘同步摘要 */
export interface RefreshModelsResult {
  models: ProviderModelBrief[]
  report: SyncReport
}

/**
 * 刷新服务商最新模型列表：用 config.toml 已存 API key（不暴露 key）。
 * sync=true（用户点「刷新」）→ 该 provider 段同步为官方 /v1/models 集合（移除清单外
 * auto 条目 + 覆写能力）；sync 省略/false（进入页面的静默自动同步）→ 只新增 + 覆写，
 * 绝不删除任何条目。
 */
export function refreshProviderModels(provider: string, baseUrl?: string, sync?: boolean) {
  return invoke<RefreshModelsResult>('refresh_provider_models', { provider, baseUrl, sync })
}

/** 读取某服务商已保存的接口地址（界面回填用）；未配置返回 null */
export function getProviderBaseUrl(provider: string) {
  return invoke<string | null>('get_provider_base_url', { provider })
}

/** 手动添加单模型到服务商配置（config.toml models 列表）——灰度/临时模型（/v1/models 未返回）使用 */
export function addProviderModel(provider: string, modelId: string) {
  return invoke<void>('add_provider_model', { provider, modelId })
}

/** 清空服务商模型列表（接口地址变更后旧模型可能失效，前端提示条调用；返回清除条目数） */
export function clearProviderModels(provider: string) {
  return invoke<number>('clear_provider_models', { provider })
}

/**
 * OAuth 订阅登录态（**只含状态与配置，绝无令牌**）。
 *
 * `oauth_status` 返回状态四字段；`ProviderInfo.oauth`（后端 OauthSummaryDto）
 * 在此之上**附带配置五项**供编辑表单回显（snake_case，与后端字段一一对应）。
 */
export interface OauthStatusInfo {
  /** 该段是否配了 OAuth（授权端点 / 令牌端点 / Client ID 齐全） */
  configured: boolean
  logged_in: boolean
  /** access token 到期时刻（unix 秒）；null = 未知 */
  expires_at: number | null
  /** 需重新登录：配置完整但未登录，或令牌已过期且无 refresh_token 可自愈 */
  needs_login: boolean
  /** ── 以下五项仅 ProviderInfo.oauth 携带（oauth_status 不返回）── */
  authorize_url?: string
  token_url?: string
  client_id?: string
  scopes?: string
  use_pkce?: boolean
  redirect_port?: number | null
}

export interface ProviderInfo {
  id: string
  name: string
  /** 界面显示名（自定义实例 = 用户填写的名称；官方 provider 为空 → 回退 name） */
  display_name?: string
  /** Protocol type: custom（OpenAI 兼容）/ anthropic（Anthropic 兼容）/ 官方 id */
  provider_type: string
  base_url: string
  default_model: string
  auth_header: string
  auth_prefix: string
  /** 段级自定义请求头（编辑回显；官方 provider 恒为空对象） */
  extra_headers?: Record<string, string>
  /** OAuth 订阅登录摘要（仅状态，绝无令牌）；未配 oauth 的段与官方 provider 为 null */
  oauth?: OauthStatusInfo | null
}

export function getSupportedProviders() {
  return invoke<ProviderInfo[]>('get_supported_providers')
}

/**
 * create / update_custom_provider 的 OAuth 配置入参（五项，camelCase；与后端
 * `OauthConfigDto` 同形）。**不含令牌字段** —— 令牌只由登录 / 刷新路径维护。
 *
 * 传 null 的语义按命令区分：create = 该实例不启用 OAuth（静态 api_key 模式）；
 * update = **不动**已存 OAuth 配置（既有令牌保留）。
 */
export interface OauthConfigPayload {
  authorizeUrl: string
  tokenUrl: string
  clientId: string
  /** 空格分隔的 scope 列表；空串 = 授权请求不带 scope */
  scopes: string
  /** PKCE（RFC 7636）：缺省 true；仅在授权服务器不支持时才关 */
  usePkce: boolean
  /** 本地回调端口；null = 由系统分配空闲端口 */
  redirectPort: number | null
}

/**
 * 新建自定义模型实例（自定义中转站）—— custom 配置界面「创建」按钮的唯一落盘入口。
 *
 * `name` 是段 id（由名称 slug 化而来，见 lib/customProvider.buildCustomInstanceId），
 * `displayName` 是用户填写的名称（界面只显示它）。`apiKey` 可空（无鉴权端点）。
 * `headers` 是段级自定义请求头（二元组数组，空数组 = 不写标头；后端存在旧 `custom`
 * 段时自动执行升级迁移，key/models 归新段）。重名/非法输入由后端拒绝并返回可读错误。
 * `oauth`：Some → 该实例以 OAuth 令牌充当请求凭证；null → 静态 api_key 模式。
 */
export function createCustomProvider(
  name: string,
  displayName: string,
  providerType: string,
  baseUrl: string,
  apiKey: string,
  headers: Array<[string, string]>,
  oauth: OauthConfigPayload | null,
) {
  return invoke<ProviderInfo>('create_custom_provider', {
    name,
    displayName,
    providerType,
    baseUrl,
    apiKey,
    headers,
    oauth,
  })
}

/**
 * 更新既有自定义模型实例（重命名 / 换协议 / 改地址 / 换密钥）—— 编辑页四字段表单的落盘入口。
 *
 * `name` 是**已有段 id**，且**永不被修改**：同名模型靠「段 id + 模型 ID」精确路由，
 * 改段 id 会把请求打到别的中转站；重命名写的是 `display_name`。
 * `apiKey` 传空串 = 保持原密钥不变（编辑表单留空即「不修改密钥」，不是清空）。
 * `headers` 是段级自定义请求头全量提交：空数组 = 清除全部已存标头（后端契约）。
 * `oauth`：Some → 覆盖写 OAuth 配置五项（既有令牌保留）；null = 不动 OAuth 配置。
 */
export function updateCustomProvider(
  name: string,
  displayName: string,
  providerType: string,
  baseUrl: string,
  apiKey: string,
  headers: Array<[string, string]>,
  oauth: OauthConfigPayload | null,
) {
  return invoke<ProviderInfo>('update_custom_provider', {
    name,
    displayName,
    providerType,
    baseUrl,
    apiKey,
    headers,
    oauth,
  })
}

// ── OAuth 订阅登录（custom 实例的授权码 + 本地回调流程）──

/** `oauth_begin` 返回：授权 URL（前端经 openExternal 打开）与本地回调端口 */
export interface OauthBeginResult {
  authorize_url: string
  port: number
}

/**
 * 发起 OAuth 授权：后端绑定本地回调 server 并返回授权 URL。
 *
 * 结果**不从这里返回** —— 浏览器回调到达后由后端推送 `oauth-login-result`
 * 事件（见 ModelsPage 的监听），本命令只负责把用户送进授权页。
 * 段未配 oauth / 配置不完整 → reject 可读中文错误；重复调用会顶替旧会话。
 */
export function oauthBegin(provider: string) {
  return invoke<OauthBeginResult>('oauth_begin', { provider })
}

/** 查询该实例的登录态（读盘 + 纯判定，不触发网络刷新） */
export function oauthStatus(provider: string) {
  return invoke<OauthStatusInfo>('oauth_status', { provider })
}

/** 清空该实例的 OAuth 令牌（配置五项保留；下次使用需重新授权） */
export function oauthLogout(provider: string) {
  return invoke<void>('oauth_logout', { provider })
}

// ── Capabilities ──

export interface Capabilities {
  model: string
  vision: string
  vision_provider?: string
  stt: string
  stt_provider?: string
  tts: string
  tts_provider?: string
  voice: string
  voice_provider?: string
  image_generation?: string
  image_generation_provider?: string
  chat_agent_max_iterations: number | null
}

export function getCapabilities() {
  return invoke<Capabilities>('get_capabilities')
}

export function setCapability(key: string, value: string) {
  return invoke('set_capability', { key, value })
}

/** 原子设置能力模型绑定：model 与 provider 必须在同一次写入内落盘，
 *  否则会留下「新 model + 旧 provider」的半绑定（能力请求按 provider+model
 *  精确解析时找不到该组合，保存看似成功但实际用不了）。
 *  kind ∈ 'vision' | 'stt' | 'tts' | 'voice' | 'image_generation'。 */
export function setCapabilityBinding(kind: string, model: string, provider: string) {
  return invoke<void>('set_capability_binding', { kind, model, provider })
}

/** 手动设定某 provider 下某模型的视觉（多模态）能力（模型行内开关）。
 *  落盘后在 providers.toml 标记来源 user，自动探测不再覆盖该值。 */
export function setModelSupportsVision(provider: string, model: string, supportsVision: boolean) {
  return invoke<string>('set_model_supports_vision', { provider, model, supportsVision })
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

export type ProjectBookmark = {
  name: string
  path: string
  /**
   * 归档标记：true = 该文件夹在会话工作台隐藏（可从「已归档文件夹」恢复）。
   * 由 `set_project_folder_archived` 增删，整表替换（`set_project_bookmarks`）会
   * 自动沿用已落盘标记——归档状态不因重命名/增删书签而丢失。
   */
  archived?: boolean
}
/** 项目目录状态（后端 prefs 为唯一事实源） */
export type ProjectDirState = { path: string; name: string; tag: string }

/** 读取当前项目目录（bridge 的 invoke 返回可空 → 统一兜底为空态） */
export async function getProjectDir(): Promise<ProjectDirState> {
  const state = await invoke<ProjectDirState>('get_project_dir')
  return state ?? { path: '', name: '', tag: 'default' }
}

/** 设置/切换项目目录：返回新状态；后端会向活跃会话注入一条 user 内部消息告知变更 */
export async function setProjectDir(path: string): Promise<ProjectDirState> {
  const state = await invoke<ProjectDirState>('set_project_dir', { path })
  return state ?? { path, name: '', tag: 'default' }
}

/** 读取项目书签（项目中心唯一数据源） */
export async function getProjectBookmarks(): Promise<ProjectBookmark[]> {
  return (await invoke<ProjectBookmark[]>('get_project_bookmarks')) ?? []
}

/** 写入项目书签（整表替换；后端去空/去重/名称兜底，同路径自动沿用归档标记） */
export async function setProjectBookmarks(
  bookmarks: ProjectBookmark[],
): Promise<ProjectBookmark[]> {
  return (await invoke<ProjectBookmark[]>('set_project_bookmarks', { bookmarks })) ?? bookmarks
}

/** 归档 / 恢复项目文件夹（归档 = 会话工作台整组隐藏，可恢复）；返回最新书签表 */
export async function setProjectFolderArchived(
  path: string,
  archived: boolean,
): Promise<ProjectBookmark[]> {
  return (await invoke<ProjectBookmark[]>('set_project_folder_archived', { path, archived })) ?? []
}

// ── 会话工作台分组设置 ──

/** 组内折叠上限变更事件（设置中心写盘后广播，会话工作台立即生效而非等 5s 轮询） */
export const SESSION_GROUP_LIMIT_CHANGED_EVENT = 'nuphus:session-group-limit-changed'

/**
 * 设置会话分组折叠上限（全局单值）。
 * 后端拒绝 0（会让每个分组折叠成空列表）→ 返回稳定错误码 `invalid_limit`；
 * 调用方（设置中心）已在前端做 >=1 校验，此处不再兜底。
 */
export async function setSessionGroupCollapsedLimit(limit: number): Promise<number> {
  const applied = await invoke<number>('set_session_group_collapsed_limit', { limit })
  // bridge 的 invoke 在无 Tauri 通道（浏览器回退链路）时返回 null → 回落到请求值，
  // 不用 0 之类的假值污染调用方读数（后端失败会 throw，不走这里）
  const value = typeof applied === 'number' && applied >= 1 ? applied : limit
  window.dispatchEvent(
    new CustomEvent<number>(SESSION_GROUP_LIMIT_CHANGED_EVENT, { detail: value }),
  )
  return value
}

/**
 * 设置会话工作台排序偏好（组序维度 + 组内排序键，两维独立）。
 *
 * 后端把非法取值归一为默认值并**返回归一后的结果**，调用方据此校准本地状态
 * （避免「界面显示 A、落盘 B」）。生效无需额外广播：会话工作台 5s 轮询
 * `list_shelf_sessions` 即读到新值，移动端同源返回体一并跟随。
 */
export async function setSessionSortPrefs(
  groupOrder: string,
  sortKey: string,
): Promise<ShelfSortPrefs> {
  const applied = await invoke<ShelfSortPrefs>('set_session_sort_prefs', { groupOrder, sortKey })
  // bridge 无 Tauri 通道（浏览器回退链路）时返回 null → 回落到请求值；
  // 后端拒绝会 throw，不走这里
  return {
    group_order: applied?.group_order ?? groupOrder,
    sort_key: applied?.sort_key ?? sortKey,
  }
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

/**
 * 后端 InputSpec[] → 前端 WorkflowInputSpec[]
 * 空数组/缺失/非法形状 → undefined（旧工作流不带 inputs 字段，保持语义一致）
 */
function normalizeInputs(raw: unknown): WorkflowInputSpec[] | undefined {
  if (!Array.isArray(raw)) return undefined
  const specs = raw
    .filter((x): x is Record<string, unknown> => !!x && typeof x === 'object')
    .map(x => {
      const kind = typeof x.type === 'string' ? (x.type as WorkflowInputKind) : undefined
      return {
        name: String(x.name ?? ''),
        type: kind,
        required: Boolean(x.required),
        default: x.default,
        description: typeof x.description === 'string' ? x.description : undefined,
        sensitive: Boolean(x.sensitive),
      } satisfies WorkflowInputSpec
    })
    .filter(s => s.name.length > 0)
  return specs.length > 0 ? specs : undefined
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
    inputs: normalizeInputs(raw.inputs),
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

/**
 * 画布确定性触发执行；进度经 workflow-event 推送。
 * fresh=true：失败后从头完整执行（force_fresh，跳过上次断点）；
 * fresh=false / 缺省：断点续连（Paused 续跑自动跳过已完成步骤）。
 * inputs：启动表单收集的声明式输入对象（只含已声明字段；无声明/未填一律省略）。
 */
export function wfRun(id: string, fresh?: boolean, inputs?: Record<string, unknown>) {
  return invoke<string>('wf_run', { id, fresh, inputs })
}

export interface WfScheduleDetails {
  config: ScheduleConfig | null
  inputs: Record<string, unknown>
  sensitive_inputs: string[]
  eligible: boolean
  ineligible_reason?: string | null
}

export interface ScheduleRunRecord {
  run_id: string
  workflow_id: string
  workflow_title: string
  started_at: string
  finished_at?: string | null
  status: RunRecord['status']
  error?: string | null
  steps: NonNullable<RunRecord['steps']>
  step_names?: Record<string, string>
}

export interface ScheduleHistoryPage {
  total: number
  page: number
  page_size: number
  runs: ScheduleRunRecord[]
}

export interface ScheduleHistoryFilter {
  workflow_id?: string
  status?: 'running' | 'success' | 'error' | 'cancelled' | 'paused'
  from?: string
  to?: string
  page?: number
  page_size?: number
}

export function wfScheduleHistoryList(filter: ScheduleHistoryFilter = {}) {
  return invoke<ScheduleHistoryPage>('wf_schedule_history_list', { filter })
}

export function wfScheduleHistoryGet(runId: string) {
  return invoke<ScheduleRunRecord>('wf_schedule_history_get', { runId })
}

export function wfScheduleHistoryDelete(filter: ScheduleHistoryFilter = {}) {
  return invoke<number>('wf_schedule_history_delete', { filter })
}

export function wfScheduleGet(id: string) {
  return invoke<WfScheduleDetails>('wf_schedule_get', { id })
}

export function wfSchedulePreview(config: ScheduleConfig) {
  return invoke<string[]>('wf_schedule_preview', { config })
}

export function wfScheduleSet(
  id: string,
  config: ScheduleConfig,
  inputs: Record<string, unknown>,
  preserveSensitive: string[],
) {
  return invoke<void>('wf_schedule_set', {
    id,
    config,
    inputs,
    preserveSensitive,
  })
}

export function wfScheduleRemove(id: string) {
  return invoke<void>('wf_schedule_remove', { id })
}

export interface WfGateStatus {
  /** true = 已锁定（有 active workflow run 或 Agent busy） */
  locked: boolean
  /** 'workflow'（工作流执行中）| 'agent'（Agent 跑任务）| 'idle' */
  reason: 'workflow' | 'agent' | 'idle'
  owner?: string
  workflow_id?: string
}

/** 全局执行闸门查询（WorkflowPage / CanvasPage 锁定态权威源） */
export function wfGateStatus() {
  return invoke<WfGateStatus>('wf_gate_status')
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
// ── 轻反馈 ──

/**
 * 轻反馈（保留原签名与相位词汇，调用方无需改动）。
 *
 * 改造前：直发 `hud_update` → 屏幕右下角的 HUD 独立窗口，绕过了应用内的
 * 前台/后台分流（应用就在眼前时反馈却甩到屏幕角落）。现在统一转调
 * ui/islandChannel 的收编入口：前台走 island，后台/最小化仍走 HUD。
 * 相位沿用 HUD 词汇（含 island 没有的 'done'），翻译集中在 islandChannel。
 */
export function hudUpdate(text: string, phase: string): void {
  showAppFeedbackByHudPhase(text, phase)
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
  /** 局域网 HTTPS 直连地址（https://IP:18773，自签 CA）；TLS 未启动时为 null */
  lan_url_https: string | null
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
  /** retrying: 最近一次失败摘要（连接成功后随状态复位消失） */
  last_error?: string
  /** fault: 故障原因摘要 */
  reason?: string
}

export interface RelayClientStatus {
  enabled: boolean
  state: { relay: RelayChannelState; tunnel: RelayChannelState }
  /** 隧道公网入口（http://host:18081），中继未启用时为 null；远程配对链接的 base */
  public_url?: string | null
  /** 本机设备标识（中继按此路由）；排查 device_id 不一致用 */
  device_id?: string
}

export function relayClientStatus() {
  return invoke<RelayClientStatus>('relay_client_status')
}

/** 中继开关：持久化 enabled + 运行时即时启停（免重启） */
export function relayClientSetEnabled(enabled: boolean) {
  return invoke<string>('relay_client_set_enabled', { enabled })
}

/** 更新中继节点配置（官方/自建 VPS）：持久化 url/token/public_url + 热重启连接。
 *  token / publicUrl 传空 = 保留现状 */
export function relayClientUpdateNode(url: string, token: string, publicUrl?: string) {
  return invoke<string>('relay_client_update_node', { url, token, publicUrl })
}

/** 恢复官方中继节点（自建 VPS 不满意一键回退）：url/public_url/token 重置官方默认 + 热重启 */
export function relayClientResetOfficial() {
  return invoke<string>('relay_client_reset_official')
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

// ── 数据目录（只读列举） ──

/** 单个数据目录条目；path 为空串 = 本机无法解析该目录（exists 必为 false） */
export interface DataDirEntry {
  /** 稳定标识（data / runtime / generated / plugin / config） */
  key: string
  path: string
  exists: boolean
}

/** 列出本机各数据目录的真实路径（只读；不做目录体积扫描） */
export function listDataDirs() {
  return invoke<DataDirEntry[]>('list_data_dirs')
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

// ── 内置工具（tools/*，内部机制命令；用户手动经工具页调用，非 agent 工具） ──

export interface PdfMergeResult {
  output: string
  pages: number
  sources: number
}
export interface PdfCompressResult {
  output: string
  pages: number
  removed_objects: number
  size_before: number
  size_after: number
  saved_bytes: number
}
export interface PdfPageCountResult {
  path: string
  pages: number
}
export interface PdfExtractTextResult {
  path: string
  pages: number
  extracted_pages: number
  truncated: boolean
  text: string
}

/** 合并多个 PDF 为一个文件 */
export function pdfMerge(inputPaths: string[], outputPath: string) {
  return invoke<PdfMergeResult>('pdf_merge', { inputPaths, outputPath })
}
/** 压缩 PDF（清理未引用对象） */
export function pdfCompress(inputPath: string, outputPath: string) {
  return invoke<PdfCompressResult>('pdf_compress', { inputPath, outputPath })
}
/** 获取 PDF 页数 */
export function pdfPageCount(path: string) {
  return invoke<PdfPageCountResult>('pdf_page_count', { path })
}
/** 提取 PDF 文本（逐页，maxPages 可选，默认 200） */
export function pdfExtractText(path: string, maxPages?: number) {
  return invoke<PdfExtractTextResult>('pdf_extract_text', { path, maxPages })
}

export interface ImageCompressResult {
  output: string
  width: number
  height: number
  size_before: number
  size_after: number
  saved_bytes: number
}
export interface ImageConvertResult {
  output: string
  width: number
  height: number
  format: string
  size_after: number
}
export interface ImageResizeResult {
  output: string
  source_width: number
  source_height: number
  width: number
  height: number
  size_after: number
}
export interface ImageInfoResult {
  path: string
  width: number
  height: number
  size_bytes: number
  format: string
}

/** 压缩图片（可指定最大宽/高与质量；输出格式随扩展名推断） */
export function imageCompress(
  inputPath: string,
  outputPath: string,
  maxWidth?: number,
  maxHeight?: number,
  quality?: number,
) {
  return invoke<ImageCompressResult>('image_compress', {
    inputPath,
    outputPath,
    maxWidth,
    maxHeight,
    quality,
  })
}
/** 转换图片格式 */
export function imageConvert(inputPath: string, outputPath: string) {
  return invoke<ImageConvertResult>('image_convert', { inputPath, outputPath })
}
/** 缩放图片（保纵横比，目标框内不放大） */
export function imageResize(inputPath: string, outputPath: string, width: number, height: number) {
  return invoke<ImageResizeResult>('image_resize', { inputPath, outputPath, width, height })
}
/** 获取图片信息 */
export function imageInfo(path: string) {
  return invoke<ImageInfoResult>('image_info', { path })
}

export interface VideoCompressResult {
  output: string
  quality: string
  size_before: number
  size_after: number
  saved_bytes: number
}
export interface VideoExtractAudioResult {
  output: string
  format: string
  size_after: number
}
export interface VideoExtractFramesResult {
  output_dir: string
  frames: number
  interval_secs: number
}
export interface VideoInfoResult {
  path: string
  duration_seconds: number | null
  video_codec: string | null
  audio_codec: string | null
  width: number | null
  height: number | null
  size_bytes: number
}

/** 压缩视频（quality: low/medium/high） */
export function videoCompress(inputPath: string, outputPath: string, quality?: string) {
  return invoke<VideoCompressResult>('video_compress', { inputPath, outputPath, quality })
}
/** 提取音频（format: mp3/wav） */
export function videoExtractAudio(inputPath: string, outputPath: string, format?: string) {
  return invoke<VideoExtractAudioResult>('video_extract_audio', { inputPath, outputPath, format })
}
/** 抽帧（intervalSecs 秒一张，输出目录 frame_0001.jpg ...） */
export function videoExtractFrames(inputPath: string, outputDir: string, intervalSecs?: number) {
  return invoke<VideoExtractFramesResult>('video_extract_frames', {
    inputPath,
    outputDir,
    intervalSecs,
  })
}
/** 获取视频信息（ffprobe） */
export function videoInfo(path: string) {
  return invoke<VideoInfoResult>('video_info', { path })
}

/** 图片转 PDF（多图各占一页，页面尺寸=图片像素） */
export function pdfImagesToPdf(inputPaths: string[], outputPath: string) {
  return invoke<PdfImagesToPdfResult>('pdf_images_to_pdf', { inputPaths, outputPath })
}
export interface PdfImagesToPdfResult {
  output: string
  pages: number
  sources: number
}

/** 长图拼接（direction: horizontal 横向统一高度 / vertical 纵向统一宽度） */
export function imageStitch(inputPaths: string[], outputPath: string, direction?: string) {
  return invoke<ImageStitchResult>('image_stitch', { inputPaths, outputPath, direction })
}
export interface ImageStitchResult {
  output: string
  width: number
  height: number
  direction: string
  sources: number
  size_after: number
}

/** 视频转 GIF（fps 1-30 默认 10；scale 输出宽度默认 480） */
export function videoToGif(inputPath: string, outputPath: string, fps?: number, scale?: number) {
  return invoke<VideoToGifResult>('video_to_gif', { inputPath, outputPath, fps, scale })
}
export interface VideoToGifResult {
  output: string
  size_before: number
  size_after: number
  fps: number
  scale: number
}

/** 提取 PDF 指定页（1-based 页码数组） */
export function pdfExtractPages(inputPath: string, pages: number[], outputPath: string) {
  return invoke<PdfExtractPagesResult>('pdf_extract_pages', { inputPath, pages, outputPath })
}
export interface PdfExtractPagesResult {
  output: string
  pages: number
  requested: number
}

/** 旋转 PDF 页面（degrees 90/180/270；pages 缺省=全部） */
export function pdfRotate(
  inputPath: string,
  outputPath: string,
  degrees: number,
  pages?: number[],
) {
  return invoke<PdfRotateResult>('pdf_rotate', { inputPath, outputPath, degrees, pages })
}
export interface PdfRotateResult {
  output: string
  degrees: number
  rotated_pages: number
  total_pages: number
}

/** 视频截取片段（-c copy 流复制；start 缺省 0，end 缺省到末尾） */
export function videoCut(
  inputPath: string,
  outputPath: string,
  startSec?: number,
  endSec?: number,
) {
  return invoke<VideoCutResult>('video_cut', { inputPath, outputPath, startSec, endSec })
}
export interface VideoCutResult {
  output: string
  start_sec: number
  end_sec: number | null
  size_bytes: number
}

/** 音频格式转换（输出扩展名决定格式 mp3/wav/m4a/flac；bitrate 可选如 "192k"） */
export function audioConvert(inputPath: string, outputPath: string, bitrate?: string) {
  return invoke<AudioConvertResult>('audio_convert', { inputPath, outputPath, bitrate })
}
export interface AudioConvertResult {
  output: string
  format: string
  size_bytes: number
}

/** 语音克隆：参考音频 + 文本 → 克隆音色合成语音（走云端，需在模型界面配置语音克隆模型） */
export function voiceClone(referencePath: string, text: string, outputPath: string) {
  return invoke<VoiceCloneResult>('voice_clone', {
    referencePath,
    text,
    outputPath,
  })
}
export interface VoiceCloneResult {
  output: string
  format: string
  size_bytes: number
}

/** 批量压缩图片（多输入 → 输出目录，原名-out.原扩展名） */
export function imageCompressBatch(
  inputPaths: string[],
  outputDir: string,
  maxWidth?: number,
  maxHeight?: number,
  quality?: number,
) {
  return invoke<ImageBatchResult>('image_compress_batch', {
    inputPaths,
    outputDir,
    maxWidth,
    maxHeight,
    quality,
  })
}

/** 批量格式转换（多输入 → 输出目录，统一 format） */
export function imageConvertBatch(inputPaths: string[], outputDir: string, format: string) {
  return invoke<ImageBatchResult>('image_convert_batch', { inputPaths, outputDir, format })
}

/** 批量缩放图片（多输入 → 输出目录，目标宽高框内保纵横比） */
export function imageResizeBatch(
  inputPaths: string[],
  outputDir: string,
  width: number,
  height: number,
) {
  return invoke<ImageBatchResult>('image_resize_batch', { inputPaths, outputDir, width, height })
}
export interface ImageBatchResult {
  output_dir: string
  format?: string
  count: number
  results: Array<{
    input: string
    output: string
    saved_bytes?: number
    size_after?: number
    width?: number
    height?: number
  }>
}

/** 文档转文本（docx/pptx/xls/ods/odt/odp/pdf） */
export function docExtractText(path: string) {
  return invoke<DocExtractTextResult>('doc_extract_text', { path })
}
export interface DocExtractTextResult {
  path: string
  chars: number
  text: string
}

// ── CHANGELOG ──

/**
 * 仓库 CHANGELOG 全文（Markdown 原文）。
 * 后端以 `include_str!` 编译期内嵌，无网络、无磁盘依赖 → 离线可读。
 * `invoke` 在通道异常时会回 null（命令本身不可能失败）→ 兜底空串，
 * 调用方据此走「暂无本版变更记录」空态，而非抛错。
 */
export async function getChangelog() {
  return (await invoke<string>('get_changelog')) ?? ''
}

// ── 外链 ──

/**
 * 用系统默认浏览器打开外链（桌面端 WebView 不处理 `target="_blank"`）。
 * 后端只放行 http/https；失败时返回可读原因，调用方通常只记录不打断。
 */
export function openExternal(url: string) {
  return invoke<void>('open_external', { url })
}
