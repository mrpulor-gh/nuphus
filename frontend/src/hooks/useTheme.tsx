import {
  createContext,
  useContext,
  useState,
  useLayoutEffect,
  useCallback,
  useRef,
  useEffect,
  type ReactNode,
} from 'react'
import {
  CORE_TOKEN_KEYS,
  deriveTweaksOverrides,
  getScopeTweaks,
  isEmptyTweaks,
  newCustomThemeId,
  parseCustomThemeJSON,
  readTweakStore,
  resetTweakAlpha as clearTweakAlpha,
  resetTweakToken as clearTweakToken,
  sameOverrides,
  setScopeTweaks,
  setTweakAlpha as putTweakAlpha,
  setTweakToken as putTweakToken,
  stripTweakKeys,
  writeTweakStore,
  EMPTY_TWEAKS,
  type AppearanceTweaks,
  type CoreTokenKey,
  type CustomTheme,
  type OpacityChannel,
  type TweakScope,
  type TweakStore,
} from './customTheme'
import { applySkinBg } from '../ui/skinBg'

export type ThemeId = 'dark' | 'light' | 'fresh'

/**
 * 皮肤背景的两层归属（2026-09-28 per-card skin 定稿，改前先读）：
 *
 * - **系统预设态**（未激活任何自定义主题）的背景 = `ui/skinBg.ts` 的 LS_SKIN，
 *   由外观浮窗的「皮肤背景」区写入；启动恢复也走它（App 层 + SkinBackdrop 挂载时）。
 * - **激活自定义主题时**以该主题的 `skin` 快照为准：`activateCustom` 按快照写回
 *   全局背景（`target.skin ?? ''` —— 无皮肤即清空，不继承上一个主题的图）。
 * - 回到预设态（停用自定义 / 删除激活项 / 点内置主题卡）→ 背景回 LS_SKIN。
 *   这三条交互都在 AppearancePanel 内发起，故由面板在对应 handler 里补
 *   `applySkinBg(readSkinBg())`；本文件只负责「激活即恢复快照」这一步，
 *   save/activate/delete 的数据语义与 skinBg 的广播/渲染链路均未改动。
 *
 * 颜色 / 透明度的三层（2026-10 微调层定稿，改前先读）：
 *
 *   内置基底（data-theme）
 *     ↑ 主题层：激活的自定义主题 overrides（随主题来去，切走即失效）
 *     ↑ 微调层：用户在外观浮窗里调的 5 个不透明度 + 5 个核心色（持久、跨主题保留）
 *
 * 微调层与旧「会话级预览草稿」的区别是**持久化**与**归属**：旧实现把拖动结果塞进
 * previewOverrides，只活到本次会话，且切主题即被清空 —— 用户看到的就是「重启 /
 * 换主题参数被恢复默认」。现在值存 localStorage、由本 Provider 统一派生应用，
 * 因此两个通道都必须走 setTweakAlpha / setTweakToken，禁止再引入会话级覆盖。
 */

interface ThemeContextType {
  theme: ThemeId
  setTheme: (theme: ThemeId) => void
  toggleTheme: () => void
  /** 当前激活的自定义主题（派生自 customThemes + activeCustomId；null = 未启用自定义） */
  customTheme: CustomTheme | null
  /** 「我的主题」列表（全部已保存自定义主题，含未激活） */
  customThemes: CustomTheme[]
  /** 当前激活的自定义主题 id */
  activeCustomId: string | null
  /**
   * 外观微调层（不透明度 5 通道 + 核心色 5 token）。
   * 属于**个人偏好**而不是主题的一部分：叠加在「基底 → 自定义主题」之上，
   * 换主题 / 重启都保留，每项可单独恢复默认。持久化见 customTheme.readTweaks。
   */
  tweaks: AppearanceTweaks
  /** 当前**实际生效**的覆盖集合（主题层 + 微调派生），供保存快照 / 插件主题同步消费 */
  resolvedOverrides: Record<string, string>
  /** 设置某项微调（立即落盘，不需要「保存」） */
  setTweakAlpha: (channel: OpacityChannel, percent: number) => void
  setTweakToken: (key: CoreTokenKey, hex: string) => void
  /** 单参数恢复默认（删键 = 回落当前基底默认值） */
  resetTweakAlpha: (channel: OpacityChannel) => void
  resetTweakToken: (key: CoreTokenKey) => void
  /** 全部恢复默认（清空微调层，主题选择不变） */
  resetTweaks: () => void
  /** 保存自定义主题到列表并激活（有 id 更新同名条目，无 id 生成新条目） */
  saveCustom: (custom: CustomTheme) => void
  /** 激活列表中某个自定义主题（清除草稿预览） */
  activateCustom: (id: string) => void
  /** 从列表删除自定义主题；删的是激活项则回纯内置基底 */
  deleteCustom: (id: string) => void
  /** 停用自定义（回纯内置基底，列表保留） */
  clearCustom: () => void
}

