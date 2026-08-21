// plugin-apps.ts — App Plugin 体系的后端命令封装（契约 docs/plugin-app-system-plan.md §5/§6）
import { invoke } from '../../core/bridge'

export interface PluginAppManifest {
  id: string
  name: string
  version: string
  entry: string
  icon?: string
  description?: string
  author?: string
  homepage?: string
  minHost?: string
  permissions: string[]
  sidecar?: unknown
}

/** plugin_app_list 返回摘要（后端 PluginAppSummary，camelCase） */
export interface PluginAppSummary {
  id: string
  name: string
  version: string
  description?: string
  /** 图标文件名（如 icon.png） */
  icon?: string
  /** 展示分类（缺失/未知 → 前端归入 other） */
  category?: string
  /** 详情视图展示：开发者 / 主页 */
  author?: string
  homepage?: string
  /** 官方示例标记（manifest sample=true 透传；缺省 false 不渲染徽章） */
  sample?: boolean
  permissions: string[]
  enabled: boolean
  installedAt: string
}

export interface ThemeSnapshot {
  base: string
  overrides: Record<string, string>
}

/** 与 useInit Toast 一致的 toast 函数形状（ThemesPage 同款局部类型） */
export type PluginToastFn = (message: string, type?: 'info' | 'success' | 'warning' | 'error') => void

// ── 安装器（4） ──

export function pluginAppInstall(path: string) {
  return invoke<PluginAppManifest>('plugin_app_install', { path })
}

export function pluginAppList() {
  return invoke<PluginAppSummary[]>('plugin_app_list')
}

export function pluginAppUninstall(id: string) {
  return invoke<void>('plugin_app_uninstall', { id })
}

export function pluginAppSetEnabled(id: string, enabled: boolean) {
  return invoke<void>('plugin_app_set_enabled', { id, enabled })
}

/** 打包导出已安装插件为 .nuph（创作侧闭环：卡片「打包」按钮） */
export function pluginAppPack(id: string, destPath: string) {
  return invoke<void>('plugin_app_pack', { id, destPath })
}

// ── 插件 KV（4） ──

export function pluginKvGet(id: string, key: string) {
  return invoke<unknown>('plugin_kv_get', { id, key })
}

export function pluginKvSet(id: string, key: string, value: unknown) {
  return invoke<void>('plugin_kv_set', { id, key, value })
}

export function pluginKvDelete(id: string, key: string) {
  return invoke<void>('plugin_kv_delete', { id, key })
}

export function pluginKvKeys(id: string) {
  return invoke<string[]>('plugin_kv_keys', { id })
}

// ── agent.chat（1）──

export interface PluginChatHistoryItem {
  role: 'user' | 'assistant' | 'system'
  content: string
}

export function pluginAgentChat(
  id: string,
  message: string,
  history?: PluginChatHistoryItem[],
) {
  const args: Record<string, unknown> = { id, message }
  if (history !== undefined) args.history = history
  return invoke<string>('plugin_agent_chat', args)
}

// ── 工作流 Bridge（2；workflow.list 与 workflow.run 均挂 workflow.run 单一权限，禁止新增枚举）──

/** plugin_workflow_list 返回的摘要（后端 PluginWorkflowSummary，camelCase） */
export interface PluginWorkflowSummary {
  id: string
  name: string
  /** 工作流生命周期状态（Draft/Ready/Running/Completed/Error） */
  status: string
  stepCount: number
}

/** plugin_workflow_run 终态结果：status ∈ "completed" / "failed"；失败时 error 携带原因 */
export interface PluginWorkflowRunResult {
  status: string
  error?: string
}

/** 插件列出用户工作流（只读，不触发执行；权限由桥接器校验 workflow.run） */
export function pluginWorkflowList() {
  return invoke<PluginWorkflowSummary[]>('plugin_workflow_list')
}

/** 插件触发工作流执行并同步等待终态（后端 300s 硬超时，前端 120s 先行返回 TIMEOUT） */
export function pluginWorkflowRun(id: string, workflowId: string) {
  return invoke<PluginWorkflowRunResult>('plugin_workflow_run', { pluginId: id, workflowId })
}

/** 导出官方示例工程到 destDir/{sampleId}/（模板编译期内嵌，安装包形态下可用） */
export function pluginExportSample(sampleId: string, destDir: string) {
  return invoke<string>('plugin_export_sample', { sampleId, destDir })
}

// ── 主题快照（1） ──

export function themeSnapshotSave(base: string, overrides: Record<string, string>) {
  return invoke<void>('theme_snapshot_save', { base, overrides })
}