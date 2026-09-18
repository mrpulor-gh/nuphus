export function routePrimaryK(
  mode: string,
  openWorkflowList: () => void,
  toggleCommandPalette: () => void,
) {
  if (mode === 'workflow') openWorkflowList()
  else toggleCommandPalette()
}
