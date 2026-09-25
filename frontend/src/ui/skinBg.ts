/**
 * skinBg — 皮肤背景的**唯一写入入口**（解析 + 广播），不再触碰任何 CSS 值。
 *
 * # 为什么必须由 App 层调用
 *
 * 早先恢复逻辑写在 `ThemesPage` 的挂载 effect 里 —— 那是错的：ThemesPage 被
 * `<CompactModal open={s.showThemes}>` 包裹，而 CompactModal 在 `open=false` 时
 * `return null`（CompactModal.tsx），**关闭状态下 ThemesPage 根本不在树上**。
 * 于是用户在聊天界面刷新 / Vite HMR 时，ThemesPage 不挂载 → 没人恢复背景 →
 * 背景必丢。（打开一次主题弹窗它反倒"出现"，正是这个层级错位的症状。）
 *
 * 因此：恢复必须放在始终挂载的 App 层；本模块只提供幂等的 apply 函数，
 * 保存 / 清除 / 恢复三条路径共用同一实现，避免各写一半。
 *
 * # 为什么不写 CSS 变量（2026-09-26 架构调整）
 *
 * 早先的形态是 `document.documentElement.style.setProperty('--app-skin-bg', url(...))`，
 * 由 `.chat-area::before` 与 `.title-bar::before` 两个伪元素各自消费。该形态有三处
 * 结构性问题，本次一并消除：
 *
 * 1. **同源不同步**：两个 ::before 各写一份 `opacity`（其一还是硬编码 0.35），
 *    调不透明度时标题栏不动。
 * 2. **图片字节进 CSS 值**：撞 Chromium/WebView2 单个 CSS 值 2 MiB 硬上限，
 *    大图 > ~1.5 MB 静默不显示（详见 assetUrl.ts 的 resolveSkinImageUrl）。
 * 3. **消费点分叉**：任何新区域要背景都得再加一条伪元素规则。
 *
 * 现在只有一条路径：本模块解析出可渲染 URL → 广播 `SKIN_CHANGE_EVENT` →
 * 唯一的 `<SkinBackdrop>`（App.tsx 内、`.app-shell` 直接子元素）订阅并渲染。
 * 标题栏与聊天区由**同一个元素**同时覆盖，同步问题从根上消失。
 *
 * # 入参两种形态
 * - 本地文件绝对路径（现行：`save_user_image` 入库后只存路径）
 * - dataURL（升级前存量数据）
 * 由 `resolveSkinImageUrl` 归一，老用户升级后不会因存储格式变化丢背景。
 */

import { resolveSkinImageUrl, releaseSkinImageUrl } from './assetUrl'

/** localStorage 键：皮肤背景（本地文件路径；早期为 dataURL） */
export const LS_SKIN_BG = 'nuphus_skin_bg'

/** 背景变更广播事件：detail.url 为可渲染 URL，null 表示清除。 */
export const SKIN_CHANGE_EVENT = 'nuphus-skin-changed'

/** 读取持久化的皮肤背景值（路径或 dataURL；无则为空串）。 */
export function readSkinBg(): string {
  try {
    return localStorage.getItem(LS_SKIN_BG) || ''
  } catch {
    return ''
  }
}

/** 广播背景变更（消费方：SkinBackdrop）。 */
function broadcast(url: string | null): void {
  window.dispatchEvent(new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url } }))
}

/**
 * 把持久化值应用到背景层（异步：解析 URL 可能要经 Rust 读文件）。
 *
 * 语义分三种，刻意区分（早先合成一种，是缺陷）：
 * - 空串 → **广播清除**（清除背景的正常路径）；
 * - 有值且能解析 → 广播新 URL；
 * - 有值但**解析失败** → 报错并**什么都不广播**。
 *
 * 第三种最关键：解析失败时绝不能广播 null。否则用户正看着一张好背景，只因为新选的
 *   那张没能渲染，眼前这张就被连带清掉 —— 表现为「换了个有问题的图之后，
 *   连原来的背景也不见了」。失败应该只影响新图，不该摧毁现状。
 */
export async function applySkinBg(value: string): Promise<void> {
  if (!value) {
    broadcast(null)
    return
  }
  const url = await resolveSkinImageUrl(value)
  if (!url) {
    console.error('[skin] 背景图无法转为可渲染 URL，已保留当前背景不变：', value)
    return
  }
  broadcast(url)
}

/**
 * 释放 `resolveSkinImageUrl` 可能产生的 blob URL。
 *
 * 仅对 blob: 生效（data:/http(s):/asset: 不是本进程创建的，释放它们无意义）。
 * 由 `SkinBackdrop` 在替换与卸载时调用 —— 消费方契约见 assetUrl.ts。
 */
export function releaseSkinBg(url: string | null | undefined): void {
  releaseSkinImageUrl(url)
}
