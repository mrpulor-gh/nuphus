/**
 * externalLink.ts — 外链点击接管（桌面端 WebView 不处理 `target="_blank"`）。
 *
 * 现状：Tauri WebView 里 `target="_blank"` 既不新开窗也不报错 —— 点了毫无反应。
 * 做法：在 App 层以捕获阶段监听点击，命中外链就拦下并交系统浏览器（`open_external`）。
 *
 * 判定范围刻意收窄：只接管 `<a href="http(s)://...">` 且 `target="_blank"` 的锚点，
 * 其余（站内路由、`#anchor`、`mailto:`、自定义 scheme）一律放行原行为。
 */

/** 命中外链则调用 `open` 并返回 true（调用方应 `preventDefault()`）。 */
export function handleExternalAnchorClick(
  target: EventTarget | null,
  open: (url: string) => void,
): boolean {
  const el = target as (HTMLElement & { closest?: HTMLElement['closest'] }) | null
  if (!el || typeof el.closest !== 'function') return false

  const anchor = el.closest('a[href]') as HTMLAnchorElement | null
  if (!anchor) return false
  if (anchor.getAttribute('target') !== '_blank') return false

  const href = anchor.getAttribute('href') ?? ''
  if (!/^https?:\/\//i.test(href)) return false

  open(href)
  return true
}
