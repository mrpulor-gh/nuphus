/**
 * SettingsCenter.tsx — 设置中心弹窗（左导航 + 右内容）
 *
 * 定位：介于 CompactModal（窄模态）与整页宿主之间 —— 居中大弹窗
 * （宿主 = 遮罩层 fixed/inset:0 + --overlay-bg，内层面板 `min(1080×760, 视口 − 64px)` 居中），
 * 既容纳「导航 + 整页级子页」，又不铺满窗口（四周保留聊天界面可见）。
 *
 * 宿主分流：两类分区**不在弹窗内渲染**，点击导航项即关闭面板、交给 App 层全屏宿主
 * （面板内容区上限 = 1080 − 导航 236 = 844px，这两类页面在 844px 内结构性不可用）：
 *   - 画布：UI 原型 / 工具 / 工作流编辑器是「左右 320 固定 + 中自适应」的工作台布局，
 *     主画布 = 容器宽 − 640 → 802px 内容区只剩 162px；叠加 `overflow:hidden`，
 *     高度不足是裁切而非滚动 → 走 `.canvas-workbench-host` 全屏宿主（openCanvas 链路）。
 *   - 模型：双栏后主区仅 566px，需在 844px 内重排信息（服务商列表 / 密钥表单 / 模型表格），
 *     样式层解决不了宿主上限 → 走 `.models-page-host` 全屏整页（Ctrl+K → 模型 链路）。
 *   其余 13 项在 844px 下信息完整、导航切换的价值正在这一档，保持弹窗内嵌。
 *
 * 复用原则：右侧内容一律复用现有页面组件，本文件只做「导航 → 分区切换」，
 * 不复制任何子页实现；子页的 lazy 说明符与 App.tsx 各入口保持一致，
 * 命中同一 chunk（不产生重复打包）。
 *
 * 分区 ↔ 子页映射（共 15 项，见 NAV_GROUPS）：
 *   浏览：记忆 MemoriesPage / **画布 → 全屏宿主** / 工作流 WorkflowPage /
 *         技能 SkillsPage / 知识库 KnowledgePage / MCP McpPage / 插件 PluginComingSoon
 *   设置：**模型 → 全屏宿主** / 灵魂 SoulPage / 移动端 MobilePage / 浏览器 BrowserPage /
 *         主题与语言 ThemesPage / 外部 Agent ExternalAgentsPage
 *   管理：权限与安全 SecurityPage / 版本与更新 UpdatePage
 *
 * 子页 onClose 语义：子页自身没有「页壳」（外壳由 CompactModal / 本组件提供），
 * 其中仅「外部 Agent」表单底部的「取消」按钮会用到 onClose → 统一接设置中心关闭
 * （= 退出该页返回聊天），与它在弹窗里的原语义一致。
 */
import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from 'react'
import type { WorkflowItem } from '../../core/types'
import { IconButton } from '../../ui/Button'
import {
  IconBrowser,
  IconBrain,
  IconCpu,
  IconFile,
  IconHistory,
  IconPalette,
  IconPlug,
  IconPuzzle,
  IconRefresh,
  IconSettings,
  IconShield,
  IconSmartphone,
  IconSparkles,
  IconWorkflow,
  IconWrench,
  IconX,
} from '../../ui/Icons'
import { useLanguage } from '../../locales'
import '../../styles/settings-center.css'

