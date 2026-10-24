/**
 * AppearancePanel.tsx — 外观浮窗（一页完成：选主题 / 调质感 / 存自定义）
 *
 * 定位：**非模态常驻浮层**，从聊天 header 的调色板按钮展开，盖在主界面之上但不遮罩、
 * 不拦交互（没有全屏遮罩、没有 aria-modal）。它取代了原先 `CompactModal` 里的
 * `ThemesPage` 整页——那份实现的痛点是「主题 token 全局注入即时生效，但模态把主界面
 * 盖死，生效却看不见；关闭即卸载，草稿也跟着丢」。本组件按四条原则重做：
 *
 * ① 打开/关闭不卸载内容（`hidden` 收起而非条件渲染）—— 本地草稿（行内改名输入）
 *    跨开合存活；主题微调本身即改即存（见 ④），不依赖开合；
 * ② **一页流，不翻页**：没有 Tab 并列、没有手风琴折叠、没有并列的条件按钮组
 *    （「全部恢复默认」是全面板唯一的条件按钮，仅存在微调时出现），也没有
 *    「选择页 / 自定义页」两页互切。自上而下按「普通用户四步」排优先级：
 *    系统主题（3 大卡）→ 我的主题（卡片网格）→ 界面不透明度 →
 *    颜色微调 → 紧凑保存行。管理型低频动作（导入 / 导出 / 停用）收进「⋯ 更多」
 *    小菜单——不进主视线，也不单独占一页；
 * ③ 「我的主题」即管理台：换背景（卡面 hover 的「上传」钮）/ 行内改名 / 删除 /
 *    ＋ 新建全在卡片上完成 —— 浮窗内**没有**独立的皮肤背景栏，背景只归主题。
 *    「保存 → 原地看到它（新卡即进入行内改名）」是本浮窗的用户感知闭环。
 * ④ **微调 = 个人偏好层，即改即存**（2026-10 定稿）：5 个不透明度通道 + 5 个核心色
 *    由 ThemeProvider 的持久微调层承载（customTheme.readTweaks / writeTweaks），
 *    每次改动立即落盘；换主题 / 重启 app 都保留，且颜色分量按新基底重新派生。
 *    每行右侧「恢复默认」只清该项，「全部恢复默认」一次清空 —— 「恢复默认」的
 *    实现就是删掉对应键，让基底 CSS 默认值重新生效。
 *
 * 界面语言**不在本浮窗**：2026-09-28 迁入设置中心新增 `language` 分区
 * （pages/LanguagePage.tsx，LangProvider / apiSetLanguage 原链路整体搬迁）。
 *
 * 浮层外形规格（fixed/right20/z60/380px/glass/blur24/radius10/shadow-elevated/
 * apnp-in）见 styles/appearance-panel.css，与 WorkflowTaskPanel 同一浮层家族；
 * 类名独立 `apnp-*`，不复用 `wfst-*`。
 *
 * 数据链路：主题数据仍归 useTheme（列表 / 激活 id / 皮肤快照）；本浮窗只把用户的
 * 微调值交给 provider（setTweakAlpha / setTweakToken），不自持生效态。
 * `CustomTheme.skin`（每主题一张皮肤快照）与 useTheme 激活时的快照恢复一步不变——
 * 边界（LS_SKIN = 系统预设态的背景）见 useTheme.tsx 顶部注释。
 */
