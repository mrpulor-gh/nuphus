/**
 * AppearancePanel.tsx — 外观浮窗（一页完成：选主题 / 调质感 / 存自定义）
 *
 * 定位：**非模态常驻浮层**，从聊天 header 的调色板按钮展开，盖在主界面之上但不遮罩、
 * 不拦交互（没有全屏遮罩、没有 aria-modal）。它取代了原先 `CompactModal` 里的
 * `ThemesPage` 整页——那份实现的痛点是「主题 token 全局注入即时生效，但模态把主界面
 * 盖死，生效却看不见；关闭即卸载，草稿也跟着丢」。本组件按三条原则重做：
 *
 * ① 打开/关闭不卸载内容（`hidden` 收起而非条件渲染）—— 未保存调整跨开合存活；
 * ② **一页流，不翻页**：没有 Tab 并列、没有手风琴折叠、没有并列的条件按钮组
 *    （「放弃修改」是全面板唯一的条件按钮，仅有无保存调整时出现），也没有
 *    「选择页 / 自定义页」两页互切。自上而下按「普通用户四步」排优先级：
 *    系统主题（3 大卡）→ 我的主题（卡片网格）→ 界面不透明度 →
 *    颜色微调 → 紧凑保存行。管理型低频动作（导入 / 导出 / 停用）收进「⋯ 更多」
 *    小菜单——不进主视线，也不单独占一页；
 *    （行内的「恢复默认」小钮 —— 5 个不透明度行 + 5 个颜色行各一个 ——
 *      **常驻渲染**，没有可恢复内容时 disabled，不是条件按钮）
 * ③ 「我的主题」即管理台：换背景（卡面 hover 的「上传」钮）/ 行内改名 / 删除 /
 *    ＋ 新建全在卡片上完成 —— 浮窗内**没有**独立的皮肤背景栏，背景只归主题。
 *    「保存 → 原地看到它（新卡即进入行内改名）」是本浮窗的用户感知闭环。
 *
 * 界面语言**不在本浮窗**：2026-09-28 迁入设置中心新增 `language` 分区
 * （pages/LanguagePage.tsx，LangProvider / apiSetLanguage 原链路整体搬迁）。
 *
 * 浮层外形规格（fixed/right20/z60/380px/glass/blur24/radius10/shadow-elevated/
 * apnp-in）见 styles/appearance-panel.css，与 WorkflowTaskPanel 同一浮层家族；
 * 类名独立 `apnp-*`，不复用 `wfst-*`。
 *
 * 数据链路（customTheme.ts / skinBg.ts / lib/localImage.ts）一行未改：只搬 UI。
 * 唯一的数据模型变更是 `CustomTheme.skin`（每主题一张皮肤快照）与 useTheme 激活时
 * 的快照恢复一步——边界（LS_SKIN = 系统预设态的背景）见 useTheme.tsx 顶部注释。
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
  ACCENT_DERIVED_KEYS,
  CORE_TOKEN_KEYS,
  SKIN_OPACITY_KEY,
  accentOverrideGroup,
  applyCoreColor,
  newCustomThemeId,
  normalizeHexInput,
  parseColorAlpha,
  parseColorValue,
  parseCustomThemeJSON,
  toRgba,
  type CoreTokenKey,
  type CustomTheme,
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

/* 无覆盖时的稳定空对象（避免每次渲染生成新引用触发 effect） */
const EMPTY_OVERRIDES: Record<string, string> = {}

/* ── 行内「恢复默认」的覆盖键分组 ──
   语义严格限定为 **未保存草稿**（previewOverrides）：只回退草稿里对应的覆盖键，
   绝不触碰、不回写用户已保存的自定义主题数据（不调 saveCustom / 不写 localStorage）。

   气泡是两个键（用户气泡 / 助手气泡），**视为一组**一起回退 ——
   只回退一个会让两边气泡透明度劈叉。 */
const OPACITY_RESET_KEYS: Record<
  'bubbles' | 'input' | 'panel' | 'modal' | 'skin',
  readonly string[]
> = {
  bubbles: ['--msg-user-bg', '--msg-assistant-bg'],
  input: ['--input-bg'],
  panel: ['--panel-bg'],
  modal: ['--modal-bg'],
  skin: [SKIN_OPACITY_KEY],
}