// ── 子页按需加载（说明符与 App.tsx 完全一致 → 共用同一 chunk）──
const MemoriesPage = lazy(() =>
  import('../memories/MemoriesPage').then(m => ({ default: m.MemoriesPage })),
)
// ⚠️ 画布 / 模型两个分区**不在此 lazy 加载**：它们走 App 层全屏宿主（见文件头「宿主分流」），
// 由 App.tsx 各自入口统一加载，避免在弹窗链路里多挂一份 chunk 引用。
const WorkflowPage = lazy(() =>
  import('../workflow/WorkflowPage').then(m => ({ default: m.WorkflowPage })),
)
const KnowledgePage = lazy(() =>
  import('../knowledge/KnowledgePage').then(m => ({ default: m.KnowledgePage })),
)
const SkillsPage = lazy(() => import('./SkillsPage').then(m => ({ default: m.SkillsPage })))
const McpPage = lazy(() => import('./McpPage').then(m => ({ default: m.McpPage })))
const PluginComingSoon = lazy(() =>
  import('./PluginComingSoon').then(m => ({ default: m.PluginComingSoon })),
)
const SoulPage = lazy(() => import('./SoulPage').then(m => ({ default: m.SoulPage })))
const MobilePage = lazy(() => import('./MobilePage').then(m => ({ default: m.MobilePage })))
const BrowserPage = lazy(() => import('./BrowserPage').then(m => ({ default: m.BrowserPage })))
const ThemesPage = lazy(() => import('./ThemesPage').then(m => ({ default: m.ThemesPage })))
const ExternalAgentsPage = lazy(() =>
  import('./ExternalAgentsPage').then(m => ({ default: m.ExternalAgentsPage })),
)
const SecurityPage = lazy(() => import('./SecurityPage').then(m => ({ default: m.SecurityPage })))
const UpdatePage = lazy(() => import('./UpdatePage').then(m => ({ default: m.UpdatePage })))

export type SettingsSectionId =
  | 'memories'
  | 'canvas'
  | 'workflows'
  | 'skills'
  | 'knowledge'
  | 'mcp'
  | 'plugins'
  | 'models'
  | 'soul'
  | 'mobile'
  | 'browser'
  | 'themes'
  | 'external-agents'
  | 'security'
  | 'update'

/** 走 App 层全屏宿主的分区：导航里可见、点击即关闭面板（不在弹窗内渲染） */
type HostedSectionId = 'canvas' | 'models'
/** 可在弹窗内容区内嵌渲染的分区 */
type EmbeddedSectionId = Exclude<SettingsSectionId, HostedSectionId>

interface SettingsNavItem {
  id: SettingsSectionId
  /** i18n key：复用既有键（与子页自身标题、Ctrl+K 命令名保持同一措辞） */
  labelKey: string
  icon: ReactNode
}

/** 左侧导航分组：分组名复用 Ctrl+K 命令面板的三档 category 键，措辞天然一致 */
const NAV_GROUPS: { titleKey: string; items: SettingsNavItem[] }[] = [
  {
    titleKey: 'cmd.category.browse',
    items: [
      { id: 'memories', labelKey: 'app.memories', icon: <IconHistory size={14} /> },
      { id: 'canvas', labelKey: 'cmd.canvas', icon: <IconPalette size={14} /> },
      { id: 'workflows', labelKey: 'app.workflows', icon: <IconWorkflow size={14} /> },
      { id: 'skills', labelKey: 'app.skills', icon: <IconWrench size={14} /> },
      { id: 'knowledge', labelKey: 'app.knowledge', icon: <IconFile size={14} /> },
      { id: 'mcp', labelKey: 'cmd.mcp', icon: <IconPlug size={14} /> },
      { id: 'plugins', labelKey: 'cmd.plugins', icon: <IconPuzzle size={14} /> },
    ],
  },
  {
    titleKey: 'cmd.category.settings',
    items: [
      { id: 'models', labelKey: 'app.models', icon: <IconBrain size={14} /> },
      { id: 'soul', labelKey: 'app.soul', icon: <IconSparkles size={14} /> },
      { id: 'mobile', labelKey: 'app.mobile', icon: <IconSmartphone size={14} /> },
      { id: 'browser', labelKey: 'app.browser', icon: <IconBrowser size={14} /> },
      { id: 'themes', labelKey: 'app.themes', icon: <IconPalette size={14} /> },
      {
        id: 'external-agents',
        labelKey: 'cmd.externalAgents',
        icon: <IconCpu size={14} />,
      },
    ],
  },
  {
    titleKey: 'cmd.category.management',
    items: [
      { id: 'security', labelKey: 'app.security', icon: <IconShield size={14} /> },
      { id: 'update', labelKey: 'app.update', icon: <IconRefresh size={14} /> },
    ],
  },
]

