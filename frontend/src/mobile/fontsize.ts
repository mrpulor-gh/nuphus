/**
 * 移动端文字大小：标准 16px / 大 17px / 特大 18px
 *
 * - 默认标准（与既有设计字阶一致）
 * - localStorage 记忆（`nuphus_mobile_fontsize`），跨会话保持
 * - 通过 `<html data-fontsize="standard|large|xlarge">` 驱动
 *   `--m-fs-scale` 变量（正文 16px 基准等比缩放），对齐 theme.ts 模式
 */

export type MobileFontSize = 'standard' | 'large' | 'xlarge'

const FONT_SIZE_KEY = 'nuphus_mobile_fontsize'
const DEFAULT_FONT_SIZE: MobileFontSize = 'standard'

const SIZES: MobileFontSize[] = ['standard', 'large', 'xlarge']

/** 读取当前字号（localStorage → 默认标准） */
export function getFontSize(): MobileFontSize {
  const stored = localStorage.getItem(FONT_SIZE_KEY)
  return SIZES.includes(stored as MobileFontSize) ? (stored as MobileFontSize) : DEFAULT_FONT_SIZE
}

/** 应用字号到 <html data-fontsize>（标准可不写属性，默认 :root 即标准） */
export function applyFontSize(size: MobileFontSize): void {
  const root = document.documentElement
  if (size === 'standard') root.removeAttribute('data-fontsize')
  else root.setAttribute('data-fontsize', size)
}

/** 设置字号：持久化 + 应用，返回设置值 */
export function setFontSize(size: MobileFontSize): MobileFontSize {
  localStorage.setItem(FONT_SIZE_KEY, size)
  applyFontSize(size)
  return size
}

/** 启动时初始化（在渲染前调用，避免首屏字号跳动） */
export function initFontSize(): void {
  applyFontSize(getFontSize())
}
