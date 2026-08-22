/* ═══════════════════════════════════════════
   customTheme.ts — 自定义主题数据模型与纯函数工具

   覆盖逻辑的宿主在 useTheme.tsx（应用/清理 inline 覆盖）；
   本文件只负责数据建模 / 颜色派生 / JSON 校验，无 React 依赖。
   数据模型：{ name, base, overrides } —— 未来可直接同步到后端。
   ═══════════════════════════════════════════ */

import type { ThemeId } from './useTheme'

/** 可编辑的核心色 token（色板区） */
export type CoreTokenKey = '--accent' | '--surface-0' | '--surface-1' | '--fg-1' | '--fg-2'

export const CORE_TOKEN_KEYS: readonly CoreTokenKey[] = [
  '--accent',
  '--surface-0',
  '--surface-1',
  '--fg-1',
  '--fg-2',
]

/** 选择 --accent 时一并派生的关联变量 */
export const ACCENT_DERIVED_KEYS = [
  '--accent-rgb',
  '--accent-hover',
  '--accent-dim',
  '--accent-glow',
] as const

export interface CustomTheme {
  /** 内部存储 id（保存/列表管理用）；导入的 JSON 可缺省，保存时生成 */
  id?: string
  name: string
  base: ThemeId
  overrides: Record<string, string>
}

/** 生成自定义主题 id（非安全上下文兼容：不用 crypto.randomUUID） */
export function newCustomThemeId(): string {
  return `ct-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

/* ── 颜色派生常量（与 tokens.css 内置派生关系一致）── */
const ACCENT_HOVER_MIX = 0.25 // accent-hover = accent 向白色混合 25%
const ACCENT_DIM_ALPHA = 0.12 // accent-dim = rgba(accent, .12)
const ACCENT_GLOW_ALPHA = 0.25 // accent-glow = rgba(accent, .25)

/** 归一化为 #rrggbb；支持 3/6 位 hex（可带可不带 #）。非法返回 null */
export function normalizeHexInput(value: string): string | null {
  let v = value.trim().toLowerCase()
  if (v.startsWith('#')) v = v.slice(1)
  if (/^[0-9a-f]{3}$/.test(v)) {
    v = v[0] + v[0] + v[1] + v[1] + v[2] + v[2]
  }
  if (!/^[0-9a-f]{6}$/.test(v)) return null
  return `#${v}`
}

export function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  const h = normalizeHexInput(hex)
  if (!h) return null
  return {
    r: parseInt(h.slice(1, 3), 16),
    g: parseInt(h.slice(3, 5), 16),
    b: parseInt(h.slice(5, 7), 16),
  }
}

export function rgbToHex(r: number, g: number, b: number): string {
  const clamp = (n: number) => Math.max(0, Math.min(255, Math.round(n)))
  const to2 = (n: number) => clamp(n).toString(16).padStart(2, '0')
  return `#${to2(r)}${to2(g)}${to2(b)}`
}

/** 向白色混合 ratio（0~1）得到亮阶变体（用于 --accent-hover） */
export function lightenHex(hex: string, ratio = ACCENT_HOVER_MIX): string {
  const rgb = hexToRgb(hex)
  if (!rgb) return hex
  return rgbToHex(
    rgb.r + (255 - rgb.r) * ratio,
    rgb.g + (255 - rgb.g) * ratio,
    rgb.b + (255 - rgb.b) * ratio,
  )
}

/** 由强调色派生 --accent-rgb / --accent-hover / --accent-dim / --accent-glow */
export function deriveAccentOverrides(accentHex: string): Record<string, string> {
  const rgb = hexToRgb(accentHex)
  if (!rgb) return {}
  return {
    '--accent-rgb': `${rgb.r}, ${rgb.g}, ${rgb.b}`,
    '--accent-hover': lightenHex(accentHex),
    '--accent-dim': `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${ACCENT_DIM_ALPHA})`,
    '--accent-glow': `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${ACCENT_GLOW_ALPHA})`,
  }
}

/** 更新单个核心色 token 到 overrides（--accent 自动带入 4 个派生变量） */
export function applyCoreColor(
  overrides: Record<string, string>,
  key: CoreTokenKey,
  value: string,
): Record<string, string> {
  const next = { ...overrides }
  if (key === '--accent') {
    for (const k of ACCENT_DERIVED_KEYS) delete next[k]
    next['--accent'] = value
    Object.assign(next, deriveAccentOverrides(value))
  } else {
    next[key] = value
  }
  return next
}

