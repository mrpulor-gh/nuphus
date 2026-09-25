import { useState, useEffect, useRef } from 'react'
import { useTheme } from '../../hooks/useTheme'
import type { ThemeId } from '../../hooks/useTheme'
import {
  CORE_TOKEN_KEYS,
  OPACITY_COLOR_KEYS,
  SKIN_OPACITY_KEY,
  applyCoreColor,
  markOpacityIntent,
  normalizeHexInput,
  parseColorAlpha,
  parseColorValue,
  parseCustomThemeJSON,
  readOpacityIntent,
  stripOpacityColorKeys,
  toRgba,
  type CoreTokenKey,
  type CustomTheme,
  type OpacityChannel,
} from '../../hooks/customTheme'
import { Save, RotateCcw, Download, Upload, X } from 'lucide-react'
import { LetterAvatar } from '../../ui/LetterAvatar'
import { applySkinBg, readSkinBg, releaseSkinBg, SKIN_CHANGE_EVENT } from '../../ui/skinBg'
import { toAssetUrl } from '../../ui/assetUrl'
import { Button } from '../../ui/Button'
import { Section, FormRow } from '../../ui/PageLayout'
import { setLanguage as apiSetLanguage, getLanguage } from '../lib/api'
import { useLanguage } from '../../locales'
import '../../styles/themes.css'

/* 主题预览色板数据（展示用主题色，属于内容数据而非样式） */
const THEMES = [
  { id: 'dark' as ThemeId, bg: '#12121a', accent: '#3b82f6' },
  { id: 'light' as ThemeId, bg: '#f0f4f8', accent: '#2563eb' },
  { id: 'tech' as ThemeId, bg: '#020408', accent: '#7c6ff7' },
]

const LANG = [
  { id: 'zh', label: 'lang.zh' },
  { id: 'en', label: 'lang.en' },
]

/* 自定义主题可编辑的核心色 token 展示名 */
const TOKEN_LABELS: Record<CoreTokenKey, string> = {
  '--accent': 'themes.tokenAccent',
  '--surface-0': 'themes.tokenSurface0',
  '--surface-1': 'themes.tokenSurface1',
  '--fg-1': 'themes.tokenFg1',
  '--fg-2': 'themes.tokenFg2',
}

/* 无覆盖时的稳定空对象（避免每次渲染生成新引用触发 effect） */
const EMPTY_OVERRIDES: Record<string, string> = {}

const LS_LANG = 'nuphus_language'
const LS_SKIN = 'nuphus_skin_bg'
const LS_SHOW_AVATAR = 'nuphus_show_avatar'
const LS_USER_AVATAR = 'nuphus_user_avatar'
const LS_NUPHUS_AVATAR = 'nuphus_nuphus_avatar'

type ToastFn = (message: string, type?: 'info' | 'success' | 'warning' | 'error') => void

/* ── 核心色 token 行：color picker + 可手改 hex ── */
function ColorTokenRow({
  tokenKey,
  label,
  value,
  onCommit,
}: {
  tokenKey: CoreTokenKey
  label: string
  value: string
  onCommit: (key: CoreTokenKey, hex: string) => void
}) {
  const [text, setText] = useState(value)

  // 外部值变化（换基底 / 导入 / 恢复默认）时同步文本
  useEffect(() => {
    setText(value)
  }, [value])

  const commitText = () => {
    const normalized = normalizeHexInput(text)
    if (normalized) onCommit(tokenKey, normalized)
    else setText(value)
  }

  return (
    <FormRow
      label={
        <span className="color-token-label">
          <span>{label}</span>
          <span className="color-token-key">{tokenKey}</span>
        </span>
      }
      control={
        <span className="color-token-control">
          <input
            type="color"
            className="color-token-picker"
            value={value}
            onChange={e => onCommit(tokenKey, e.target.value)}
            aria-label={label}
          />
          <input
            type="text"
            className="color-token-hex"
            value={text}
            onChange={e => setText(e.target.value)}
            onBlur={commitText}
            onKeyDown={e => {
              if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
            }}
            spellCheck={false}
            aria-label={`${label} hex`}
          />
        </span>
      }
    />
  )
}

/* ── 不透明度滑块 ── */
interface OpacityAlphas {
  bubbles: number // 0-100（%）
  input: number // 0-100（%）
  panel: number // 0-100（%）
  skin: number // 0-100（%）
  modal: number // 0-100（%）
}

/** 读当前生效的皮肤不透明度（%）——无覆盖时来自 tokens.css :root 默认 0.35 */
function readSkinOpacityPercent(): number {
  const cs = getComputedStyle(document.documentElement)
  const raw = cs.getPropertyValue(SKIN_OPACITY_KEY).trim()
  const n = raw ? parseFloat(raw) : Number.NaN
  return Number.isFinite(n) ? Math.round(n * 100) : 35
}