const ThemeContext = createContext<ThemeContextType | null>(null)

const LS_THEME = 'nuphus_theme'
/** 旧版单自定义主题 key（迁移源） */
const LS_CUSTOM_THEME_LEGACY = 'nuphus_custom_theme'
const LS_CUSTOM_THEMES = 'nuphus_custom_themes'
const LS_CUSTOM_ACTIVE = 'nuphus_custom_active'

/** 读取「我的主题」列表 + 激活 id。自动迁移旧版单主题存储。 */
function readCustomThemes(): { themes: CustomTheme[]; activeId: string | null } {
  try {
    const raw = localStorage.getItem(LS_CUSTOM_THEMES)
    if (raw) {
      const arr = JSON.parse(raw)
      if (Array.isArray(arr)) {
        const themes: CustomTheme[] = []
        for (const item of arr) {
          const res = parseCustomThemeJSON(JSON.stringify(item))
          if (res.ok) themes.push({ ...res.theme, id: res.theme.id ?? newCustomThemeId() })
          // 解析失败不能静默跳过：若被跳过的正是激活项，启动就会整体回落默认主题，
          // 而现场无迹可寻（2026-09-28：用户报「刷新后回默认主题」时的取证缺口）
          else
            console.error(
              '[theme] 自定义主题条目解析失败已跳过；若它是激活项，本次启动会回落默认主题：',
              res.reason,
              item,
            )
        }
        const activeId = localStorage.getItem(LS_CUSTOM_ACTIVE)
        const validActive = activeId && themes.some(t => t.id === activeId) ? activeId : null
        if (activeId && !validActive) {
          console.error('[theme] 激活 id 不在已解析列表内，本次启动回落系统预设主题：', activeId)
        }
        return { themes, activeId: validActive }
      }
    }
    // 迁移：旧版单主题
    const legacy = localStorage.getItem(LS_CUSTOM_THEME_LEGACY)
    if (legacy) {
      const res = parseCustomThemeJSON(legacy)
      if (res.ok) {
        const migrated: CustomTheme = { ...res.theme, id: res.theme.id ?? newCustomThemeId() }
        const themes = [migrated]
        localStorage.setItem(LS_CUSTOM_THEMES, JSON.stringify(themes))
        localStorage.setItem(LS_CUSTOM_ACTIVE, migrated.id!)
        localStorage.removeItem(LS_CUSTOM_THEME_LEGACY)
        return { themes, activeId: migrated.id! }
      }
    }
  } catch {}
  return { themes: [], activeId: null }
}

function readBaseTheme(): ThemeId {
  try {
    const saved = localStorage.getItem(LS_THEME) as ThemeId
    if (saved === 'dark' || saved === 'light' || saved === 'fresh') return saved
  } catch {}
  return 'dark'
}

/**
 * 启动时皮肤背景的**恢复源**（必须与 `activateCustom` 的写入语义逐字一致）：
 * - 激活自定义主题 → 该主题的 `skin` 快照；**无皮肤 = 空串（清空）**，
 *   不继承上一个主题/系统预设的图（`activateCustom` 里就是 `target.skin ?? ''`）
 * - 系统预设态（未激活）→ LS_SKIN（`presetSkin`）
 *
 * 为什么单独抽成纯函数：这条二选一曾经直接写在 App 层的 effect 里且只读了
 * LS_SKIN，导致激活自定义主题的用户刷新后背景被预设值覆盖（快照无人重放）。
 * 抽出来后①App 层只是它的一个调用点；②语义被测试钉住，回归会立刻红。
 *
 * 注意 `customTheme ? … : …` 不能图省事写成 `customTheme?.skin ?? presetSkin`：
 * 那会让「激活但无皮肤」错落回预设图——与手动激活时的清空语义自相矛盾。
 */