/* ── 不透明度覆盖（气泡 / 输入框 / 皮肤背景图）── */

/** 不透明度滑块覆盖的颜色 token：气泡 ×2 + 输入框 + 弹窗（走 rgba 派生） */
export const OPACITY_COLOR_KEYS = [
  '--msg-user-bg',
  '--msg-assistant-bg',
  '--input-bg',
  '--modal-bg',
] as const

/** 皮肤背景图不透明度 token（数值型覆盖，非 rgba 派生） */
export const SKIN_OPACITY_KEY = '--skin-bg-opacity'

/** 解析 hex / rgb() / rgba() 颜色字符串 → {r,g,b}；非法返回 null */
export function parseColorValue(value: string): { r: number; g: number; b: number } | null {
  const v = value.trim()
  const hex = normalizeHexInput(v)
  if (hex) return hexToRgb(hex)
  // 兼容 getComputedStyle 的 rgb()/rgba()：逗号或空格分隔，alpha 可省略
  const m =
    /^rgba?\(\s*([\d.]+%?)\s*[, ]\s*([\d.]+%?)\s*[, ]\s*([\d.]+%?)(?:\s*[,/]\s*[\d.]+%?)?\s*\)$/i.exec(
      v,
    )
  if (m) {
    const to255 = (s: string) =>
      s.endsWith('%') ? Math.round((parseFloat(s) / 100) * 255) : parseFloat(s)
    const r = to255(m[1])
    const g = to255(m[2])
    const b = to255(m[3])
    if (Number.isFinite(r) && Number.isFinite(g) && Number.isFinite(b)) {
      return { r: Math.round(r), g: Math.round(g), b: Math.round(b) }
    }
  }
  return null
}

/** 从 rgb()/rgba() 中解析 alpha（缺省 = 1）；非 rgba 颜色返回 null */
export function parseColorAlpha(value: string): number | null {
  const v = value.trim()
  const m =
    /^rgba?\(\s*[\d.]+%?\s*[, ]\s*[\d.]+%?\s*[, ]\s*[\d.]+%?(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/i.exec(
      v,
    )
  if (!m) return null
  if (m[1] == null) return 1
  const raw = m[1]
  const alpha = raw.endsWith('%') ? parseFloat(raw) / 100 : parseFloat(raw)
  return Number.isFinite(alpha) ? Math.max(0, Math.min(1, alpha)) : null
}

/** rgb + alpha → "rgba(r, g, b, α)"（α 去掉多余尾零） */
export function toRgba(rgb: { r: number; g: number; b: number }, alpha: number): string {
  const a = Math.max(0, Math.min(1, alpha))
  const aStr = String(Math.round(a * 1000) / 1000)
  return `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${aStr})`
}

/** 从 overrides 中剥离派生色（气泡/输入框 rgba），保留皮肤不透明度等数值覆盖 */
export function stripOpacityColorKeys(overrides: Record<string, string>): Record<string, string> {
  const next = { ...overrides }
  for (const key of OPACITY_COLOR_KEYS) delete next[key]
  return next
}

export type CustomThemeParseResult =
  { ok: true; theme: CustomTheme } | { ok: false; reason: 'invalid-json' | 'invalid-structure' }

/** 解析并校验自定义主题 JSON（name/base/overrides 结构） */
export function parseCustomThemeJSON(text: string): CustomThemeParseResult {
  let data: unknown
  try {
    data = JSON.parse(text)
  } catch {
    return { ok: false, reason: 'invalid-json' }
  }
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    return { ok: false, reason: 'invalid-structure' }
  }
  const obj = data as Record<string, unknown>
  const base = obj.base
  if (base !== 'dark' && base !== 'light' && base !== 'tech') {
    return { ok: false, reason: 'invalid-structure' }
  }
  const overrides = obj.overrides
  if (!overrides || typeof overrides !== 'object' || Array.isArray(overrides)) {
    return { ok: false, reason: 'invalid-structure' }
  }
  const ov: Record<string, string> = {}
  for (const [key, value] of Object.entries(overrides as Record<string, unknown>)) {
    if (!key.startsWith('--') || typeof value !== 'string') {
      return { ok: false, reason: 'invalid-structure' }
    }
    ov[key] = value
  }
  const name = typeof obj.name === 'string' ? obj.name.trim() : ''
  const id = typeof obj.id === 'string' && obj.id ? obj.id : undefined
  return { ok: true, theme: { id, name, base: base as ThemeId, overrides: ov } }
}