/**
 * 读某个颜色 token 当前生效的 α（%）：
 * 有覆盖 → 直接读覆盖值；无覆盖 → 读 **computed** 值（tokens.css 里它就可能是半透明，
 * 如 --modal-bg 默认 0.8 / --panel-bg 默认 0.9），拿不到才回落 100%。
 *
 * 早先只有 --modal-bg 做了 computed 兜底，输入框/气泡走 `colorPercent` 直接返回 100 ——
 * 于是「滑块读数」与「画面实际 α」在无覆盖时必然脱钩（例如输入框已因覆盖变淡，
 * 但一旦覆盖被切主题流程剥掉，读数仍是旧值，用户看到的却是实色）。
 */
function readColorOpacityPercent(key: string): number {
  const cs = getComputedStyle(document.documentElement)
  const computed = parseColorAlpha(cs.getPropertyValue(key).trim())
  return computed === null ? 100 : Math.round(computed * 100)
}

/** 读当前生效的面板不透明度（%）——无覆盖时读 computed --panel-bg（三主题默认 0.9） */
function readPanelOpacityPercent(): number {
  return readColorOpacityPercent('--panel-bg')
}

/** 读当前生效的弹窗不透明度（%）——无覆盖时读 computed --modal-bg（三主题默认 0.8） */
function readModalOpacityPercent(): number {
  return readColorOpacityPercent('--modal-bg')
}

/** 从 overrides 反向初始化滑块：rgba → α；无覆盖 → 读该键当前 computed α */
function readOpacityAlphas(overrides: Record<string, string>, skinDefault: number): OpacityAlphas {
  const colorPercent = (keys: readonly string[]): number => {
    for (const key of keys) {
      const raw = overrides[key]
      if (raw !== undefined) {
        const alpha = parseColorAlpha(raw)
        return alpha === null ? 100 : Math.round(alpha * 100)
      }
    }
    // 无覆盖 → 按该键的实际生效值（基底可能是半透明玻璃，如弹窗/面板族）
    return keys.length > 0 ? readColorOpacityPercent(keys[0]) : 100
  }
  const rawSkin = overrides[SKIN_OPACITY_KEY]
  const skin =
    rawSkin !== undefined && Number.isFinite(Number(rawSkin))
      ? Math.round(Math.max(0, Math.min(1, Number(rawSkin))) * 100)
      : skinDefault
  const modalOverride = overrides['--modal-bg']
  const modal =
    modalOverride !== undefined ? colorPercent(['--modal-bg']) : readModalOpacityPercent()
  const panelOverride = overrides['--panel-bg']
  const panel =
    panelOverride !== undefined ? colorPercent(['--panel-bg']) : readPanelOpacityPercent()
  return {
    bubbles: colorPercent(['--msg-user-bg', '--msg-assistant-bg']),
    input: colorPercent(['--input-bg']),
    panel,
    skin,
    modal,
  }
}

/* ── 不透明度滑块行：range + 百分比数值 ── */
function OpacitySliderRow({
  label,
  value,
  onChange,
  min = 20,
}: {
  label: string
  value: number
  onChange: (percent: number) => void
  min?: number
}) {
  return (
    <FormRow
      label={label}
      control={
        <span className="opacity-control">
          <input
            type="range"
            className="opacity-range"
            min={min}
            max={100}
            step={5}
            value={value}
            onChange={e => onChange(Number(e.target.value))}
            aria-label={label}
          />
          <span className="opacity-value">{value}%</span>
        </span>
      }
    />
  )
}