/** 扁平化索引：分区 → 导航项（取当前分区标题用） */
const NAV_ITEMS: SettingsNavItem[] = NAV_GROUPS.flatMap(g => g.items)

export interface SettingsCenterProps {
  /** 右上角关闭 → 返回聊天 */
  onClose: () => void
  /** HUD 提示通道（ThemesPage 需要；沿用 App 的 showToast） */
  showToast: (message: string, type?: 'info' | 'success' | 'warning' | 'error') => void
  /**
   * 工作流列表「运行」：交给 App 层弹运行确认框。
   * ⚠️ 运行确认弹窗（wcf-wrapper z-index 100）低于设置中心宿主（2500），
   * 必须由 App 先关面板再弹，否则确认框会被盖住。
   */
  onRunWorkflow: (workflow: WorkflowItem) => void
  /**
   * 画布分区（UI 原型 / 工具 / 工作流编辑器）：关闭面板 → 走 App 层
   * `.canvas-workbench-host` 全屏宿主（openCanvas 链路）。
   * workflowId 由工作流列表的行内「画布」带入，null = 工作台自选最近草稿。
   */
  onOpenCanvas: (workflowId?: string | null) => void
  /** 模型分区：关闭面板 → 走 App 层 `.models-page-host` 全屏整页 */
  onOpenModels: () => void
}

