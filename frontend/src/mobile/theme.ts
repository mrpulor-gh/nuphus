/**
 * 移动端主题切换：亮色（品牌蓝体系）/ 深色（酷炫黑）
 *
 * - 默认深色（酷炫黑，用户当前视觉基调）
 * - localStorage 记忆（`nuphus_mobile_theme`），跨会话保持
 * - 通过 `<html data-theme="dark|light">` 驱动 CSS token 切换
 */

export type MobileTheme = 'dark' | 'light'

const THEME_KEY = 'nuphus_mobile_theme'
const DEFAULT_THEME: MobileTheme = 'dark'

/** 读取当前主题（localStorage → 默认深色） */
export function getTheme(): MobileTheme {
  const stored = localStorage.getItem(THEME_KEY)
  return stored === 'light' ? 'light' : DEFAULT_THEME
}

/** 应用主题到 <html data-theme>（亮色可不写属性，默认 :root 即亮色） */
export function applyTheme(theme: MobileTheme): void {
  const root = document.documentElement
  if (theme === 'dark') root.setAttribute('data-theme', 'dark')
  else root.removeAttribute('data-theme')
  // 同步浏览器 UI 色（地址栏 / 状态栏 / PWA 启动屏）
  const meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]')
  if (meta) meta.content = theme === 'dark' ? '#0d0d10' : '#ffffff'
}

/** 切换主题：返回切换后的值，并持久化 + 应用 */
export function toggleTheme(): MobileTheme {
  const next: MobileTheme = getTheme() === 'dark' ? 'light' : 'dark'
  localStorage.setItem(THEME_KEY, next)
  applyTheme(next)
  return next
}

/** 启动时初始化（在渲染前调用，避免首屏闪烁） */
export function initTheme(): void {
  applyTheme(getTheme())
}
