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
const ACCENT_DERIVED_KEYS = [
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
  /**
   * 该主题的皮肤背景快照（本地图片路径，形态同 `ui/skinBg.ts` 的 LS_SKIN 值）。
   * 缺省 / 空 = 无皮肤：激活这类主题时全局背景被清空（不继承上一个主题的图）。
   * 边界：LS_SKIN 仍是「系统预设态的背景」；激活自定义主题以本快照为准，
   * 未激活（停用 / 删除激活项 / 切内置主题）时回 LS_SKIN —— 见 useTheme.tsx 顶部注释。
   */
  skin?: string
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
function lightenHex(hex: string, ratio = ACCENT_HOVER_MIX): string {
  const rgb = hexToRgb(hex)
  if (!rgb) return hex
  return rgbToHex(
    rgb.r + (255 - rgb.r) * ratio,
    rgb.g + (255 - rgb.g) * ratio,
    rgb.b + (255 - rgb.b) * ratio,
  )
}

/** 由强调色派生 --accent-rgb / --accent-hover / --accent-dim / --accent-glow */
function deriveAccentOverrides(accentHex: string): Record<string, string> {
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

/**
 * 不透明度滑块覆盖的颜色 token：气泡 ×2 + 输入框 + 面板 + 弹窗（走 rgba 派生）。
 *
 * `--panel-bg` 是设置中心面板底（语义上属弹窗族，与 `--modal-bg` 同值但独立成键）：
 * 面板原先用不透明硬色 `--surface-0`，滑块遍历不到它 → 面板永远是一块盖死皮肤的
 * 白板。并入本集合后，滑块与「切主题后按新基底重派」两条链路同时覆盖它。
 */
export const OPACITY_COLOR_KEYS = [
  '--msg-user-bg',
  '--msg-assistant-bg',
  '--input-bg',
  '--panel-bg',
  '--modal-bg',
] as const

/** 皮肤背景图不透明度 token（数值型覆盖，非 rgba 派生） */
export const SKIN_OPACITY_KEY = '--skin-bg-opacity'

/** 不透明度滑块的通道名（与 AppearancePanel 的 OpacityAlphas 一一对应） */
export type OpacityChannel = 'bubbles' | 'input' | 'panel' | 'modal' | 'skin'

/**
 * 通道的规范顺序（与滑块在设置页的排列一致）：前四项是 rgba 派生颜色通道，
 * 与 `OPACITY_COLOR_KEYS` 一一对应；末项 `skin` 是数值型（背景图不透明度）。
 *
 * 存在的意义是让「加了颜色键却忘了加 intent 通道」这类漏配在测试里立刻暴露，
 * 而不是等用户反馈「某个滑块重开页面后失效」。
 */
export const OPACITY_INTENT_KEY_ORDER: readonly OpacityChannel[] = [
  'bubbles',
  'input',
  'panel',
  'modal',
  'skin',
]

/**
 * 外观微调层的持久化（2026-10 二次定稿：**按主题各自一份**，改前先读）。
 *
 * 「微调」= 5 个不透明度通道 + 5 个核心色 token。它不是全局偏好，而是**某个主题的
 * 参数**：内置主题按 `dark / light / fresh` 分槽，自定义主题按主题 id 分槽。于是：
 * - 切主题 → 加载目标槽位自己的值（没调过就是该主题默认值），A 的参数不会带到 B；
 * - 切回来 → A 的参数原样回来（不丢）；
 * - 重启 → 当前主题（`activeCustomId ?? LS_THEME`）的槽位被恢复，不回到出厂值。
 *
 * 存「值」而不是派生后的 rgba：百分比 / α 属于用户，rgb 属于基底（深色气泡底 vs
 * 浅色气泡底），换基底必须按新基底重新派生（见 deriveTweaksOverrides）。
 *
 * 兼容：上一层迭代（同日早期实现）存的是「一份不分主题的微调」——读取时作为
 * `legacy` 返回，由 useTheme 迁移到当前主题槽位；更早的布尔意图键
 * `nuphus_opacity_intent` 直接忽略。
 */
const LS_TWEAKS = 'nuphus_appearance_tweaks'

/** 微调作用域：内置主题 id 或自定义主题 id */
export type TweakScope = string

/** 微调层：缺省的通道 / token 表示「用该基底默认值」，不写任何覆盖 */
export interface AppearanceTweaks {
  alphas: Partial<Record<OpacityChannel, number>>
  tokens: Partial<Record<CoreTokenKey, string>>
}

/** 全量槽位表（scope → 微调层） */
export type TweakStore = Record<string, AppearanceTweaks>

/** 空微调层（稳定引用，避免每次渲染生成新对象） */
export const EMPTY_TWEAKS: AppearanceTweaks = { alphas: {}, tokens: {} }

const isOpacityChannel = (v: string): v is OpacityChannel =>
  v === 'bubbles' || v === 'input' || v === 'panel' || v === 'modal' || v === 'skin'

const clampPercent = (n: number) => Math.max(0, Math.min(100, Math.round(n)))

/** 把任意值收敛成合法微调层（缺省 / 损坏字段直接丢弃） */
function parseTweaks(input: unknown): AppearanceTweaks {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return EMPTY_TWEAKS
  const obj = input as { alphas?: unknown; tokens?: unknown }
  const alphas: Partial<Record<OpacityChannel, number>> = {}
  if (obj.alphas && typeof obj.alphas === 'object' && !Array.isArray(obj.alphas)) {
    for (const [key, value] of Object.entries(obj.alphas as Record<string, unknown>)) {
      if (isOpacityChannel(key) && typeof value === 'number' && Number.isFinite(value)) {
        alphas[key] = clampPercent(value)
      }
    }
  }
  const tokens: Partial<Record<CoreTokenKey, string>> = {}
  if (obj.tokens && typeof obj.tokens === 'object' && !Array.isArray(obj.tokens)) {
    for (const [key, value] of Object.entries(obj.tokens as Record<string, unknown>)) {
      if (!(CORE_TOKEN_KEYS as readonly string[]).includes(key)) continue
      const hex = typeof value === 'string' ? normalizeHexInput(value) : null
      if (hex) tokens[key as CoreTokenKey] = hex
    }
  }
  return { alphas, tokens }
}

export function isEmptyTweaks(t: AppearanceTweaks): boolean {
  return Object.keys(t.alphas).length === 0 && Object.keys(t.tokens).length === 0
}

/**
 * 读取全量槽位表。早期「一份不分主题」的存储被识别为 `legacy` 返回 ——
 * 迁移到哪个槽位只有 useTheme 知道（它持有当前主题），故不在此处落盘。
 */
export function readTweakStore(): { store: TweakStore; legacy: AppearanceTweaks | null } {
  try {
    const raw = localStorage.getItem(LS_TWEAKS)
    if (!raw) return { store: {}, legacy: null }
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { store: {}, legacy: null }
    }
    const obj = parsed as Record<string, unknown>
    // 旧形态：顶层直接是 { alphas, tokens }
    if ('alphas' in obj || 'tokens' in obj) return { store: {}, legacy: parseTweaks(obj) }
    const store: TweakStore = {}
    for (const [scope, value] of Object.entries(obj)) {
      const tweaks = parseTweaks(value)
      if (!isEmptyTweaks(tweaks)) store[scope] = tweaks
    }
    return { store, legacy: null }
  } catch {
    return { store: {}, legacy: null }
  }
}

/** 落盘全量槽位表（唯一写入点；空表直接删键） */
export function writeTweakStore(store: TweakStore): void {
  try {
    if (Object.keys(store).length === 0) localStorage.removeItem(LS_TWEAKS)
    else localStorage.setItem(LS_TWEAKS, JSON.stringify(store))
  } catch {}
}

/** 取某主题的微调层（无 = 空层） */
export function getScopeTweaks(store: TweakStore, scope: TweakScope): AppearanceTweaks {
  return store[scope] ?? EMPTY_TWEAKS
}

/** 写某主题的微调层（空层 = 删除该槽位，不留 `{}` 残渣） */
export function setScopeTweaks(
  store: TweakStore,
  scope: TweakScope,
  tweaks: AppearanceTweaks,
): TweakStore {
  const next = { ...store }
  if (isEmptyTweaks(tweaks)) delete next[scope]
  else next[scope] = tweaks
  return next
}

/** 设置某通道的不透明度（纯函数，返回新层） */
export function setTweakAlpha(
  t: AppearanceTweaks,
  channel: OpacityChannel,
  percent: number,
): AppearanceTweaks {
  return { ...t, alphas: { ...t.alphas, [channel]: clampPercent(percent) } }
}

/** 恢复某通道默认（删键 = 回落当前基底默认值） */
export function resetTweakAlpha(t: AppearanceTweaks, channel: OpacityChannel): AppearanceTweaks {
  const alphas = { ...t.alphas }
  delete alphas[channel]
  return { ...t, alphas }
}

/** 设置某核心色（非法 hex 原样返回，不写脏值） */
export function setTweakToken(
  t: AppearanceTweaks,
  key: CoreTokenKey,
  hex: string,
): AppearanceTweaks {
  const normalized = normalizeHexInput(hex)
  if (!normalized) return t
  return { ...t, tokens: { ...t.tokens, [key]: normalized } }
}

/** 恢复某核心色默认 */
export function resetTweakToken(t: AppearanceTweaks, key: CoreTokenKey): AppearanceTweaks {
  const tokens = { ...t.tokens }
  delete tokens[key]
  return { ...t, tokens }
}

/** 通道 → 受它驱动的颜色 token（与不透明度滑块的排列一一对应） */
const CHANNEL_COLOR_TOKENS: Record<'bubbles' | 'input' | 'panel' | 'modal', readonly string[]> = {
  bubbles: ['--msg-user-bg', '--msg-assistant-bg'],
  input: ['--input-bg'],
  panel: ['--panel-bg'],
  modal: ['--modal-bg'],
}

/** --accent 在 overrides 里连同派生变量一起占用（见 applyCoreColor） */
const ACCENT_OVERRIDE_KEYS: readonly string[] = ['--accent', ...ACCENT_DERIVED_KEYS]

/** 某条微调项占用的 overrides 键 */
function tweakOverrideKeys(
  target: { channel: OpacityChannel } | { token: CoreTokenKey },
): readonly string[] {
  if ('channel' in target) {
    if (target.channel === 'skin') return [SKIN_OPACITY_KEY]
    return CHANNEL_COLOR_TOKENS[target.channel]
  }
  return target.token === '--accent' ? ACCENT_OVERRIDE_KEYS : [target.token]
}

/**
 * 从一份 overrides（主题快照）里摘掉某条微调项占用的键。
 *
 * 为什么需要：自定义主题把当时的值烘进了 `overrides`（主题层）。若只在微调层里删键，
 * 主题层仍会把它套回去 —— 表现为「点了恢复默认却没变」。无变化时返回原对象。
 */
export function stripTweakKeys(
  overrides: Record<string, string>,
  target: { channel: OpacityChannel } | { token: CoreTokenKey },
): Record<string, string> {
  const keys = tweakOverrideKeys(target)
  if (!keys.some(key => overrides[key] !== undefined)) return overrides
  const next = { ...overrides }
  for (const key of keys) delete next[key]
  return next
}

/** 某主题快照里是否含有该微调项（决定「恢复默认」是否可点） */
export function overridesHaveTweak(
  overrides: Record<string, string> | null | undefined,
  target: { channel: OpacityChannel } | { token: CoreTokenKey },
): boolean {
  if (!overrides) return false
  return tweakOverrideKeys(target).some(key => overrides[key] !== undefined)
}

/**
 * 把微调层派生为 CSS 覆盖集合。
 *
 * `readBase(key)` 必须返回**当前基底**该 token 的计算值（不含本层覆盖）：
 * 由 useTheme 在两阶段 apply 的第一阶段（只上主题层）之后读取，读到的就是基底色。
 * 通道缺省 = 不写覆盖，让基底 CSS 默认值生效（这正是「恢复默认」的实现方式）。
 */
export function deriveTweaksOverrides(
  t: AppearanceTweaks,
  readBase: (key: string) => string,
): Record<string, string> {
  let out: Record<string, string> = {}
  for (const [key, hex] of Object.entries(t.tokens) as [CoreTokenKey, string][]) {
    out = applyCoreColor(out, key, hex)
  }
  for (const channel of Object.keys(
    CHANNEL_COLOR_TOKENS,
  ) as (keyof typeof CHANNEL_COLOR_TOKENS)[]) {
    const percent = t.alphas[channel]
    if (percent === undefined) continue
    for (const key of CHANNEL_COLOR_TOKENS[channel]) {
      const rgb = parseColorValue(readBase(key))
      if (!rgb) continue
      out[key] = toRgba(rgb, percent / 100)
    }
  }
  const skin = t.alphas.skin
  if (skin !== undefined) out[SKIN_OPACITY_KEY] = String(skin / 100)
  return out
}

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

/** 两个覆盖集合是否等价（键集合相同且逐键同值；键序无关） */
export function sameOverrides(a: Record<string, string>, b: Record<string, string>): boolean {
  const keys = Object.keys(a)
  if (keys.length !== Object.keys(b).length) return false
  return keys.every(k => a[k] === b[k])
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
  if (base !== 'dark' && base !== 'light' && base !== 'fresh') {
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
  // skin 缺省按无皮肤处理（schema 不强制）：旧导出/旧存储没有该字段，导入后主题即无皮肤
  const skin = typeof obj.skin === 'string' && obj.skin ? obj.skin : undefined
  return { ok: true, theme: { id, name, base: base as ThemeId, overrides: ov, skin } }
}