import { useEffect, useRef, useState, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import {
  IconCheck,
  IconEdit3,
  IconMoreHorizontal,
  IconPalette,
  IconPlus,
  IconRotateCcw,
  IconSave,
  IconUpload,
  IconX,
} from '../../ui/Icons'
import { useTheme } from '../../hooks/useTheme'
import type { ThemeId } from '../../hooks/useTheme'
import {
  CORE_TOKEN_KEYS,
  SKIN_OPACITY_KEY,
  isEmptyTweaks,
  newCustomThemeId,
  normalizeHexInput,
  overridesHaveTweak,
  parseColorAlpha,
  parseCustomThemeJSON,
  type CoreTokenKey,
  type CustomTheme,
  type OpacityChannel,
} from '../../hooks/customTheme'
import { Button } from '../../ui/Button'
import { FormRow } from '../../ui/PageLayout'
import { applySkinBg, readSkinBg } from '../../ui/skinBg'
import { releaseSkinImageUrl, resolveSkinThumbnailUrl } from '../../ui/assetUrl'
import { pickAndImportImage } from '../lib/localImage'
import { showAppFeedback } from '../../ui/islandChannel'
import { useLanguage } from '../../locales'
import '../../styles/appearance-panel.css'
// 浮层内部复用既有样式件（theme-swatch / custom-theme-badge / color-token-* /
// opacity-* / segmented / form-row）；磨砂小按钮见 apnp-glass-btn
import '../../styles/themes.css'

/* 主题预览色板数据（展示用主题色，属于内容数据而非样式） */
const THEMES = [
  { id: 'dark' as ThemeId, bg: '#12121a', accent: '#3b82f6' },
  { id: 'light' as ThemeId, bg: '#f0f4f8', accent: '#2563eb' },
  { id: 'fresh' as ThemeId, bg: '#edf5f0', accent: '#0b7a52' },
]

/* 自定义主题可编辑的核心色 token → 人话标签（名称 + 括号内效果说明） */
const TOKEN_LABELS: Record<CoreTokenKey, string> = {
  '--accent': 'themes.tokenAccentHint',
  '--surface-0': 'themes.tokenSurface0Hint',
  '--surface-1': 'themes.tokenSurface1Hint',
  '--fg-1': 'themes.tokenFg1Hint',
  '--fg-2': 'themes.tokenFg2Hint',
}

/** 不透明度通道 → 人话标签（二次确认要指名道姓说改的是哪一项） */
const OPACITY_LABELS: Record<OpacityChannel, string> = {
  bubbles: 'themes.opacityBubbles',
  input: 'themes.opacityInput',
  panel: 'themes.opacityPanel',
  modal: 'themes.opacityModal',
  skin: 'themes.opacitySkin',
}

type ToastFn = (message: string, type?: 'info' | 'success' | 'warning' | 'error') => void

/**
 * 浮窗自带轻提示通道：直接走 `showAppFeedback`（前台 island / 后台 HUD 分流），
 * 与 App 层 `useInit.showToast` 同一个入口（见 ui/islandChannel.ts 模块说明）。
 * 这样浮窗不必为一条 toast 从 App 层层传 prop 进来。
 */
const showToast: ToastFn = (message, type = 'info') => showAppFeedback(message, type)

/* ── 核心色 token 行：color picker + 可手改 hex ── */
function ColorTokenRow({
  tokenKey,
  label,
  value,
  onCommit,
  onReset,
  modified,
  resetLabel,
}: {
  tokenKey: CoreTokenKey
  label: string
  value: string
  /** 该 token 是否被微调过（为 false 时「恢复默认」不可点） */
  modified: boolean
  onCommit: (key: CoreTokenKey, hex: string) => void
  onReset: (key: CoreTokenKey) => void
  resetLabel: string
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
      label={<span className="color-token-label">{label}</span>}
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
          <button
            type="button"
            className="tweak-reset-btn"
            onClick={() => onReset(tokenKey)}
            disabled={!modified}
            aria-label={`${label} ${resetLabel}`}
            title={resetLabel}
          >
            <IconRotateCcw size={12} />
          </button>
        </span>
      }
    />
  )
}

/* ── 主题卡缩略图（卡面）──
   有皮肤快照 → 显示该主题皮肤的**缩小版**；无皮肤 → 用该主题色板兜底
   （overrides['--surface-0']，缺失回落内置基底 bg）。
   ⚠️ 卡面绝不渲染原图：皮肤图是用户选的全屏图（数 MB / 4K），卡面只有 ~170×64。
   缩小版由前端「取字节 → 同源 blob → canvas 缩到 320px 内 → JPEG 重编码」产出
   （见 ui/assetUrl.ts 的 resolveSkinThumbnailUrl，含 jsdom 测试替身注入点）；
   全局背景（SkinBackdrop）仍然用原图，那才是要铺满全屏的那张。
   blob: 的生命周期按 assetUrl.ts 的调用方契约释放（替换时放旧的、卸载时放当前的）。 */
function ThemeSkinThumb({
  skin,
  fallbackBg,
  accent,
}: {
  skin?: string
  fallbackBg: string
  accent: string
}) {
  const [url, setUrl] = useState('')
  const urlRef = useRef<string | null>(null)
  useEffect(() => {
    let stale = false
    void resolveSkinThumbnailUrl(skin).then(next => {
      if (stale) {
        releaseSkinImageUrl(next)
        return
      }
      const prev = urlRef.current
      urlRef.current = next
      setUrl(next ?? '')
      if (prev) releaseSkinImageUrl(prev)
    })
    return () => {
      stale = true
    }
  }, [skin])
  // 卸载释放（asset:/data: 原样返回，revoke 对它们是 no-op）
  useEffect(
    () => () => {
      if (urlRef.current) {
        releaseSkinImageUrl(urlRef.current)
        urlRef.current = null
      }
    },
    [],
  )
  return (
    <span
      className={`theme-swatch apnp-theme-thumb${url ? ' apnp-theme-thumb--img' : ''}`}
      style={
        url
          ? { backgroundImage: `url("${url}")` }
          : { background: fallbackBg, ['--swatch-accent' as string]: accent }
      }
    />
  )
}