/** 颜色行 → 覆盖键组（普通 token 单键；强调色带 4 个派生键）。
 *
 * 强调色必须与派生键同组：applyCoreColor 随主键一起写草稿，只回退主键会把
 * 派生键留在上一版强调色的取值上（主键与派生键脱钩）。清单取自 customTheme，
 * 不另建镜像常量 —— 清单漂移会让恢复只回退一半。 */
function colorResetKeys(key: CoreTokenKey): readonly string[] {
  return key === '--accent' ? ['--accent', ...ACCENT_DERIVED_KEYS] : [key]
}

type ToastFn = (message: string, type?: 'info' | 'success' | 'warning' | 'error') => void

/**
 * 浮窗自带轻提示通道：直接走 `showAppFeedback`（前台 island / 后台 HUD 分流），
 * 与 App 层 `useInit.showToast` 同一个入口（见 ui/islandChannel.ts 模块说明）。
 * 这样浮窗不必为一条 toast 从 App 层层传 prop 进来。
 */
const showToast: ToastFn = (message, type = 'info') => showAppFeedback(message, type)

/* ── 行内「恢复默认」小钮（每个不透明度行 / 每个颜色行各一个）──
   **常驻渲染**：没有可恢复的内容时 disabled，而不是不渲染 ——
   面板形态约束里「放弃修改」是唯一的条件按钮，行内小钮不做条件渲染。

   语义只到「未保存草稿」一层：回退 previewOverrides 里的覆盖键，
   已保存的自定义主题数据一概不碰。 */
function ResetItemButton({
  label,
  canReset,
  onReset,
}: {
  /** 行标签（气泡名 / 颜色名），进无障碍名以便 10 个小钮彼此可区分 */
  label: string
  /** 草稿里该行有值且与基线不同 → 可恢复；否则 disabled */
  canReset: boolean
  onReset: () => void
}) {
  const { t } = useLanguage()
  const title = `${t('themes.resetItem')} · ${label}`
  return (
    <button
      type="button"
      className="apnp-reset-btn"
      aria-label={title}
      title={title}
      disabled={!canReset}
      onClick={onReset}
    >
      <IconRotateCcw size={12} />
    </button>
  )
}

