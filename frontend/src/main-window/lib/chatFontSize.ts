/**
 * chatFontSize.ts — 会话内容字号（通用设置 chat_font_size）的前端注入点。
 *
 * 作用域：**仅聊天消息区正文**（styles/chat-messages.css 的 `.message-content`
 * 消费 `--chat-font-size`），不做全局 UI 缩放——其余界面文字保持既有字号。
 *
 * 注入层：`document.documentElement` 内联样式（与主题覆盖同一层），默认值由 CSS
 * 回退承担（`var(--chat-font-size, var(--fz-lg))` = 14px = 历史正文大小），
 * 未设置过该偏好的用户与历史渲染完全一致。
 *
 * 消费方：App 层启动恢复（App.tsx 挂载 effect）+ 通用页设置链路（GeneralPage）。
 * 唯一事实源是后端 UserPreferences.chat_font_size —— 本模块不存档、不缓存。
 */

/** 默认字号（px）。与后端 `CHAT_FONT_SIZE_DEFAULT` 一一对应。 */
export const CHAT_FONT_SIZE_DEFAULT = 14

/** 允许的字号档位（px）。必须与后端 `CHAT_FONT_SIZE_MIN/MAX`（12-16）区间一致。 */
export const CHAT_FONT_SIZE_OPTIONS = [12, 13, 14, 15, 16] as const

/** 注入 CSS 变量（px）。传入后端归一后的读数；越界值由调用方/后端保证不至此处。 */
export function applyChatFontSize(px: number) {
  document.documentElement.style.setProperty('--chat-font-size', `${px}px`)
}
