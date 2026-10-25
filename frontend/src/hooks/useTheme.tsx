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
import { newCustomThemeId, parseCustomThemeJSON, type CustomTheme } from './customTheme'
import { applySkinBg, readSkinBg } from '../ui/skinBg'

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
  /** 未保存的实时预览覆盖（仅作用于当前会话，不落盘） */
  previewOverrides: Record<string, string> | null
  /** 应用实时预览：切换基底 + 临时覆盖 */
  applyCustomPreview: (custom: CustomTheme) => void
  /** 清除实时预览覆盖（保留基底选择） */
  clearCustomPreview: () => void
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
  const [previewOverrides, setPreviewOverrides] = useState<Record<string, string> | null>(null)
  const appliedVarsRef = useRef<Set<string>>(new Set())

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

  // 应用基底主题 + 覆盖（预览优先，其次已保存自定义，最后纯内置）。
  // 用 useLayoutEffect 而非 useEffect：保证在子组件（设置页等）读取
  // computed style 前，data-theme 与覆盖已写入 DOM（子 passive effect 总在其后执行）。
  useLayoutEffect(() => {
    const html = document.documentElement
    html.setAttribute('data-theme', theme)
    if (previewOverrides !== null) {
      applyOverrides(previewOverrides)
    } else if (customTheme) {
      applyOverrides(customTheme.overrides)
    } else {
      applyOverrides({})
    }
    // 可选：添加过渡动画类
    html.classList.add('theme-transitioning')
    const timer = setTimeout(() => html.classList.remove('theme-transitioning'), 350)
    return () => clearTimeout(timer)
  }, [theme, customTheme, previewOverrides, applyOverrides])

  // 主题快照同步（App Plugin 体系 §4.3）：任何主题变化（内置切换 / 自定义保存 /
  // 滑块拖动预览）→ 300ms 防抖写入后端 theme_snapshot_save，插件 iframe 经
  // /plugins-shared/theme.css 获得与主窗口逐像素一致的主题（预览态也同步——
  // 插件看到的就是用户看到的）。
  useEffect(() => {
    const overrides =
      previewOverrides !== null ? previewOverrides : customTheme ? customTheme.overrides : {}
    const timer = setTimeout(() => {
      import('../main-window/lib/plugin-apps')
        .then(m => m.themeSnapshotSave(theme, overrides).catch(() => {}))
        .catch(() => {})
    }, 300)
    return () => clearTimeout(timer)
  }, [theme, customTheme, previewOverrides])

  // 选内置主题 = 停用自定义激活（列表保留，用户可再回来）+ 清草稿预览
  const setTheme = useCallback(
    (newTheme: ThemeId) => {
      setThemeState(newTheme)
      setPreviewOverrides(null)
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
    setPreviewOverrides(null)
    setCustomStore(prev => {
      persistCustomStore(prev.themes, null)
      return { themes: prev.themes, activeId: null }
    })
    // 停用自定义 = 回系统预设态 → 背景同步回 LS_SKIN（与外观浮窗点系统主题卡
    // 同一口径）。早先这里不碰背景，HUD 快捷切主题后背景图会留在原主题上。
    void applySkinBg(readSkinBg())
  }, [persistCustomStore])

  // 实时预览：切换基底 + 临时覆盖（不持久化，用于「保存前」的编辑态）
  const applyCustomPreview = useCallback((custom: CustomTheme) => {
    setThemeState(custom.base)
    setPreviewOverrides(custom.overrides)
  }, [])

  const clearCustomPreview = useCallback(() => {
    setPreviewOverrides(null)
  }, [])

  // 保存到「我的主题」列表并激活（有 id 更新该条目，无 id 生成新条目）
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
      setPreviewOverrides(null)
      setThemeState(entry.base)
      try {
        localStorage.setItem(LS_THEME, entry.base)
      } catch {}
    },
    [persistCustomStore],
  )

  /** 激活列表中某个自定义主题（清草稿预览） */
  const activateCustom = useCallback(
    (id: string) => {
      setCustomStore(prev => {
        const target = prev.themes.find(t => t.id === id)
        if (!target) return prev
        persistCustomStore(prev.themes, id)
        setPreviewOverrides(null)
        setThemeState(target.base)
        // 皮肤快照恢复（本文件唯一允许的结构新增，见顶部边界注释）：
        // 激活哪个主题，全局背景就回到哪个主题存的样子；无皮肤 = 清空。
        // 写入点仍只有 skinBg.applySkinBg 一个，广播 / SkinBackdrop 渲染链路不动。
        void applySkinBg(target.skin ?? '')
        try {
          localStorage.setItem(LS_THEME, target.base)
        } catch {}
        return { themes: prev.themes, activeId: id }
      })
    },
    [persistCustomStore],
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
      setPreviewOverrides(null)
    },
    [persistCustomStore],
  )

  // 停用自定义：回纯内置基底（列表保留，不删库）
  const clearCustom = useCallback(() => {
    setPreviewOverrides(null)
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
        previewOverrides,
        applyCustomPreview,
        clearCustomPreview,
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
