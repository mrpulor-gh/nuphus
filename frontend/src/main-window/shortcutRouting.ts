export function routePrimaryK(
  mode: string,
  openWorkflowList: () => void,
  toggleCommandPalette: () => void,
) {
  if (mode === 'workflow') openWorkflowList()
  else toggleCommandPalette()
}

export function modelSetupHint(mode: string, shortcut: string): string {
  return mode === 'workflow'
    ? '⚠ 尚未配置模型，请前往模型设置'
    : `⚠ 尚未配置模型，${shortcut} → 模型设置`
}