export function skinRestoreSource(customTheme: CustomTheme | null, presetSkin: string): string {
  return customTheme ? (customTheme.skin ?? '') : presetSkin
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  // 「我的主题」列表 + 激活 id（启动时迁移旧版单主题存储）
  const [customStore, setCustomStore] = useState(readCustomThemes)
  const { themes: customThemes, activeId: activeCustomId } = customStore
  // 派生：当前激活的自定义主题（兼容现有消费方的 customTheme 读取）
  const customTheme = customThemes.find(t => t.id === activeCustomId) ?? null
  // 基底：激活自定义的 base 优先，其次内置主题
  const [theme, setThemeState] = useState<ThemeId>(() => {
    const store = readCustomThemes()
    const active = store.themes.find(t => t.id === store.activeId)
    return active?.base ?? readBaseTheme()
  })
  const appliedVarsRef = useRef<Set<string>>(new Set())

  /**
   * 微调层（**按主题分槽**）：内置主题用 dark/light/fresh，自定义主题用主题 id。
   * 启动即从 localStorage 恢复 —— 这是「重启后参数不恢复默认」的唯一来源；
   * 切换主题只是换到另一个槽位，A 的参数留在 A。
   */
  const [tweakStore, setTweakStoreState] = useState<TweakStore>(() => {
    const { store, legacy } = readTweakStore()
    if (!legacy || isEmptyTweaks(legacy)) return store
    // 迁移：早期「一份不分主题」的微调 → 归到当前主题槽位（只有这里知道当前主题）
    const currentScope = localStorage.getItem(LS_CUSTOM_ACTIVE) ?? readBaseTheme()
    const migrated = setScopeTweaks(store, currentScope, legacy)
    writeTweakStore(migrated)
    return migrated
  })
  /** 当前微调作用域：激活的自定义主题 id 优先，否则内置主题 id */
  const tweakScope: TweakScope = activeCustomId ?? theme
  const tweaks = getScopeTweaks(tweakStore, tweakScope)
  const scopeRef = useRef<TweakScope>(tweakScope)
  scopeRef.current = tweakScope
  const customThemeRef = useRef<CustomTheme | null>(customTheme)
  customThemeRef.current = customTheme
  /** 实际生效的覆盖（主题层 + 微调派生）——保存快照 / 插件主题同步消费它 */
  const [resolvedOverrides, setResolvedOverrides] = useState<Record<string, string>>({})

  /** 列表 + 激活 id 落盘（统一写入点） */
  const persistCustomStore = useCallback((themes: CustomTheme[], activeId: string | null) => {
    try {
      localStorage.setItem(LS_CUSTOM_THEMES, JSON.stringify(themes))
      if (activeId) localStorage.setItem(LS_CUSTOM_ACTIVE, activeId)
      else localStorage.removeItem(LS_CUSTOM_ACTIVE)
    } catch {}
  }, [])

  // 覆盖应用：先清掉上一次已设置的变量，再写入新值。
  // 内联样式优先级天然压过 tokens.css 的 [data-theme=*] 规则，无需改动 tokens.css。
  const applyOverrides = useCallback((overrides: Record<string, string>) => {
    const html = document.documentElement
    for (const key of appliedVarsRef.current) html.style.removeProperty(key)
    appliedVarsRef.current.clear()
    for (const [key, value] of Object.entries(overrides)) {
      html.style.setProperty(key, value)
      appliedVarsRef.current.add(key)
    }
  }, [])

  /**
   * 应用「基底主题 → 自定义主题 → 微调层」三层，分两阶段：
   *
   * 阶段一：写入 data-theme 与主题层覆盖（customTheme.overrides）。
   * 阶段二：读**基底色**（此时内联里还没有微调的派生色），按微调层的百分比
   *        派生出 rgba 覆盖再叠上。之所以要两阶段，是因为「百分比属于用户、
   *        rgb 属于基底」：换主题时同一条 60% 气泡必须按新基底的色重算，
   *        不能把旧基底的 rgba 硬带过去。
   *
   * 用 useLayoutEffect 而非 useEffect：保证子组件（外观浮窗 / 设置页）读
   * computed style 时，data-theme 与全部覆盖已落地（子 passive effect 总在其后）。
   */
  useLayoutEffect(() => {
    const html = document.documentElement
    html.setAttribute('data-theme', theme)
    const themeOverrides = customTheme ? customTheme.overrides : {}
    applyOverrides(themeOverrides)
    const derived = deriveTweaksOverrides(tweaks, key =>
      getComputedStyle(html).getPropertyValue(key).trim(),
    )
    const merged =
      Object.keys(derived).length === 0 ? themeOverrides : { ...themeOverrides, ...derived }
    if (!sameOverrides(merged, themeOverrides)) applyOverrides(merged)
    setResolvedOverrides(merged)
    // 可选：添加过渡动画类
    html.classList.add('theme-transitioning')
    const timer = setTimeout(() => html.classList.remove('theme-transitioning'), 350)
    return () => clearTimeout(timer)
  }, [theme, customTheme, tweaks, applyOverrides])

  // 主题快照同步（App Plugin 体系 §4.3）：任何主题 / 微调变化 → 300ms 防抖写入后端
  // theme_snapshot_save，插件 iframe 经 /plugins-shared/theme.css 获得与主窗口逐像素
  // 一致的主题（微调层同样同步——插件看到的就是用户看到的）。
  useEffect(() => {
    const timer = setTimeout(() => {
      import('../main-window/lib/plugin-apps')
        .then(m => m.themeSnapshotSave(theme, resolvedOverrides).catch(() => {}))
        .catch(() => {})
    }, 300)
    return () => clearTimeout(timer)
  }, [theme, resolvedOverrides])

  // 选内置主题 = 停用自定义激活（列表保留，用户可再回来）。
  // 微调层不需要在这里清理：每个主题各有槽位，切主题只是换槽位。
  const setTheme = useCallback(
    (newTheme: ThemeId) => {
      setThemeState(newTheme)
      setCustomStore(prev => {
        const next = { themes: prev.themes, activeId: null }
        persistCustomStore(next.themes, null)
        return next
      })
      try {
        localStorage.setItem(LS_THEME, newTheme)
      } catch {}
    },
    [persistCustomStore],
  )

  const toggleTheme = useCallback(() => {
    setThemeState(prev => {
      const next = prev === 'dark' ? 'light' : prev === 'light' ? 'fresh' : 'dark'
      try {
        localStorage.setItem(LS_THEME, next)
      } catch {}
      return next
    })
    setCustomStore(prev => {
      persistCustomStore(prev.themes, null)
      return { themes: prev.themes, activeId: null }
    })
  }, [persistCustomStore])

  // 保存到「我的主题」列表并激活（有 id 更新该条目，无 id 生成新条目）。
  // 快照 = 调用方传入的 overrides（外观浮窗给的是「当前实际生效」的合并结果），
  // 微调层本身不动 —— 保存只是把它固化成一个命名主题，不改变当前显示。
  const saveCustom = useCallback(
    (custom: CustomTheme) => {
      const id = custom.id ?? newCustomThemeId()
      const entry: CustomTheme = { ...custom, id }
      setCustomStore(prev => {
        const idx = prev.themes.findIndex(t => t.id === id)
        const themes =
          idx >= 0 ? prev.themes.map(t => (t.id === id ? entry : t)) : [...prev.themes, entry]
        persistCustomStore(themes, id)
        return { themes, activeId: id }
      })
      setThemeState(entry.base)
      try {
        localStorage.setItem(LS_THEME, entry.base)
      } catch {}
    },
    [persistCustomStore],
  )

  /* ── 微调层写入（立即落盘；只写**当前主题的槽位**）──
     放在 saveCustom 之后：单参数/全部恢复默认要把自定义主题快照里「烘入」的值一并
     摘掉（否则主题层会把旧值套回去，界面看着像没生效）。 */
  const updateScopeTweaks = useCallback((fn: (current: AppearanceTweaks) => AppearanceTweaks) => {
    setTweakStoreState(prev => {
      const scope = scopeRef.current
      const nextStore = setScopeTweaks(prev, scope, fn(getScopeTweaks(prev, scope)))
      writeTweakStore(nextStore)
      return nextStore
    })
  }, [])

  const setTweakAlpha = useCallback(
    (channel: OpacityChannel, percent: number) =>
      updateScopeTweaks(current => putTweakAlpha(current, channel, percent)),
    [updateScopeTweaks],
  )

  const setTweakToken = useCallback(
    (key: CoreTokenKey, hex: string) =>
      updateScopeTweaks(current => putTweakToken(current, key, hex)),
    [updateScopeTweaks],
  )

  /** 单参数恢复默认：删当前主题槽位里的一项，并把该键从主题快照里摘掉 */
  const resetScopeEntry = useCallback(
    (target: { channel: OpacityChannel } | { token: CoreTokenKey }) => {
      updateScopeTweaks(current =>
        'channel' in target
          ? clearTweakAlpha(current, target.channel)
          : clearTweakToken(current, target.token),
      )
      const active = customThemeRef.current
      if (!active) return
      const stripped = stripTweakKeys(active.overrides, target)
      if (stripped !== active.overrides) saveCustom({ ...active, overrides: stripped })
    },
    [saveCustom, updateScopeTweaks],
  )

  const resetTweakAlpha = useCallback(
    (channel: OpacityChannel) => resetScopeEntry({ channel }),
    [resetScopeEntry],
  )

  const resetTweakToken = useCallback(
    (key: CoreTokenKey) => resetScopeEntry({ token: key }),
    [resetScopeEntry],
  )

  /** 全部恢复默认：清空当前主题的微调槽位（自定义主题连同快照里的烘入值一起摘掉） */
  const resetTweaks = useCallback(() => {
    updateScopeTweaks(() => EMPTY_TWEAKS)
    const active = customThemeRef.current
    if (!active) return
    let overrides = { ...active.overrides }
    for (const channel of ['bubbles', 'input', 'panel', 'modal', 'skin'] as OpacityChannel[]) {
      overrides = stripTweakKeys(overrides, { channel })
    }
    for (const key of CORE_TOKEN_KEYS) overrides = stripTweakKeys(overrides, { token: key })
    if (overrides !== active.overrides) saveCustom({ ...active, overrides })
  }, [saveCustom, updateScopeTweaks])

  /**
   * 激活列表中某个自定义主题：只切基底与主题层，微调层原样保留
   * （每个主题各有自己的参数槽位，切过去就是它自己的参数）。
   */
  const activateCustom = useCallback(
    (id: string) => {
      const target = customThemes.find(t => t.id === id)
      if (!target) return
      persistCustomStore(customThemes, id)
      setCustomStore({ themes: customThemes, activeId: id })
      setThemeState(target.base)
      // 皮肤快照恢复（本文件唯一允许的结构新增，见顶部边界注释）：
      // 激活哪个主题，全局背景就回到哪个主题存的样子；无皮肤 = 清空。
      // 写入点仍只有 skinBg.applySkinBg 一个，广播 / SkinBackdrop 渲染链路不动。
      void applySkinBg(target.skin ?? '')
      try {
        localStorage.setItem(LS_THEME, target.base)
      } catch {}
    },
    [customThemes, persistCustomStore],
  )

  /** 删除自定义主题；删的是激活项则回纯内置基底 */
  const deleteCustom = useCallback(
    (id: string) => {
      setCustomStore(prev => {
        const themes = prev.themes.filter(t => t.id !== id)
        const activeId = prev.activeId === id ? null : prev.activeId
        persistCustomStore(themes, activeId)
        return { themes, activeId }
      })
    },
    [persistCustomStore],
  )

  // 停用自定义：回纯内置基底（列表保留，不删库）。微调层同样保留 —— 它是用户偏好。
  const clearCustom = useCallback(() => {
    setCustomStore(prev => {
      persistCustomStore(prev.themes, null)
      return { themes: prev.themes, activeId: null }
    })
  }, [persistCustomStore])

  return (
    <ThemeContext.Provider
      value={{
        theme,
        setTheme,
        toggleTheme,
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
      }}
    >
      {children}
    </ThemeContext.Provider>
  )
}

export function useTheme() {
  const ctx = useContext(ThemeContext)
  if (!ctx) throw new Error('useTheme must be used within ThemeProvider')
  return ctx
}
