export function isMacPlatform(platform = globalThis.navigator?.platform ?? ''): boolean {
  return /Mac|iPhone|iPad|iPod/i.test(platform)
}

export function formatPrimaryShortcut(key: string, platform?: string): string {
  return `${isMacPlatform(platform) ? 'Cmd' : 'Ctrl'}+${key}`
}
