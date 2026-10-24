/**
 * 主快捷键路由：Ctrl+K / Ctrl+F 在**所有模式**下语义一致。
 *
 * 历史：Workflow 模式曾把 Ctrl+K 接管为「打开工作流列表」，同一快捷键在不同模式
 * 行为分裂——在 Workflow 会话里按 Ctrl+K 得不到命令面板，自动化脚本按「Ctrl+K →
 * 输入搜索词」的通行套路会直接落到工作流列表上。现拆成两条独立快捷键：
 *   - Ctrl+K = 命令面板（全局唯一入口）
 *   - Ctrl+F = 工作流列表（Find / 列表的直觉语义，同样全局可用）
 */
export function routePrimaryK(toggleCommandPalette: () => void) {
  toggleCommandPalette()
}

export function routePrimaryF(openWorkflowList: () => void) {
  openWorkflowList()
}

export function modelSetupHint(mode: string, shortcut: string): string {
  return mode === 'workflow'
    ? '⚠ 尚未配置模型，请前往模型设置'
    : `⚠ 尚未配置模型，${shortcut} → 模型设置`
}