export function ThemesPage({ onClose, showToast }: { onClose: () => void; showToast: ToastFn }) {
  const {
    theme,
    setTheme,
    customTheme,
    customThemes,
    activeCustomId,
    previewOverrides,
    applyCustomPreview,
    clearCustomPreview,
    saveCustom,
    activateCustom,
    deleteCustom,
    clearCustom,
  } = useTheme()
  const { t, setLang } = useLanguage()
  const [language, setLanguage] = useState('zh')
  const [showAvatar, setShowAvatar] = useState(false)
  const [skinBg, setSkinBg] = useState('')
  /**
   * 皮肤预览用的**可渲染 URL**（`resolveSkinImageUrl` 的结果），不是持久化路径。
   *
   * 预览区必须显示"应用里实际生效的那张图"，而生效值走的是 asset:// 或 blob:
   * （见 assetUrl.ts），裸路径直接塞进 `backgroundImage` 必然什么都不显示 ——
   * 这正是 814 行那个坏点。这里与应用层共用同一个解析函数，预览和真实背景同源。
   */
  const [skinPreviewUrl, setSkinPreviewUrl] = useState('')
  const [userAvatar, setUserAvatar] = useState('')
  const [nuphusAvatar, setNuphusAvatar] = useState('')

  /**
   * 预览 URL 变更订阅：与应用层共用同一份解析结果（`skinBg` 广播），
   * 而不是各自再解析一遍 —— 两处解析必然漂移（预览显示一张、应用里是另一张）。
   */
  const skinPreviewUrlRef = useRef<string | null>(null)
  useEffect(() => {
    const onSkinChange = (e: Event) => {
      const next = (e as CustomEvent<{ url: string | null }>).detail?.url ?? null
      const prev = skinPreviewUrlRef.current
      if (prev === next) return
      skinPreviewUrlRef.current = next
      setSkinPreviewUrl(next ?? '')
      // 旧值是 blob URL → 释放（生命周期契约见 assetUrl.ts）
      releaseSkinBg(prev)
    }
    window.addEventListener(SKIN_CHANGE_EVENT, onSkinChange)
    return () => {
      window.removeEventListener(SKIN_CHANGE_EVENT, onSkinChange)
      releaseSkinBg(skinPreviewUrlRef.current)
      skinPreviewUrlRef.current = null
    }
  }, [])

  /* ── 自定义主题 ── */
  const [customName, setCustomName] = useState(
    () => customTheme?.name || t('themes.customDefaultName'),
  )
  // 激活主题切换时名称输入框跟随（改名编辑从当前主题名开始）
  useEffect(() => {
    setCustomName(
      customThemes.find(x => x.id === activeCustomId)?.name || t('themes.customDefaultName'),
    )
  }, [activeCustomId])
  // 当前生效的覆盖（实时预览优先，其次已保存自定义，最后无覆盖）
  const activeOverrides = previewOverrides ?? customTheme?.overrides ?? EMPTY_OVERRIDES
  // 无覆盖时各核心 token 的基底有效值（读自 computed style，避免与 tokens.css 重复维护）
  const [baseDefaults, setBaseDefaults] = useState<Record<string, string>>({})
  // 五个不透明度滑块的当前值（%）；覆盖存在时由 rgba/数值反向解析，无覆盖时回退 computed α
  const [opacityAlphas, setOpacityAlphas] = useState<OpacityAlphas>({
    bubbles: 100,
    input: 100,
    panel: 80,
    skin: 35,
    modal: 80,
  })

  // 基底变化 / 覆盖变化后，重新读取 5 个核心 token 的当前有效值。
  // ThemeProvider（父级）的 effect 先于本 effect 执行，此处读到的是已应用后的值。
  useEffect(() => {
    const cs = getComputedStyle(document.documentElement)
    const next: Record<string, string> = {}
    for (const key of CORE_TOKEN_KEYS) {
      const raw = cs.getPropertyValue(key).trim()
      next[key] = normalizeHexInput(raw) ?? raw
    }
    setBaseDefaults(next)
  }, [theme, activeOverrides])

  // 滑块状态反向初始化：从 overrides 解析 α；无覆盖 → 颜色 100%、皮肤读当前计算值。
  // 覆盖来自拖动（本组件写入）、导入 JSON、恢复默认等路径，统一在此回填。
  useEffect(() => {
    setOpacityAlphas(readOpacityAlphas(activeOverrides, readSkinOpacityPercent()))
  }, [theme, activeOverrides])

  const effectiveValue = (key: CoreTokenKey): string =>
    activeOverrides[key] ?? baseDefaults[key] ?? '#000000'

  // 有自定义激活（保存或实时预览有实际覆盖）时，内置主题区顶部显示标识
  const hasCustomApplied =
    customTheme !== null || (previewOverrides !== null && Object.keys(previewOverrides).length > 0)

  const buildTheme = (
    base: ThemeId,
    overrides: Record<string, string>,
    nameOverride?: string,
  ): CustomTheme => ({
    name: (nameOverride ?? customName).trim() || t('themes.customDefaultName'),
    base,
    overrides,
  })

  // 基底切换后的第二阶段：新基底颜色已写入 DOM（旧派生色已被上一步剥离），
  // 此时按当前 α 重新派生 rgba 覆盖。用 ref 记住已处理过的基底，避免拖动等渲染重复执行。
  //
  // dirty 语义：只有用户**显式拖过**的滑块通道才在基底切换后重派覆盖。
  // 若不做这层约束，切换内置主题卡片就会因「旧基底滑块读数 ≠ 新基底默认 α」
  // 误写 --modal-bg 覆盖 → 预览态/落盘被标记为「用户自定义」且刷新后回不来纯主题。
  //
  // dirty 的**来源必须是持久的**（localStorage，见 customTheme.readOpacityIntent）：
  // 本组件被 CompactModal 包裹，弹窗关闭即卸载，组件内 ref 归零；若以 ref 为准，
  // 用户「拖滑块 → 关弹窗 → 重开 → 切主题」这条最常见的路径会让 dirty 判为 false，
  // 于是 --input-bg 等派生色被 strip 后不再重派 → 回落基底实色，
  // 用户设定的透明度被静默丢弃（滑块读数仍显示旧值）。这是输入框/气泡不跟透明度
  // 变化的确定性根因。
  const lastDerivedBaseRef = useRef(theme)
  const dirtyOpacityRef = useRef<Record<OpacityChannel, boolean>>(readOpacityIntent())
  useEffect(() => {
    if (lastDerivedBaseRef.current === theme) return
    lastDerivedBaseRef.current = theme
    const dirty = dirtyOpacityRef.current
    const { bubbles, input, panel, modal } = opacityAlphas
    const cs = getComputedStyle(document.documentElement)
    const colors: Record<string, { r: number; g: number; b: number } | null> = {}
    for (const key of OPACITY_COLOR_KEYS) {
      colors[key] = parseColorValue(cs.getPropertyValue(key).trim())
    }
    const user = colors['--msg-user-bg']
    const assistant = colors['--msg-assistant-bg']
    const inputColor = colors['--input-bg']
    const panelColor = colors['--panel-bg']
    const modalColor = colors['--modal-bg']
    const next = stripOpacityColorKeys(activeOverrides)
    if (dirty.bubbles && bubbles < 100 && user && assistant) {
      next['--msg-user-bg'] = toRgba(user, bubbles / 100)
      next['--msg-assistant-bg'] = toRgba(assistant, bubbles / 100)
    }
    if (dirty.input && input < 100 && inputColor) {
      next['--input-bg'] = toRgba(inputColor, input / 100)
    }
    // 面板与弹窗同族：基底即半透明玻璃，仅当滑块值 ≠ 新基底默认 α 时才写覆盖
    // （100% 写 α=1 实色，否则回落基底仍是玻璃）
    if (dirty.panel && panelColor) {
      const computedPct = Math.round(
        (parseColorAlpha(cs.getPropertyValue('--panel-bg').trim()) ?? 0.8) * 100,
      )
      if (panel !== computedPct) {
        next['--panel-bg'] = toRgba(panelColor, panel / 100)
      }
    }
    if (dirty.modal && modalColor) {
      const computedPct = Math.round(
        (parseColorAlpha(cs.getPropertyValue('--modal-bg').trim()) ?? 0.8) * 100,
      )
      if (modal !== computedPct) {
        next['--modal-bg'] = toRgba(modalColor, modal / 100)
      }
    }
    // 基底已生效：滑块读数按新基底重读显示。
    // dirty **不清零** —— 用户「拖过这个通道」是一个持久的用户声明（已落 localStorage），
    // 后续每次切基底都应继续按新基底重派，而不是只生效一次。
    setOpacityAlphas(readOpacityAlphas(next, readSkinOpacityPercent()))
    // 无任何 dirty 通道 → 用户只是切换内置主题：不产生预览/落盘，保持纯主题
    const hasDerived =
      next['--msg-user-bg'] !== undefined ||
      next['--input-bg'] !== undefined ||
      next['--panel-bg'] !== undefined ||
      next['--modal-bg'] !== undefined
    if (!hasDerived && Object.keys(next).length === 0) return
    const nextTheme = buildTheme(theme, next)
    applyCustomPreview(nextTheme)
  }, [theme, opacityAlphas, activeOverrides, customTheme])

  const handleColorCommit = (key: CoreTokenKey, hex: string) => {
    const next = applyCoreColor(activeOverrides, key, hex)
    const nextTheme = buildTheme(theme, next)
    applyCustomPreview(nextTheme)
  }

  /* ── 不透明度滑块 ── */
  // 取当前生效色（computed style，兼容 hex/rgb/rgba）→ 转 rgba(color, α) 写入覆盖通道。
  // 拖动时读到的 rgb 即基底色（α 不改变 rgb 分量），后续拖动可继续以此派生。
  //
  // 每个 handler 都调 markOpacityIntent：把「用户拖过这个通道」落成持久声明。
  // 只写组件内 ref 是不够的 —— 弹窗关一次就归零，切主题时该通道的覆盖会被
  // strip 掉且不再重派（详见 dirtyOpacityRef 处注释）。mark 写 ref 供本次会话立即生效，
  // 写 localStorage 供重开 / 重启后仍然生效。
  const markDirty = (channel: OpacityChannel) => {
    dirtyOpacityRef.current[channel] = true
    markOpacityIntent(channel)
  }

  const handleBubbleOpacity = (percent: number) => {
    markDirty('bubbles')
    setOpacityAlphas(prev => ({ ...prev, bubbles: percent }))
    const cs = getComputedStyle(document.documentElement)
    const user = parseColorValue(cs.getPropertyValue('--msg-user-bg').trim())
    const assistant = parseColorValue(cs.getPropertyValue('--msg-assistant-bg').trim())
    const next = { ...activeOverrides }
    if (percent >= 100) {
      delete next['--msg-user-bg']
      delete next['--msg-assistant-bg']
    } else if (user && assistant) {
      next['--msg-user-bg'] = toRgba(user, percent / 100)
      next['--msg-assistant-bg'] = toRgba(assistant, percent / 100)
    }
    const nextTheme = buildTheme(theme, next)
    applyCustomPreview(nextTheme)
  }

  const handleInputOpacity = (percent: number) => {
    markDirty('input')
    setOpacityAlphas(prev => ({ ...prev, input: percent }))
    const cs = getComputedStyle(document.documentElement)
    const inputColor = parseColorValue(cs.getPropertyValue('--input-bg').trim())
    const next = { ...activeOverrides }
    if (percent >= 100) delete next['--input-bg']
    else if (inputColor) next['--input-bg'] = toRgba(inputColor, percent / 100)
    const nextTheme = buildTheme(theme, next)
    applyCustomPreview(nextTheme)
  }

  // 设置中心面板：语义上属弹窗族（--panel-bg），与弹窗同规则 ——
  // 基底即半透明玻璃，100% 需写 α=1 实色覆盖（不能 delete 回落基底，否则仍是玻璃）
  const handlePanelOpacity = (percent: number) => {
    markDirty('panel')
    setOpacityAlphas(prev => ({ ...prev, panel: percent }))
    const cs = getComputedStyle(document.documentElement)
    const panelColor = parseColorValue(cs.getPropertyValue('--panel-bg').trim())
    const next = { ...activeOverrides }
    if (panelColor) next['--panel-bg'] = toRgba(panelColor, percent / 100)
    const nextTheme = buildTheme(theme, next)
    applyCustomPreview(nextTheme)
  }

  const handleModalOpacity = (percent: number) => {
    markDirty('modal')
    setOpacityAlphas(prev => ({ ...prev, modal: percent }))
    const cs = getComputedStyle(document.documentElement)
    const modalColor = parseColorValue(cs.getPropertyValue('--modal-bg').trim())
    const next = { ...activeOverrides }
    // 弹窗基底即半透明玻璃，100% 需写 α=1 实色覆盖（不能 delete 回落基底，否则仍是玻璃）
    if (modalColor) next['--modal-bg'] = toRgba(modalColor, percent / 100)
    const nextTheme = buildTheme(theme, next)
    applyCustomPreview(nextTheme)
  }

  // 皮肤背景图为数值直写（非 rgba 派生），0% 即隐藏背景图
  const handleSkinOpacity = (percent: number) => {
    markDirty('skin')
    setOpacityAlphas(prev => ({ ...prev, skin: percent }))
    const next = { ...activeOverrides }
    next[SKIN_OPACITY_KEY] = String(percent / 100)
    const nextTheme = buildTheme(theme, next)
    applyCustomPreview(nextTheme)
  }

  const handleCustomNameChange = (value: string) => {
    setCustomName(value)
    // 改名也只进草稿（预览态），不落盘——保存按钮统一收口
  }

  // 预览草稿 = 未保存修改（编辑一律先预览不落盘；保存按钮才持久化——
  // 修复「已激活自定义主题时编辑直接落盘，下次启动仍是草稿」）
  const hasUnsavedPreview = previewOverrides !== null && Object.keys(previewOverrides).length > 0

  const handleCustomSave = () => {
    // 编辑激活主题（同基底）→ 更新该条目；否则新建条目（大王：每个主题有名字，可区分）
    const editingActive = customTheme !== null && customTheme.base === theme
    saveCustom({
      ...buildTheme(theme, activeOverrides),
      id: editingActive ? customTheme.id : undefined,
    })
    showToast(t('themes.customSaved'), 'success')
  }

  /** 激活「我的主题」列表中的某个主题；有未保存草稿时先确认放弃 */
  const handleActivateCustom = (ct: CustomTheme) => {
    if (!ct.id) return
    if (ct.id === activeCustomId && !hasUnsavedPreview) return
    if (hasUnsavedPreview && !window.confirm(t('themes.customSwitchConfirm'))) return
    activateCustom(ct.id)
    setCustomName(ct.name || t('themes.customDefaultName'))
  }

  /** 删除「我的主题」条目 */
  const handleDeleteCustom = (id: string) => {
    if (!window.confirm(t('themes.customDeleteConfirm'))) return
    deleteCustom(id)
    showToast(t('themes.customDeleted'), 'info')
  }

  /** 放弃未保存修改：清预览覆盖，回到已保存主题 / 纯内置基底 */
  const handleDiscardPreview = () => {
    clearCustomPreview()
    showToast(t('themes.customDiscardDone'), 'info')
  }

  const handleCustomReset = () => {
    clearCustom()
    setCustomName(t('themes.customDefaultName'))
    showToast(t('themes.customResetDone'), 'success')
  }

  const handleCustomExport = () => {
    try {
      const data = JSON.stringify(buildTheme(theme, activeOverrides), null, 2)
      const blob = new Blob([data], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      const safe = customName.trim().replace(/[\\/:*?"<>|]/g, '-') || 'custom-theme'
      a.download = `${safe}.json`
      a.click()
      URL.revokeObjectURL(url)
    } catch {
      showToast(t('themes.customExportFail'), 'error')
    }
  }

  const handleCustomImport = () => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.json,application/json'
    input.onchange = e => {
      const file = (e.target as HTMLInputElement).files?.[0]
      if (!file) return
      const reader = new FileReader()
      reader.onload = ev => {
        const text = String(ev.target?.result ?? '')
        const res = parseCustomThemeJSON(text)
        if (!res.ok) {
          showToast(
            res.reason === 'invalid-json'
              ? t('themes.customImportErrJson')
              : t('themes.customImportErrStructure'),
            'error',
          )
          return
        }
        const imported = {
          ...res.theme,
          name: res.theme.name || t('themes.customDefaultName'),
        }
        setCustomName(imported.name)
        saveCustom(imported)
        showToast(t('themes.customImported'), 'success')
      }
      reader.readAsText(file)
    }
    input.click()
  }

  useEffect(() => {
    // 优先读取后端语言设置，兜底 localStorage（后端返回 zh-CN/en-US，归一化到 zh/en）
    getLanguage()
      .then(backendLang => {
        const raw = backendLang || localStorage.getItem(LS_LANG) || 'zh'
        const lang = raw.startsWith('zh') ? 'zh' : 'en'
        setLanguage(lang)
      })
      .catch(() => {
        const raw = localStorage.getItem(LS_LANG) || 'zh'
        setLanguage(raw.startsWith('zh') ? 'zh' : 'en')
      })
    setShowAvatar(localStorage.getItem(LS_SHOW_AVATAR) === 'true')
    // 只恢复本页预览所需的 state；背景广播的恢复由 App 层挂载时统一负责
    // （本页关闭态不在组件树上，不能承担恢复职责 —— 见 ui/skinBg.ts）
    setSkinBg(readSkinBg())
    // 预览区同样在挂载时同步读一次：App 层已广播过的那一次本组件多半没听到
    // （本页按需挂载，晚于 App 层 effect），只订阅事件会一直空到用户重新选图。
    void applySkinBg(readSkinBg())
    setUserAvatar(localStorage.getItem(LS_USER_AVATAR) || '')
    setNuphusAvatar(localStorage.getItem(LS_NUPHUS_AVATAR) || '')
  }, [])

  const handleLang = (id: string) => {
    setLanguage(id)
    setLang(id)
    localStorage.setItem(LS_LANG, id)
    apiSetLanguage(id === 'zh' ? 'zh-CN' : 'en-US')
  }

  /**
   * 选一张本地图片并**入库**（复制进应用数据目录），返回本地方径。
   *
   * 本地应用架构：图片本来就落在本地磁盘上，这里不是「读成 base64 塞进
   * localStorage」，而是把文件复制进 `nuphus_data_dir()/images/` 后只记路径，
   * 渲染侧用 `toAssetUrl(path)` 走 asset:// 读出来。由此同时消掉三件事：
   * localStorage 5MB 配额被大图占满导致后续写入（含主题）静默失败、
   * 数 MB base64 刷新后常驻内存、以及「用户移动/删除原文件 → 背景失效」。
   *
   * 走 `plugin-dialog` 的 open() 而非 `<input type=file>`：后者只给 File 对象，
   * 拿不到可靠的绝对路径（本项目前端历来只用 FileReader 读 dataURL）。
   */
  const pickAndImportImage = async (): Promise<string | null> => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const selected = await open({
      multiple: false,
      filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'] }],
    })
    if (!selected || typeof selected !== 'string') return null
    const { invoke } = await import('@tauri-apps/api/core')
    return await invoke<string>('save_user_image', { sourcePath: selected })
  }

  /**
   * 皮肤背景的应用入口（保存 / 清除 → 本页；恢复 → App 层挂载时）。
   *
   * 实现收敛在 `ui/skinBg.ts`，本页不再持有私有副本：早先「保存 setProperty、
   * 清除 removeProperty、恢复只 setState」三路各写一半，恢复路径还写在了一个
   * 关闭态根本不挂载的组件里（见 skinBg.ts 模块说明）。现在只有一套语义 ——
   * 且写的是「广播可渲染 URL」，不再有把图片字节塞进 CSS 值的路径。
   */
  const handleSkinSelect = async () => {
    try {
      const path = await pickAndImportImage()
      if (!path) return
      setSkinBg(path)
      localStorage.setItem(LS_SKIN, path)
      await applySkinBg(path)
      // 运行时链路与启动路径（App 层 effect）行为不一致时，这一行是唯一能分辨
      // 「卡在入库 / 解析 URL / 还是渲染」的线索。真机复现时看 Console 即可定位。
      console.info('[skin] 已应用背景：', path)
    } catch (e) {
      console.error('皮肤背景入库失败:', e)
      alert(t('themes.skinImportFailed'))
    }
  }

  const clearSkin = () => {
    setSkinBg('')
    localStorage.removeItem(LS_SKIN)
    void applySkinBg('')
  }

  const handleAvatarSelect = async (type: 'user' | 'nuphus') => {
    // 与皮肤背景同一套本地架构：入库到磁盘、只存路径（见 pickAndImportImage）
    try {
      const path = await pickAndImportImage()
      if (!path) return
      const key = type === 'user' ? LS_USER_AVATAR : LS_NUPHUS_AVATAR
      const setter = type === 'user' ? setUserAvatar : setNuphusAvatar
      setter(path)
      localStorage.setItem(key, path)
    } catch (e) {
      console.error('头像入库失败:', e)
      alert(t('themes.avatarImportFailed'))
    }
  }

  const clearAvatar = (type: 'user' | 'nuphus') => {
    const key = type === 'user' ? LS_USER_AVATAR : LS_NUPHUS_AVATAR
    ;(type === 'user' ? setUserAvatar : setNuphusAvatar)('')
    localStorage.removeItem(key)
  }

  const handleToggleAvatar = () => {
    const next = !showAvatar
    setShowAvatar(next)
    localStorage.setItem(LS_SHOW_AVATAR, String(next))
  }

  // 默认头像 = 字母头像：用户侧 U、智能体侧 A（LetterAvatar 尺寸即容器尺寸，铺满）。
  // 自定义头像存的是本地方径，经 toAssetUrl 转 asset:// 渲染（存量 dataURL 亦兼容）。
  const renderAvatar = (src: string, fallback: 'user' | 'nuphus') => (
    <span className="avatar-preview">
      {src ? (
        <img src={toAssetUrl(src) ?? undefined} alt="" />
      ) : (
        <LetterAvatar letter={fallback === 'user' ? 'U' : 'A'} size={34} />
      )}
    </span>
  )

  return (
    <div>
      {/* ── 语言（全局偏好，置页面顶部；非主题设置） ── */}
      <Section title={t('themes.language')}>
        <div className="segmented" role="tablist">
          {LANG.map(l => (
            <button
              key={l.id}
              role="tab"
              aria-selected={language === l.id}
              className={`segmented-item ${language === l.id ? 'active' : ''}`}
              onClick={() => handleLang(l.id)}
            >
              {t(l.label)}
            </button>
          ))}
        </div>
      </Section>

      {/* ── 主题外观 ── */}
      <Section
        title={t('themes.appearance')}
        actions={
          hasCustomApplied ? (
            <span className="custom-theme-badge">
              <span className="custom-theme-badge-dot" aria-hidden="true" />
              {t('themes.customBadge')}
            </span>
          ) : undefined
        }
      >
        <div className="compact-option-grid">
          {THEMES.map(th => (
            <button
              key={th.id}
              className={`compact-option-btn ${theme === th.id ? 'active' : ''}`}
              onClick={() => setTheme(th.id)}
            >
              {/* 色板颜色为主题数据（预览内容），非样式硬编码 */}
              <div
                className="theme-swatch"
                style={{ background: th.bg, ['--swatch-accent' as any]: th.accent }}
              />
              <div className="compact-option-label">{t(`theme.${th.id}`)}</div>
              <div className="compact-option-desc">{t(`theme.${th.id}Desc`)}</div>
            </button>
          ))}
        </div>
      </Section>

      {/* ── 自定义主题 ── */}
      <Section title={t('themes.custom')} description={t('themes.customDesc')}>
        {/* 基底 = 当前使用的主题（只读跟随，不重复摆卡片再选一遍——大王：用户改
            的肯定是当前基底。要换基底去上方「主题外观」换主题） */}
        <div className="custom-base-label">
          {t('themes.customBaseFollow')}: {t(`theme.${theme}`)}
          {hasUnsavedPreview && (
            <span className="custom-unsaved-badge">{t('themes.customUnsaved')}</span>
          )}
        </div>

        {/* ── 我的主题（已保存列表：名称可区分，点击激活，可删除） ── */}
        {customThemes.length > 0 && (
          <>
            <div className="custom-base-label">{t('themes.myThemes')}</div>
            <div className="my-themes-list">
              {customThemes.map(ct => {
                const swatchBg =
                  ct.overrides['--surface-0'] ?? THEMES.find(x => x.id === ct.base)?.bg ?? '#12121a'
                const swatchAccent =
                  ct.overrides['--accent'] ??
                  THEMES.find(x => x.id === ct.base)?.accent ??
                  '#3b82f6'
                return (
                  <div
                    key={ct.id}
                    className={`my-theme-card ${ct.id === activeCustomId ? 'active' : ''}`}
                  >
                    <button
                      type="button"
                      className="my-theme-main"
                      onClick={() => handleActivateCustom(ct)}
                      title={t('themes.myThemeActivate')}
                    >
                      <span
                        className="theme-swatch my-theme-swatch"
                        style={{
                          background: swatchBg,
                          ['--swatch-accent' as string]: swatchAccent,
                        }}
                      />
                      <span className="my-theme-name">
                        {ct.name || t('themes.customDefaultName')}
                      </span>
                      <span className="my-theme-base">{t(`theme.${ct.base}`)}</span>
                    </button>
                    <button
                      type="button"
                      className="my-theme-delete"
                      aria-label={t('themes.myThemeDelete')}
                      onClick={() => ct.id && handleDeleteCustom(ct.id)}
                    >
                      <X size={13} />
                    </button>
                  </div>
                )
              })}
            </div>
          </>
        )}

        <FormRow
          label={t('themes.customName')}
          control={
            <input
              type="text"
              className="input custom-name-input"
              value={customName}
              maxLength={40}
              onChange={e => handleCustomNameChange(e.target.value)}
              spellCheck={false}
            />
          }
        />

        {CORE_TOKEN_KEYS.map(key => (
          <ColorTokenRow
            key={key}
            tokenKey={key}
            label={t(TOKEN_LABELS[key])}
            value={effectiveValue(key)}
            onCommit={handleColorCommit}
          />
        ))}

        <div className="custom-base-label">{t('themes.opacity')}</div>
        <OpacitySliderRow
          label={t('themes.opacityBubbles')}
          value={opacityAlphas.bubbles}
          onChange={handleBubbleOpacity}
        />
        <OpacitySliderRow
          label={t('themes.opacityInput')}
          value={opacityAlphas.input}
          onChange={handleInputOpacity}
        />
        <OpacitySliderRow
          label={t('themes.opacityPanel')}
          value={opacityAlphas.panel}
          onChange={handlePanelOpacity}
        />
        <OpacitySliderRow
          label={t('themes.opacityModal')}
          value={opacityAlphas.modal}
          onChange={handleModalOpacity}
        />
        <OpacitySliderRow
          label={t('themes.opacitySkin')}
          value={opacityAlphas.skin}
          min={0}
          onChange={handleSkinOpacity}
        />

        {/* ── 皮肤背景（自定义主题内：背景是主题外观的一部分）── */}
        <div className="custom-base-label">{t('themes.skinBg')}</div>
        {skinBg && (
          /* 预览图为用户上传数据（动态值），保留内联。
             值必须是 `resolveSkinImageUrl` 解析出的**可渲染 URL**（asset:// 或 blob:），
             并包成 `url(...)` —— 早先传的是裸文件路径，CSS 把它当无效值整条丢弃，
             预览区必然空白（2026-09-26 修复）。 */
          <div
            className="skin-preview"
            style={skinPreviewUrl ? { backgroundImage: `url("${skinPreviewUrl}")` } : undefined}
          >
            <div className="skin-preview-overlay">
              <span className="skin-preview-badge">{t('themes.applied')}</span>
            </div>
          </div>
        )}
        <div className="btn-row">
          <Button variant="default" size="sm" onClick={handleSkinSelect}>
            {skinBg ? t('themes.changeBg') : t('themes.selectBg')}
          </Button>
          {skinBg && (
            <Button variant="danger" size="sm" onClick={clearSkin}>
              {t('themes.clearBg')}
            </Button>
          )}
        </div>

        {/* ── 头像（自定义主题内）── */}
        <div className="custom-base-label">{t('themes.avatarSettings')}</div>
        <FormRow
          label={t('themes.showAvatar')}
          control={
            <button
              type="button"
              role="switch"
              aria-checked={showAvatar}
              className="switch"
              onClick={handleToggleAvatar}
            />
          }
        />
        <FormRow
          label={
            <span className="avatar-label">
              {renderAvatar(userAvatar, 'user')}
              <span>{t('themes.userAvatar')}</span>
            </span>
          }
          control={
            <>
              <Button variant="default" size="sm" onClick={() => handleAvatarSelect('user')}>
                {userAvatar ? t('themes.changeBg') : t('themes.upload')}
              </Button>
              {userAvatar && (
                <Button variant="danger" size="sm" onClick={() => clearAvatar('user')}>
                  {t('themes.clearBg')}
                </Button>
              )}
            </>
          }
        />
        <FormRow
          label={
            <span className="avatar-label">
              {renderAvatar(nuphusAvatar, 'nuphus')}
              <span>{t('themes.nuphusAvatar')}</span>
            </span>
          }
          control={
            <>
              <Button variant="default" size="sm" onClick={() => handleAvatarSelect('nuphus')}>
                {nuphusAvatar ? '更换' : '上传'}
              </Button>
              {nuphusAvatar && (
                <Button variant="danger" size="sm" onClick={() => clearAvatar('nuphus')}>
                  清除
                </Button>
              )}
            </>
          }
        />

        <div className="btn-row custom-actions">
          <Button variant="primary" size="sm" icon={<Save size={14} />} onClick={handleCustomSave}>
            {t('themes.customSave')}
          </Button>
          {hasUnsavedPreview && (
            <Button
              variant="default"
              size="sm"
              icon={<X size={14} />}
              onClick={handleDiscardPreview}
            >
              {t('themes.customDiscard')}
            </Button>
          )}
          <Button
            variant="default"
            size="sm"
            icon={<RotateCcw size={14} />}
            onClick={handleCustomReset}
          >
            {t('themes.customReset')}
          </Button>
          <Button
            variant="default"
            size="sm"
            icon={<Download size={14} />}
            onClick={handleCustomExport}
          >
            {t('themes.customExport')}
          </Button>
          <Button
            variant="default"
            size="sm"
            icon={<Upload size={14} />}
            onClick={handleCustomImport}
          >
            {t('themes.customImport')}
          </Button>
        </div>
      </Section>
    </div>
  )
}