export function SettingsCenter({
  onClose,
  showToast,
  onRunWorkflow,
  onOpenCanvas,
  onOpenModels,
}: SettingsCenterProps) {
  const { t } = useLanguage()
  const [section, setSection] = useState<EmbeddedSectionId>('memories')

  /**
   * 分区切换入口：宿主分流的两项交给 App 层全屏宿主，其余落在弹窗内容区。
   * TS 在两条早返回之后把 id 收窄为 EmbeddedSectionId（无需断言）。
   */
  const openSection = (id: SettingsSectionId) => {
    if (id === 'canvas') return onOpenCanvas(null)
    if (id === 'models') return onOpenModels()
    setSection(id)
  }

  /**
   * 焦点陷阱：面板声明了 aria-modal，但焦点仍可能 Tab 到背后聊天界面。
   * 打开时把焦点收进面板本体（tabIndex=-1），Tab / Shift+Tab 在面板内循环，
   * 关闭时把焦点还给触发按钮。全应用只保留一个设置入口，故触发元素稳定。
   */
  const panelRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const el = panelRef.current
    if (!el) return
    const restoreTo = document.activeElement as HTMLElement | null
    /** 面板内可聚焦元素（按 DOM 顺序）。不用 offsetParent 过滤：jsdom 恒为 null。 */
    const focusables = (): HTMLElement[] =>
      Array.from(
        el.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter(node => !node.hasAttribute('hidden') && node.getAttribute('aria-hidden') !== 'true')
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Tab') return
      const nodes = focusables()
      if (nodes.length === 0) {
        e.preventDefault()
        return
      }
      const active = document.activeElement as HTMLElement | null
      const idx = active ? nodes.indexOf(active) : -1
      // 焦点在面板本体（初始态）或面板外 → 强制拉回边界项
      if (idx === -1) {
        e.preventDefault()
        ;(e.shiftKey ? nodes[nodes.length - 1] : nodes[0]).focus()
        return
      }
      if (e.shiftKey && idx === 0) {
        e.preventDefault()
        nodes[nodes.length - 1].focus()
      } else if (!e.shiftKey && idx === nodes.length - 1) {
        e.preventDefault()
        nodes[0].focus()
      }
    }
    el.focus()
    el.addEventListener('keydown', onKeyDown)
    return () => {
      el.removeEventListener('keydown', onKeyDown)
      // 触发按钮可能已随面板卸载（画布/模型分流会整页切换）→ 先确认仍在文档中
      if (restoreTo && document.contains(restoreTo)) restoreTo.focus()
    }
  }, [])

  const activeItem = NAV_ITEMS.find(item => item.id === section) ?? NAV_ITEMS[0]

  const renderSection = (): ReactNode => {
    switch (section) {
      case 'memories':
        return <MemoriesPage />
      case 'workflows':
        return (
          <WorkflowPage
            onClose={onClose}
            onRunClick={onRunWorkflow}
            /* 行内「画布」= 关闭面板 → 由 App 层打开全屏画布工作台 */
            onCanvasClick={wf => onOpenCanvas(wf.id)}
          />
        )
      case 'skills':
        return <SkillsPage />
      case 'knowledge':
        return <KnowledgePage onClose={onClose} />
      case 'mcp':
        return <McpPage onClose={onClose} />
      case 'plugins':
        return <PluginComingSoon />
      case 'soul':
        return <SoulPage onClose={onClose} />
      case 'mobile':
        return <MobilePage />
      case 'browser':
        return <BrowserPage onClose={onClose} />
      case 'themes':
        return <ThemesPage onClose={onClose} showToast={showToast} />
      case 'external-agents':
        return <ExternalAgentsPage onClose={onClose} />
      case 'security':
        return <SecurityPage onClose={onClose} />
      case 'update':
        return <UpdatePage />
    }
  }

  return (
    /* 宿主 = 遮罩层：点空白处（mousedown 命中自身）关闭；面板拦截在下一层 */
    <div
      className="settings-center-host"
      data-testid="settings-center-scrim"
      onMouseDown={e => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        className="settings-center-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t('app.settings')}
        ref={panelRef}
        tabIndex={-1}
      >
        {/* ── 顶部 bar：与 models-page-bar 同规格（48px / surface-1 / line-1）── */}
        <div className="settings-center-bar">
          <span className="settings-center-bar-icon">
            <IconSettings size={16} />
          </span>
          <span className="settings-center-title">{t('app.settings')}</span>
          {/* 全屏覆盖层会盖住 TitleBar 的 data-tauri-drag-region，补一条拖动区保证窗口仍可拖动 */}
          <span className="settings-center-drag" data-tauri-drag-region />
          <IconButton
            type="button"
            variant="modal-close"
            className="settings-center-close"
            label={t('common.close')}
            onClick={onClose}
          >
            <IconX size={14} />
          </IconButton>
        </div>

        <div className="settings-center-body">
          {/* ── 左侧导航：分组列表（视觉与 models-rail 同族）── */}
          <nav className="settings-center-nav" aria-label={t('app.settings')}>
            <div className="settings-center-nav-scroll">
              {NAV_GROUPS.map(group => (
                <div key={group.titleKey} className="settings-center-nav-group">
                  <div className="settings-center-nav-group-title">{t(group.titleKey)}</div>
                  <div className="settings-center-nav-list">
                    {group.items.map(item => (
                      <button
                        type="button"
                        key={item.id}
                        className={`settings-center-nav-item${item.id === section ? ' active' : ''}`}
                        aria-current={item.id === section ? 'page' : undefined}
                        /* 画布 / 模型两项由 openSection 分流到 App 层全屏宿主（本面板随即关闭） */
                        onClick={() => openSection(item.id)}
                      >
                        <span className="settings-center-nav-icon">{item.icon}</span>
                        <span className="settings-center-nav-label">{t(item.labelKey)}</span>
                      </button>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </nav>

          {/* ── 右侧内容区：切换分区不卸载外壳（面板保持打开）── */}
          <div className="settings-center-main">
            <div className="settings-center-main-head">
              <span className="settings-center-main-title">{t(activeItem.labelKey)}</span>
            </div>
            {/* key=section：切换分区时重置内容区滚动位置 */}
            <div className="settings-center-main-body" key={section}>
              <Suspense fallback={<div className="page-loading">{t('common.loading')}</div>}>
                {renderSection()}
              </Suspense>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
