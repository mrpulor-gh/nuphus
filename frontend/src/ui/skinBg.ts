/**
 * applySkinBg — 皮肤背景（CSS `--app-skin-bg`）的唯一写入点
 *
 * # 为什么必须在共享模块、且由 App 层调用
 *
 * 皮肤的消费点是 `chat-messages.css` 的
 * `.chat-area::before { background: var(--app-skin-bg) }`，而 `.chat-area`
 * 位于 **App.tsx（常驻主 UI）**内；变量本身挂在 `document.documentElement`
 * 上（inline style），谁写都全局生效。
 *
 * 早先恢复逻辑写在 `ThemesPage` 的挂载 effect 里 —— 那是错的：ThemesPage 被
 * `<CompactModal open={s.showThemes}>` 包裹，而 CompactModal 在 `open=false` 时
 * `return null`（CompactModal.tsx），**关闭状态下 ThemesPage 根本不在树上**。
 * 于是用户在聊天界面刷新 / Vite HMR 时，ThemesPage 不挂载 → 没人恢复
 * `--app-skin-bg` → 变量回落到默认值 `none` → 背景必丢。
 * （打开一次主题弹窗它反倒"出现"，正是这个层级错位的症状。）
 *
 * 因此：恢复必须放在始终挂载的 App 层；本模块只提供幂等的写入函数，
 * 保存 / 清除 / 恢复三条路径共用同一实现，避免各写一半。
 *
 * # 入参两种形态
 * - 本地文件绝对路径（现行：`save_user_image` 入库后只存路径）
 * - dataURL（升级前存量数据）
 * 由 `toAssetUrl` 归一，老用户升级后不会因存储格式变化丢背景。
 */

import { resolveLocalImageUrl } from './assetUrl'

/** localStorage 键：皮肤背景（本地文件路径；早期为 dataURL） */
export const LS_SKIN_BG = 'nuphus_skin_bg'

/** 读取持久化的皮肤背景值（路径或 dataURL；无则为空串）。 */
export function readSkinBg(): string {
  try {
    return localStorage.getItem(LS_SKIN_BG) || ''
  } catch {
    return ''
  }
}

/**
 * 把持久化值应用到 DOM（异步：解析 URL 可能要经 Rust 读文件）。
 *
 * 语义分三种，刻意区分（早先合成一种，是缺陷）：
 * - 空串 → **移除**变量（清除背景的正常路径）；
 * - 有值且能解析 → 写入；
 * - 有值但**解析失败** → 报错并**保留当前变量不动**。
 *
 * 第三种最关键：解析失败时绝不能移除。否则用户正看着一张好背景，只因为新选的
 *   那张没能渲染，眼前这张就被连带清掉 —— 表现为「换了个有问题的图之后，
 *   连原来的背景也不见了」。失败应该只影响新图，不该摧毁现状。
 */
export async function applySkinBg(value: string): Promise<void> {
  if (!value) {
    document.documentElement.style.removeProperty('--app-skin-bg')
    return
  }
  const url = await resolveLocalImageUrl(value)
  if (!url) {
    console.error('[skin] 背景图无法转为可渲染 URL，已保留当前背景不变：', value)
    return
  }
  document.documentElement.style.setProperty('--app-skin-bg', `url(${url})`)
}