/* ── 核心色 token 行：color picker + 可手改 hex ── */
function ColorTokenRow({
  tokenKey,
  label,
  value,
  onCommit,
  onReset,
  canReset,
}: {
  tokenKey: CoreTokenKey
  label: string
  value: string
  onCommit: (key: CoreTokenKey, hex: string) => void
  /** 行内「恢复默认」：把该行的草稿覆盖回退到基线（不碰已保存数据） */
  onReset: () => void
  /** 草稿里该行有值且与基线不同 → 可恢复；否则小钮 disabled */
  canReset: boolean
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
        <>
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
          <ResetItemButton label={label} canReset={canReset} onReset={onReset} />
        </>
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
  return Number.isFinite(n) ? Math.round(n * 100) : 0
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

/* ── 不透明度滑块行：range + 百分比数值 ──
   `label` 直接就是**被调整的对象名**（消息气泡 / 输入框 / 控制面板 / 弹窗 /
   背景图），不再挂「影响对象」小注：label 与 note 早先互为同义反复
   （「气泡不透明度 · 消息气泡」），对用户是纯噪音。 */
function OpacitySliderRow({
  label,
  value,
  onChange,
  onReset,
  canReset,
  min = 20,
}: {
  label: string
  value: number
  onChange: (percent: number) => void
  /** 行内「恢复默认」：把该通道的草稿覆盖回退到基线（不碰已保存数据） */
  onReset: () => void
  /** 草稿里该通道有值且与基线不同 → 可恢复；否则小钮 disabled */
  canReset: boolean
  min?: number
}) {
  return (
    <FormRow
      label={label}
      control={
        <>
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
          </span>
          <ResetItemButton label={label} canReset={canReset} onReset={onReset} />
        </>
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
    previewOverrides,
    applyCustomPreview,
    clearCustomPreview,
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
  // 当前生效的覆盖（实时预览优先，其次已保存自定义，最后无覆盖）
  const activeOverrides = previewOverrides ?? customTheme?.overrides ?? EMPTY_OVERRIDES
  // 无覆盖时各核心 token 的基底有效值（读自 computed style，避免与 tokens.css 重复维护）
  const [baseDefaults, setBaseDefaults] = useState<Record<string, string>>({})
  // 五个不透明度滑块的当前值（%）；覆盖存在时由 rgba/数值反向解析，无覆盖时回退 computed α
  const [opacityAlphas, setOpacityAlphas] = useState<OpacityAlphas>({
    bubbles: 100,
    input: 100,
    panel: 80,
    skin: 0,
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

  // 有自定义激活（保存或实时预览有实际覆盖）时，「我的主题」标题旁显示标识
  const hasCustomApplied =
    customTheme !== null || (previewOverrides !== null && Object.keys(previewOverrides).length > 0)

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

  /** 实时预览对象（`applyCustomPreview` 只消费 base + overrides；
   *  名称与皮肤不属于预览态，落盘时另走 createCustomFromPreview / 保存入口） */
  const buildPreview = (base: ThemeId, overrides: Record<string, string>): CustomTheme => ({
    name: '',
    base,
    overrides,
  })

  // 不透明度属于**每个自定义主题自己的数据**（以 rgba / 数值覆盖的形式存于 overrides，
  // 由主题 JSON 一起保存 / 导出 / 导入）。激活主题只应用该主题的 overrides，
  // 绝不能用「上一个主题的滑块读数」重新派生覆盖 —— 那会把目标主题已保存的
  // 不透明度改写成源主题的值（基底不同时必现，同基底时偶发，取决于 dirty 通道）。
  // 早先的 dirty-intent 全局重派机制正是这一缺陷的来源，已整体移除。
  const handleColorCommit = (key: CoreTokenKey, hex: string) => {
    const next = applyCoreColor(activeOverrides, key, hex)
    const nextTheme = buildPreview(theme, next)
    applyCustomPreview(nextTheme)
  }

  /* ── 不透明度滑块 ── */
  // 取当前生效色（computed style，兼容 hex/rgb/rgba）→ 转 rgba(color, α) 写入覆盖通道。
  // 拖动时读到的 rgb 即基底色（α 不改变 rgb 分量），后续拖动可继续以此派生。
  // 覆盖随「保存」落到当前自定义主题；切换主题时按目标主题的覆盖原样恢复。
  const handleBubbleOpacity = (percent: number) => {
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
    const nextTheme = buildPreview(theme, next)
    applyCustomPreview(nextTheme)
  }

  const handleInputOpacity = (percent: number) => {
    setOpacityAlphas(prev => ({ ...prev, input: percent }))
    const cs = getComputedStyle(document.documentElement)
    const inputColor = parseColorValue(cs.getPropertyValue('--input-bg').trim())
    const next = { ...activeOverrides }
    if (percent >= 100) delete next['--input-bg']
    else if (inputColor) next['--input-bg'] = toRgba(inputColor, percent / 100)
    const nextTheme = buildPreview(theme, next)
    applyCustomPreview(nextTheme)
  }

  // 设置中心面板：语义上属弹窗族（--panel-bg），与弹窗同规则 ——
  // 基底即半透明玻璃，100% 需写 α=1 实色覆盖（不能 delete 回落基底，否则仍是玻璃）
  const handlePanelOpacity = (percent: number) => {
    setOpacityAlphas(prev => ({ ...prev, panel: percent }))
    const cs = getComputedStyle(document.documentElement)
    const panelColor = parseColorValue(cs.getPropertyValue('--panel-bg').trim())
    const next = { ...activeOverrides }
    if (panelColor) next['--panel-bg'] = toRgba(panelColor, percent / 100)
    const nextTheme = buildPreview(theme, next)
    applyCustomPreview(nextTheme)
  }

  const handleModalOpacity = (percent: number) => {
    setOpacityAlphas(prev => ({ ...prev, modal: percent }))
    const cs = getComputedStyle(document.documentElement)
    const modalColor = parseColorValue(cs.getPropertyValue('--modal-bg').trim())
    const next = { ...activeOverrides }
    // 弹窗基底即半透明玻璃，100% 需写 α=1 实色覆盖（不能 delete 回落基底，否则仍是玻璃）
    if (modalColor) next['--modal-bg'] = toRgba(modalColor, percent / 100)
    const nextTheme = buildPreview(theme, next)
    applyCustomPreview(nextTheme)
  }

  // 皮肤背景图为数值直写（非 rgba 派生），0% 即隐藏背景图
  const handleSkinOpacity = (percent: number) => {
    setOpacityAlphas(prev => ({ ...prev, skin: percent }))
    const next = { ...activeOverrides }
    next[SKIN_OPACITY_KEY] = String(percent / 100)
    const nextTheme = buildPreview(theme, next)
    applyCustomPreview(nextTheme)
  }

  /* ── 行内「恢复默认」（每个不透明度行 / 每个颜色行各一个小钮）──
     语义严格限定为 **main 现行草稿语义**：只删「未保存草稿」里的对应覆盖键，
     **绝不触碰、不回写用户已保存的自定义主题数据** ——
     不调 saveCustom、不写 localStorage、不新增任何持久化键。

     恢复目标 = **当前生效基线**：已激活的自定义主题里有该键 → 回到它的值；
     基线里没有该键 → 从草稿里删掉该键（回落主题基底默认）。 */
  const baselineOverrides = customTheme?.overrides ?? EMPTY_OVERRIDES

  /** 该组键在草稿里「有值且与基线不同」才有可恢复的东西（否则小钮 disabled） */
  const canResetKeys = (keys: readonly string[]): boolean => {
    if (previewOverrides === null) return false
    return keys.some(key => {
      const draftValue: string | undefined = previewOverrides[key]
      return draftValue !== undefined && draftValue !== baselineOverrides[key]
    })
  }

  /**
   * 把该组键恢复到基线：**只改草稿**（previewOverrides），已保存主题一概不碰。
   * 草稿与基线等值（含被清空）→ clearCustomPreview()：空草稿若仍留在
   * previewOverrides 上会遮蔽已保存主题的覆盖
   * （activeOverrides = previewOverrides ?? customTheme…），用户会看不到自己
   * 保存过的值；等值草稿留着则让「放弃修改」亮着却没有任何真实未保存差异
   * （假未保存态）——两者都清。
   */
  const resetKeysToBaseline = (keys: readonly string[]) => {
    const next: Record<string, string> = { ...(previewOverrides ?? {}) }
    if (keys.includes('--accent')) {
      // 强调色**成组**回退：主键与 4 个派生键必须同源 —— 只回退主键会让派生键留在
      // 旧强调色的取值上（底色回蓝、悬停/辉光还是红的）
      for (const key of keys) delete next[key]
      const baselineAccent = baselineOverrides['--accent']
      // 基线可能**只有主键**（导入的主题 JSON）→ 按主键重新派生，保持同源
      if (baselineAccent !== undefined) Object.assign(next, accentOverrideGroup(baselineAccent))
    } else {
      for (const key of keys) {
        const baseline: string | undefined = baselineOverrides[key]
        if (baseline !== undefined) next[key] = baseline
        else delete next[key]
      }
    }
    // 只剩与基线等值的键 = 没有真实未保存差异 → 清草稿（空草稿同样在此收敛）：
    // 等值草稿留着会让「放弃修改」亮着，构成假未保存态
    const hasRealDiff = Object.keys(next).some(key => next[key] !== baselineOverrides[key])
    if (!hasRealDiff) {
      clearCustomPreview()
      return
    }
    applyCustomPreview(buildPreview(theme, next))
  }

  // 预览草稿 = 未保存修改（编辑一律先预览不落盘；保存按钮才持久化——
  // 修复「已激活自定义主题时编辑直接落盘，下次启动仍是草稿」）
  const hasUnsavedPreview = previewOverrides !== null && Object.keys(previewOverrides).length > 0

  /**
   * 新建链路（「＋ 新建」卡与「保存」的非编辑态共用）：
   * 以「当前系统主题 + 当前微调预览值」建条，皮肤继承当前**生效**的那张图作为
   * 初始快照（用户随后可在卡上改）；新卡落盘即进入行内改名态（预填「未命名主题」）。
   */
  const createCustomFromPreview = () => {
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
    createCustomFromPreview()
  }

  /** 激活「我的主题」网格里的某张卡；有未保存草稿时先确认放弃 */
  const handleActivateCustom = (ct: CustomTheme) => {
    if (!ct.id) return
    if (ct.id === activeCustomId && !hasUnsavedPreview) return
    if (hasUnsavedPreview && !window.confirm(t('themes.customSwitchConfirm'))) return
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

  /** 放弃未保存修改：清预览覆盖，回到已保存主题 / 纯内置基底 */
  const handleDiscardPreview = () => {
    clearCustomPreview()
    showToast(t('themes.customDiscardDone'), 'info')
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
                  // 点系统预设卡 = 回纯净预设态（铁律②）：自定义覆盖是自定义主题
                  // 专属，不得跨系统主题残留。useTheme.setTheme 本就清草稿预览，
                  // 此处显式再清一次是把契约钉在本调用点，不依赖上游实现的自觉。
                  clearCustomPreview()
                  // 背景同步回预设态（LS_SKIN）：**无条件**执行，不看此前是否挂着
                  // 自定义主题 —— 背景图是上一状态留下的，不回退就会一直挂着
                  // （主题皮肤快照只归该主题，边界见 useTheme.tsx 顶部注释）。
                  // 早先这里有 `if (fromCustom)` 守卫，导致预设态之间切主题时
                  // 背景纹丝不动 —— 上一张图继续显示在新的系统主题上。
                  setTheme(th.id)
                  void applySkinBg(readSkinBg())
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
            {/* 末位固定「＋ 新建」：以「当前系统主题 + 当前微调预览值」建新主题，
                落盘即出现在主题卡首位并进入行内改名态 */}
            <button
              type="button"
              className="apnp-theme-card apnp-theme-card-new"
              onClick={createCustomFromPreview}
              title={t('themes.newTheme')}
            >
              <span className="apnp-theme-new-inner">
                <IconPlus size={18} />
                <span>{t('themes.newTheme')}</span>
              </span>
            </button>
          </div>
        </section>

        {/* ── 三「界面不透明度」：5 个滑块铺开（进到这里就是要调，铺开是诚实）── */}
        <section className="apnp-zone">
          <div className="apnp-zone-title">{t('themes.opacityZone')}</div>
          <OpacitySliderRow
            label={t('themes.opacityBubbles')}
            value={opacityAlphas.bubbles}
            onChange={handleBubbleOpacity}
            canReset={canResetKeys(OPACITY_RESET_KEYS.bubbles)}
            onReset={() => resetKeysToBaseline(OPACITY_RESET_KEYS.bubbles)}
          />
          <OpacitySliderRow
            label={t('themes.opacityInput')}
            value={opacityAlphas.input}
            onChange={handleInputOpacity}
            canReset={canResetKeys(OPACITY_RESET_KEYS.input)}
            onReset={() => resetKeysToBaseline(OPACITY_RESET_KEYS.input)}
          />
          <OpacitySliderRow
            label={t('themes.opacityPanel')}
            value={opacityAlphas.panel}
            onChange={handlePanelOpacity}
            canReset={canResetKeys(OPACITY_RESET_KEYS.panel)}
            onReset={() => resetKeysToBaseline(OPACITY_RESET_KEYS.panel)}
          />
          <OpacitySliderRow
            label={t('themes.opacityModal')}
            value={opacityAlphas.modal}
            onChange={handleModalOpacity}
            canReset={canResetKeys(OPACITY_RESET_KEYS.modal)}
            onReset={() => resetKeysToBaseline(OPACITY_RESET_KEYS.modal)}
          />
          <OpacitySliderRow
            label={t('themes.opacitySkin')}
            value={opacityAlphas.skin}
            min={0}
            onChange={handleSkinOpacity}
            canReset={canResetKeys(OPACITY_RESET_KEYS.skin)}
            onReset={() => resetKeysToBaseline(OPACITY_RESET_KEYS.skin)}
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
              onCommit={handleColorCommit}
              canReset={canResetKeys(colorResetKeys(key))}
              onReset={() => resetKeysToBaseline(colorResetKeys(key))}
            />
          ))}
        </section>

        {/* ── 紧凑保存行：次要在左、主操作「保存」贴最右缘 ──
            「保存」常驻：编辑激活主题 → 更新该条；否则走新建链路（新卡进入行内改名）。
            「放弃修改」是全面板唯一的条件按钮：仅有无保存调整时出现。 */}
        <div className="apnp-save-control">
          {hasUnsavedPreview && (
            <button type="button" className="apnp-glass-btn" onClick={handleDiscardPreview}>
              {t('themes.customDiscard')}
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