/* ── 行内改名输入框（卡底铅笔点开）──
   回车 / 失焦落盘（由调用方决定写不写），Esc 取消；挂载即聚焦并全选，
   预填的默认名（未命名主题）可被直接键入覆盖。 */
function RenameInput({
  value,
  ariaLabel,
  onChange,
  onCommit,
  onCancel,
}: {
  value: string
  ariaLabel: string
  onChange: (value: string) => void
  onCommit: () => void
  onCancel: () => void
}) {
  const ref = useRef<HTMLInputElement>(null)
  useEffect(() => {
    ref.current?.focus()
    ref.current?.select()
  }, [])
  return (
    <input
      ref={ref}
      type="text"
      className="input apnp-theme-rename"
      value={value}
      maxLength={40}
      onChange={e => onChange(e.target.value)}
      onBlur={onCommit}
      onKeyDown={e => {
        if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
        else if (e.key === 'Escape') onCancel()
      }}
      spellCheck={false}
      aria-label={ariaLabel}
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

/* ── 不透明度滑块行：range + 百分比数值 + 恢复默认 ──
   `label` 直接就是**被调整的对象名**（消息气泡 / 输入框 / 控制面板 / 弹窗 /
   背景图），不再挂「影响对象」小注：label 与 note 早先互为同义反复
   （「气泡不透明度 · 消息气泡」），对用户是纯噪音。 */
function OpacitySliderRow({
  label,
  value,
  onChange,
  onReset,
  modified,
  resetLabel,
  min = 20,
}: {
  label: string
  value: number
  onChange: (percent: number) => void
  /** 恢复该通道默认（删掉微调项，回落基底默认值） */
  onReset: () => void
  /** 该通道是否被微调过（为 false 时「恢复默认」不可点） */
  modified: boolean
  resetLabel: string
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
            step={1}
            value={value}
            onChange={e => onChange(Number(e.target.value))}
            aria-label={label}
          />
          <span className="opacity-value">{value}%</span>
          <button
            type="button"
            className="tweak-reset-btn"
            onClick={onReset}
            disabled={!modified}
            aria-label={`${label} ${resetLabel}`}
            title={resetLabel}
          >
            <IconRotateCcw size={12} />
          </button>
        </span>
      }
    />
  )
}

export interface AppearancePanelProps {
  /** 浮窗是否展开。收起 ≠ 卸载：内容与本地 state 常驻保活（见 .apnp-panel[hidden]） */
  open: boolean
  /** 收起浮窗（header X / Esc / 点面板外） */
  onClose: () => void
  /**
   * 开关按钮 ref（聊天 header 的调色板钮）。点它**不算**「点面板外」——
   * 否则 mousedown 先关、随后的 click 再开，用户会看到一次抖动。
   */
  toggleRef?: RefObject<HTMLButtonElement | null>
}

export function AppearancePanel({ open, onClose, toggleRef }: AppearancePanelProps) {
  const {
    theme,
    setTheme,
    customTheme,
    customThemes,
    activeCustomId,
    tweaks,
    resolvedOverrides,
    setTweakAlpha,
    setTweakToken,
    resetTweakAlpha,
    resetTweakToken,
    resetTweaks,
    saveCustom,
    activateCustom,
    deleteCustom,
    clearCustom,
  } = useTheme()
  const { t } = useLanguage()
  const panelRef = useRef<HTMLDivElement>(null)
  /**
   * 系统预设态（未激活任何自定义主题）的背景值 = LS_SKIN（只读一次）。
   * 浮窗内**没有**皮肤背景栏：换背景只走「我的主题」卡面 hover 的「上传」钮
   * （写该主题的皮肤快照）。这个值用于：
   * ① 「新建」主题继承当前生效背景；② 回预设态时把它应用回全局（见各 handler）。
   */
  const [presetSkin] = useState(readSkinBg)

  /* ── 自定义主题 ── */
  // 行内改名态：哪张卡在改名 + 输入框草稿（点铅笔开，回车/失焦落盘，Esc 取消）
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState('')
  /** 当前实际生效的覆盖（主题层 + 微调派生，由 ThemeProvider 合成） */
  const activeOverrides = resolvedOverrides
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
  // ThemeProvider（父级）的 layout effect 先于本 effect 执行，此处读到的是已应用后的值。
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
  //
  // deps 必须含 theme（铁律③）：纯净预设态切系统主题时 overrides 不变（同为
  // EMPTY_OVERRIDES），但新基底的 computed α 可能不同 —— 缺 theme 会让滑块读数
  // 停在上一套主题，与画面实际 α 脱钩。
  useEffect(() => {
    setOpacityAlphas(readOpacityAlphas(activeOverrides, readSkinOpacityPercent()))
  }, [activeOverrides, theme])

  const effectiveValue = (key: CoreTokenKey): string =>
    activeOverrides[key] ?? baseDefaults[key] ?? '#000000'

  // 有自定义激活或存在微调时，「我的主题」标题旁显示标识
  const hasCustomApplied = customTheme !== null || !isEmptyTweaks(tweaks)

  /**
   * 当前**生效**的皮肤值（路径）—— 皮肤背景区预览 / 换图 / 清除都对准它：
   * 激活自定义主题 → 该主题的皮肤快照（无皮肤即空，激活时已把全局背景清空）；
   * 未激活（系统预设态）→ LS_SKIN。与「背景实际显示什么」恒等，
   * 边界见 useTheme.tsx 顶部注释。
   */
  const effectiveSkin = customTheme ? (customTheme.skin ?? '') : presetSkin

  /** 「我的主题」网格展示序：最新保存/创建的主题在前（新卡一眼可见、落在网格首位）。
   *  只动展示层，useTheme 里列表的存储顺序不变。 */
  const displayThemes = [...customThemes].reverse()

  /* ── 微调（不透明度 5 通道 + 核心色 5 token）：即改即存 ──
     值写进 useTheme 的持久微调层（localStorage），因此换主题 / 重启 app 都不丢；
     rgba 的 rgb 分量由 ThemeProvider 按「当前基底」统一派生，本组件不自己做
     两阶段重派（旧实现把派生色塞进会话级预览通道，重启/切主题即丢）。 */
  const handleColorCommit = (key: CoreTokenKey, hex: string) => setTweakToken(key, hex)

  const handleBubbleOpacity = (percent: number) => setTweakAlpha('bubbles', percent)
  const handleInputOpacity = (percent: number) => setTweakAlpha('input', percent)
  const handlePanelOpacity = (percent: number) => setTweakAlpha('panel', percent)
  const handleModalOpacity = (percent: number) => setTweakAlpha('modal', percent)
  const handleSkinOpacity = (percent: number) => setTweakAlpha('skin', percent)

  /** 单参数是否被设置过（决定该行「恢复默认」是否可点）：
   *  微调层里有该项，或当前自定义主题快照里烘着该项 */
  const isAlphaModified = (channel: OpacityChannel): boolean =>
    tweaks.alphas[channel] !== undefined || overridesHaveTweak(customTheme?.overrides, { channel })
  const isTokenModified = (key: CoreTokenKey): boolean =>
    tweaks.tokens[key] !== undefined || overridesHaveTweak(customTheme?.overrides, { token: key })

  /** 当前主题是否有任何可恢复的设置（微调层 或 主题快照里烘着的值） */
  const themeHasBakedTweaks =
    customTheme !== null &&
    ((['bubbles', 'input', 'panel', 'modal', 'skin'] as OpacityChannel[]).some(channel =>
      overridesHaveTweak(customTheme.overrides, { channel }),
    ) ||
      CORE_TOKEN_KEYS.some(key => overridesHaveTweak(customTheme.overrides, { token: key })))
  const hasAnyTweak = !isEmptyTweaks(tweaks) || themeHasBakedTweaks

  /**
   * 「恢复默认」若会**改写用户已保存的自定义主题快照**（该值烘在主题的 overrides 里），
   * 必须先确认 —— 恢复默认不该在用户不知情的情况下修改已保存的数据。
   * 只动未保存的微调（内置主题 / 未烘值的自定义主题）时不打扰：没有已保存数据可改。
   */
  const confirmSavedRewrite = (what: string): boolean => {
    const name = customTheme?.name ?? t('themes.themeUnnamed')
    return window.confirm(t('themes.tweakResetSavedConfirm', name, what))
  }

  /** 单参数恢复默认：删掉该项即回落当前基底默认值 */
  const handleOpacityReset = (channel: OpacityChannel) => {
    if (
      overridesHaveTweak(customTheme?.overrides, { channel }) &&
      !confirmSavedRewrite(t(OPACITY_LABELS[channel]))
    ) {
      return
    }
    resetTweakAlpha(channel)
  }
  const handleColorReset = (key: CoreTokenKey) => {
    if (
      overridesHaveTweak(customTheme?.overrides, { token: key }) &&
      !confirmSavedRewrite(t(TOKEN_LABELS[key]))
    ) {
      return
    }
    resetTweakToken(key)
  }

  /** 全部恢复默认：清空微调层（主题选择保持不变） */
  const handleResetAllTweaks = () => {
    if (themeHasBakedTweaks && !confirmSavedRewrite(t('themes.resetAllItems'))) return
    resetTweaks()
    showToast(t('themes.resetAllDone'), 'success')
  }

  /**
   * 新建链路（「＋ 新建」卡与「保存」的非编辑态共用）：
   * 以「当前基底 + 当前实际生效的微调」建条，皮肤继承当前**生效**的那张图作为
   * 初始快照（用户随后可在卡上改）；新卡落盘即进入行内改名态（预填「未命名主题」）。
   */
  const createCustomFromCurrent = () => {
    const id = newCustomThemeId()
    const name = t('themes.themeUnnamed')
    saveCustom({
      id,
      name,
      base: theme,
      overrides: activeOverrides,
      skin: effectiveSkin || undefined,
    })
    setRenamingId(id)
    setRenameDraft(name)
  }

  const handleCustomSave = () => {
    // 编辑激活主题（同基底）→ 更新该条目：名称与皮肤快照原样保留（改名在卡上进行）；
    // 否则 = 新建链路
    if (customTheme !== null && customTheme.base === theme) {
      saveCustom({ ...customTheme, base: theme, overrides: activeOverrides })
      showToast(t('themes.customSaved'), 'success')
      return
    }
    createCustomFromCurrent()
  }

  /** 激活「我的主题」网格里的某张卡（微调层随并入而非清空，见 useTheme.activateCustom） */
  const handleActivateCustom = (ct: CustomTheme) => {
    if (!ct.id) return
    if (ct.id === activeCustomId) return
    activateCustom(ct.id)
  }

  /** 删除网格里的某个主题（卡片右上小删除钮，confirm 后落盘） */
  const handleDeleteCustom = (id: string) => {
    if (!window.confirm(t('themes.customDeleteConfirm'))) return
    // 删的是激活项 → 回到系统预设态，背景同步回 LS_SKIN（边界见 useTheme.tsx 顶部）
    if (id === activeCustomId) void applySkinBg(readSkinBg())
    deleteCustom(id)
    showToast(t('themes.customDeleted'), 'info')
  }

  /** 开行内改名：名称变输入框，草稿从该主题现名开始 */
  const startRename = (ct: CustomTheme) => {
    setRenamingId(ct.id ?? null)
    setRenameDraft(ct.name || t('themes.customDefaultName'))
  }

  /** 行内改名落盘：回车 / 失焦触发；空输入视为不改名（保留原名，防误清空） */
  const commitRename = (ct: CustomTheme) => {
    if (renamingId !== ct.id) return
    setRenamingId(null)
    const name = renameDraft.trim()
    if (name && name !== ct.name) saveCustom({ ...ct, name })
  }

  const cancelRename = () => setRenamingId(null)

  /**
   * 卡面 hover 的「上传」—— 皮肤背景的**唯一**入口（浮窗内不再有独立的皮肤背景栏：
   * 换哪张卡的背景，就落哪张卡的皮肤快照，全局背景随之切换）。
   * saveCustom 会激活该条，故随后一次 applySkinBg 即让新快照成为全局生效值。
   */
  const handleCardSkinUpload = async (ct: CustomTheme) => {
    try {
      const path = await pickAndImportImage()
      if (!path) return
      saveCustom({ ...ct, skin: path })
      await applySkinBg(path)
      console.info('[skin] 已应用主题皮肤：', path)
    } catch (e) {
      console.error('主题皮肤图入库失败:', e)
      alert(t('themes.skinImportFailed'))
    }
  }

  const handleCustomReset = () => {
    clearCustom()
    // 停用 = 回系统预设态：背景同步回 LS_SKIN（边界见 useTheme.tsx 顶部注释）
    void applySkinBg(readSkinBg())
    showToast(t('themes.customResetDone'), 'success')
  }

  const handleCustomExport = () => {
    try {
      // 导出当前编辑对象：编辑激活主题 → 连 id/名称/皮肤快照一起导出；否则导
      // 「当前主题 + 预览值 + 未命名」的草稿形状（skin 缺省 = 无皮肤）
      const editingActive = customTheme !== null && customTheme.base === theme
      const payload: CustomTheme = editingActive
        ? { ...customTheme, overrides: activeOverrides }
        : {
            name: t('themes.themeUnnamed'),
            base: theme,
            overrides: activeOverrides,
            skin: effectiveSkin || undefined,
          }
      const data = JSON.stringify(payload, null, 2)
      const blob = new Blob([data], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      const safe = payload.name.trim().replace(/[\\/:*?"<>|]/g, '-') || 'custom-theme'
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
        saveCustom(imported)
        // 导入即激活该条：按快照恢复它的皮肤（无皮肤 = 清空），与激活链路同一口径
        void applySkinBg(imported.skin ?? '')
        showToast(t('themes.customImported'), 'success')
      }
      reader.readAsText(file)
    }
    input.click()
  }

  /* ── 「⋯ 更多」小菜单（低频管理动作收纳）──
      导入 JSON / 导出 JSON / 停用自定义都在这里，不占主视线、不单独成页
      （「放弃修改」不在此处：它在紧凑保存行，仅有无保存调整时出现）。
      位置用 **portal + fixed**：保存行在面板底部，向下的浮层必被
      `.apnp-panel` 的 overflow:hidden 裁掉（backdrop-filter 还会让
      fixed 的包含块变成面板自身），放进文档流又会顶动内容。坐标取自触发钮的
      getBoundingClientRect，贴视口边界时翻转。 */
  const [moreOpen, setMoreOpen] = useState(false)
  const [morePos, setMorePos] = useState<{ x: number; y: number } | null>(null)
  const moreWrapRef = useRef<HTMLSpanElement>(null)
  const moreTriggerRef = useRef<HTMLButtonElement>(null)

  const closeMore = () => {
    setMoreOpen(false)
  }

  const toggleMore = () => {
    if (moreOpen) {
      closeMore()
      return
    }
    const rect = moreTriggerRef.current?.getBoundingClientRect()
    if (!rect) return
    const estH = 110 // 三项 + 分隔线的估算高度（用于向下溢出时向上翻转）
    const x = Math.max(8, Math.min(rect.left, window.innerWidth - 168))
    const y =
      rect.bottom + 4 + estH > window.innerHeight
        ? Math.max(8, rect.top - estH - 4)
        : rect.bottom + 4
    setMorePos({ x, y })
    setMoreOpen(true)
  }

  // 点面板内别处 / 滚动 / 窗口尺寸变化 → 收菜单（点在触发钮或菜单内不收）
  useEffect(() => {
    if (!moreOpen) return
    const onDown = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null
      if (moreWrapRef.current?.contains(el)) return
      if (el?.closest('.apnp-more-menu')) return
      closeMore()
    }
    window.addEventListener('mousedown', onDown, true)
    // 滚动会让 fixed 坐标与触发钮错位（面板内滚动同样会上浮到 window capture）
    window.addEventListener('scroll', closeMore, true)
    window.addEventListener('resize', closeMore)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('scroll', closeMore, true)
      window.removeEventListener('resize', closeMore)
    }
  }, [moreOpen])

  /* ── 浮窗开合行为：Esc 收起 + 点面板外收起 ──
     模式同 ui/AppContextMenu.tsx（document 级监听 + 只在展开期挂载，收起即摘）。
     刻意不用全屏透明 backdrop：那会拦掉主界面交互，与「不挡」的定位相反。 */
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // 菜单开着时 Esc 先收菜单，不把整个浮窗一起关掉
        if (moreOpen) {
          closeMore()
          return
        }
        onClose()
      }
    }
    const onDown = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null
      // 面板内交互不判外点；开关按钮也不算（否则 mousedown 先关、click 再开）；
      // 「⋯ 更多」菜单虽是 portal 出去的，逻辑上仍是面板的一部分
      if (panelRef.current?.contains(el)) return
      if (toggleRef?.current?.contains(el)) return
      if (el?.closest('.apnp-more-menu')) return
      onClose()
    }
    document.addEventListener('keydown', onKey)
    window.addEventListener('mousedown', onDown, true)
    return () => {
      document.removeEventListener('keydown', onKey)
      window.removeEventListener('mousedown', onDown, true)
    }
  }, [open, onClose, toggleRef, moreOpen])

  /** 「⋯ 更多」菜单项：点了先收菜单再执行（导入会弹系统选文件框） */
  const runMoreAction = (fn: () => void) => {
    closeMore()
    fn()
  }

  return (
    <div
      ref={panelRef}
      className="apnp-panel"
      /* 收起 = 保活：内容与未保存调整常驻树上，只摘掉浮层自身 */
      hidden={!open}
      /* 非模态浮层：没有全屏遮罩、没有 aria-modal、不拦主界面交互 */
      role="dialog"
      aria-label={t('app.appearance')}
    >
      <header className="apnp-header">
        <span className="apnp-header-icon">
          <IconPalette size={15} />
        </span>
        <span className="apnp-header-title">{t('app.appearance')}</span>
        <button
          type="button"
          className="apnp-close-btn"
          aria-label={t('common.close')}
          title={t('common.close')}
          onClick={onClose}
        >
          <IconX size={14} />
        </button>
      </header>

      <div className="apnp-body">
        {/* ── 一「系统主题」：3 张大卡横排，点击即时全局生效 ── */}
        <section className="apnp-zone">
          <div className="apnp-zone-title">{t('themes.systemThemes')}</div>
          {/* desc 文案在 380px 窄窗放不下 → 并入 title tooltip，卡片只留色板 + 名称 */}
          <div className="apnp-preset-grid">
            {THEMES.map(th => (
              <button
                key={th.id}
                type="button"
                className={`apnp-preset-card ${theme === th.id ? 'active' : ''}`}
                onClick={() => {
                  // 点系统预设卡 = 停用自定义主题（自定义覆盖是主题专属，不跨基底残留）。
                  // 微调层**保留**：它是个人偏好，换主题照旧生效（用户明确要求）。
                  // 若此前挂着自定义主题，背景同步回 LS_SKIN（主题皮肤快照只归
                  // 该主题，边界见 useTheme.tsx 顶部注释）
                  const fromCustom = customTheme !== null
                  setTheme(th.id)
                  if (fromCustom) void applySkinBg(readSkinBg())
                }}
                title={`${t(`theme.${th.id}`)} — ${t(`theme.${th.id}Desc`)}`}
              >
                {/* 色板颜色为主题数据（预览内容），非样式硬编码 */}
                <span
                  className="theme-swatch apnp-preset-swatch"
                  style={{ background: th.bg, ['--swatch-accent' as string]: th.accent }}
                />
                <span className="apnp-preset-name">{t(`theme.${th.id}`)}</span>
                {theme === th.id && (
                  <span className="apnp-preset-check" aria-hidden="true">
                    <IconCheck size={11} />
                  </span>
                )}
              </button>
            ))}
          </div>
        </section>

        {/* ── 二「我的主题」：卡片网格（与系统主题大卡同款式）──
            每张卡：卡面 = 该主题皮肤的缩小版缩略图（无皮肤用该主题色板兜底；
            绝不渲染原图，见 ThemeSkinThumb）；hover 变暗 + 中央浮出「上传」换这张
            卡的背景；卡底 = 主题名 + 铅笔行内改名；右上小钮删除。
            末位固定「＋ 新建」。空态只显示这张新建卡。 */}
        <section className="apnp-zone">
          <div className="apnp-zone-title">
            {t('themes.myThemes')}
            {hasCustomApplied && (
              <span className="custom-theme-badge">
                <span className="custom-theme-badge-dot" aria-hidden="true" />
                {t('themes.customBadge')}
              </span>
            )}
          </div>
          <div className="apnp-theme-grid">
            {displayThemes.map(ct => {
              const swatchBg =
                ct.overrides['--surface-0'] ?? THEMES.find(x => x.id === ct.base)?.bg ?? '#12121a'
              const swatchAccent =
                ct.overrides['--accent'] ?? THEMES.find(x => x.id === ct.base)?.accent ?? '#3b82f6'
              return (
                <div
                  key={ct.id}
                  className={`apnp-theme-card ${ct.id === activeCustomId ? 'active' : ''}`}
                >
                  <span className="apnp-theme-media">
                    <button
                      type="button"
                      className="apnp-theme-face"
                      onClick={() => handleActivateCustom(ct)}
                      title={t('themes.myThemeActivate')}
                    >
                      <ThemeSkinThumb skin={ct.skin} fallbackBg={swatchBg} accent={swatchAccent} />
                    </button>
                    <button
                      type="button"
                      className="apnp-theme-upload"
                      onClick={() => void handleCardSkinUpload(ct)}
                      aria-label={t('themes.upload')}
                      title={t('themes.upload')}
                    >
                      <IconUpload size={12} />
                      <span>{t('themes.upload')}</span>
                    </button>
                    {ct.id === activeCustomId && (
                      <span className="apnp-theme-check" aria-hidden="true">
                        <IconCheck size={11} />
                      </span>
                    )}
                    <button
                      type="button"
                      className="apnp-theme-delete"
                      aria-label={t('themes.myThemeDelete')}
                      onClick={() => ct.id && handleDeleteCustom(ct.id)}
                    >
                      <IconX size={12} />
                    </button>
                  </span>
                  <span className="apnp-theme-foot">
                    {renamingId === ct.id ? (
                      <RenameInput
                        value={renameDraft}
                        ariaLabel={t('themes.themeRename')}
                        onChange={setRenameDraft}
                        onCommit={() => commitRename(ct)}
                        onCancel={cancelRename}
                      />
                    ) : (
                      <span className="apnp-theme-name">
                        {ct.name || t('themes.customDefaultName')}
                      </span>
                    )}
                    <button
                      type="button"
                      className="apnp-theme-edit"
                      aria-label={t('themes.themeRename')}
                      title={t('themes.themeRename')}
                      onClick={() => startRename(ct)}
                    >
                      <IconEdit3 size={12} />
                    </button>
                  </span>
                </div>
              )
            })}
            {/* 末位固定「＋ 新建」：以「当前基底 + 当前生效微调」建新主题，
                落盘即出现在主题卡首位并进入行内改名态 */}
            <button
              type="button"
              className="apnp-theme-card apnp-theme-card-new"
              onClick={createCustomFromCurrent}
              title={t('themes.newTheme')}
            >
              <span className="apnp-theme-new-inner">
                <IconPlus size={18} />
                <span>{t('themes.newTheme')}</span>
              </span>
            </button>
          </div>
        </section>

        {/* ── 三「界面不透明度」：5 个滑块铺开（进到这里就是要调，铺开是诚实）──
            每行右侧「恢复默认」= 删掉该通道的微调项，回落当前基底默认值 */}
        <section className="apnp-zone">
          <div className="apnp-zone-title">{t('themes.opacityZone')}</div>
          <OpacitySliderRow
            label={t('themes.opacityBubbles')}
            value={opacityAlphas.bubbles}
            onChange={handleBubbleOpacity}
            onReset={() => handleOpacityReset('bubbles')}
            modified={isAlphaModified('bubbles')}
            resetLabel={t('themes.tweakReset')}
          />
          <OpacitySliderRow
            label={t('themes.opacityInput')}
            value={opacityAlphas.input}
            onChange={handleInputOpacity}
            onReset={() => handleOpacityReset('input')}
            modified={isAlphaModified('input')}
            resetLabel={t('themes.tweakReset')}
          />
          <OpacitySliderRow
            label={t('themes.opacityPanel')}
            value={opacityAlphas.panel}
            onChange={handlePanelOpacity}
            onReset={() => handleOpacityReset('panel')}
            modified={isAlphaModified('panel')}
            resetLabel={t('themes.tweakReset')}
          />
          <OpacitySliderRow
            label={t('themes.opacityModal')}
            value={opacityAlphas.modal}
            onChange={handleModalOpacity}
            onReset={() => handleOpacityReset('modal')}
            modified={isAlphaModified('modal')}
            resetLabel={t('themes.tweakReset')}
          />
          <OpacitySliderRow
            label={t('themes.opacitySkin')}
            value={opacityAlphas.skin}
            min={0}
            onChange={handleSkinOpacity}
            onReset={() => handleOpacityReset('skin')}
            modified={isAlphaModified('skin')}
            resetLabel={t('themes.tweakReset')}
          />
        </section>

        {/* ── 五「颜色微调」：5 个核心色行（标签 = 名称 + 括号内效果说明）── */}
        <section className="apnp-zone">
          <div className="apnp-zone-title">{t('themes.colorZone')}</div>
          {CORE_TOKEN_KEYS.map(key => (
            <ColorTokenRow
              key={key}
              tokenKey={key}
              label={t(TOKEN_LABELS[key])}
              value={effectiveValue(key)}
              modified={isTokenModified(key)}
              onCommit={handleColorCommit}
              onReset={handleColorReset}
              resetLabel={t('themes.tweakReset')}
            />
          ))}
        </section>

        {/* ── 紧凑保存行：次要在左、主操作「保存」贴最右缘 ──
            「保存」= 把当前样子固化成一个命名主题（微调本身已即时落盘，保存不是必需）。
            「恢复默认」是唯一的条件按钮：仅当存在微调时出现，一次清空全部微调。 */}
        <div className="apnp-save-control">
          {hasAnyTweak && (
            <button type="button" className="apnp-glass-btn" onClick={handleResetAllTweaks}>
              {t('themes.resetAllTweaks')}
            </button>
          )}
          <span className="apnp-more-wrap" ref={moreWrapRef}>
            <button
              ref={moreTriggerRef}
              type="button"
              className="apnp-more-trigger"
              aria-haspopup="menu"
              aria-expanded={moreOpen}
              onClick={toggleMore}
            >
              <IconMoreHorizontal size={14} />
              {t('themes.moreActions')}
            </button>
          </span>
          <Button
            variant="primary"
            size="sm"
            icon={<IconSave size={14} />}
            onClick={handleCustomSave}
          >
            {t('themes.customSave')}
          </Button>
        </div>
      </div>

      {/* 「⋯ 更多」浮层：portal 出去（避开 .apnp-panel 的 overflow:hidden 裁剪），
          逻辑上仍属本浮窗 —— Esc 先收它、面板外点击不算「点面板外」。 */}
      {moreOpen &&
        morePos &&
        createPortal(
          <div className="apnp-more-menu" role="menu" style={{ left: morePos.x, top: morePos.y }}>
            <button
              type="button"
              role="menuitem"
              className="apnp-more-item"
              onClick={() => runMoreAction(handleCustomImport)}
            >
              {t('themes.customImport')}
            </button>
            <button
              type="button"
              role="menuitem"
              className="apnp-more-item"
              onClick={() => runMoreAction(handleCustomExport)}
            >
              {t('themes.customExport')}
            </button>
            <div className="apnp-more-divider" />
            <button
              type="button"
              role="menuitem"
              className="apnp-more-item"
              onClick={() => runMoreAction(handleCustomReset)}
            >
              {t('themes.customReset')}
            </button>
          </div>,
          document.body,
        )}
    </div>
  )
}
