/**
 * SettingsCenter.tsx — 设置中心全屏覆盖层（左导航 + 右内容）
 *
 * 定位：区别于 CompactModal（窄模态），设置中心是**全屏宿主**，沿用
 * `.models-page-host` 的既有模式（fixed / inset:0 + 顶部 bar + data-tauri-drag-region 拖动区），
 * 因为要同时容纳「导航 + 整页级子页」。
 *
 * 复用原则：右侧内容一律复用现有页面组件，本文件只做「导航 → 分区切换」，
 * 不复制任何子页实现；子页的 lazy 说明符与 App.tsx 各入口保持一致，
 * 命中同一 chunk（不产生重复打包）。
 *
 * 分区 ↔ 子页映射（共 15 项，见 NAV_GROUPS）：
 *   浏览：记忆 MemoriesPage / 画布 CanvasWorkbenchPage / 工作流 WorkflowPage /
 *         技能 SkillsPage / 知识库 KnowledgePage / MCP McpPage / 插件 PluginComingSoon
 *   设置：模型 ModelsPage / 灵魂 SoulPage / 移动端 MobilePage / 浏览器 BrowserPage /
 *         主题与语言 ThemesPage / 外部 Agent ExternalAgentsPage
 *   管理：权限与安全 SecurityPage / 版本与更新 UpdatePage
 *
 * 子页 onClose 语义：子页自身没有「页壳」（外壳由 CompactModal / 本组件提供），
 * 其中仅「外部 Agent」表单底部的「取消」按钮会用到 onClose → 统一接设置中心关闭
 * （= 退出该页返回聊天），与它在弹窗里的原语义一致。
 */
import { lazy, Suspense, useState, type ReactNode } from 'react'
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
const CanvasWorkbenchPage = lazy(() =>
  import('../workflow/CanvasWorkbenchPage').then(m => ({ default: m.CanvasWorkbenchPage })),
)
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
const ModelsPage = lazy(() => import('./ModelsPage').then(m => ({ default: m.ModelsPage })))
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
  /** 模型切换 / 密钥保存后刷新输入栏模型信息 */
  onModelChanged?: () => void
}

export function SettingsCenter({
  onClose,
  showToast,
  onRunWorkflow,
  onModelChanged,
}: SettingsCenterProps) {
  const { t } = useLanguage()
  const [section, setSection] = useState<SettingsSectionId>('memories')
  /** 画布分区目标工作流：由工作流列表的行内「画布」按钮带入；null = 工作台自选最近草稿 */
  const [canvasWorkflowId, setCanvasWorkflowId] = useState<string | null>(null)

  const activeItem = NAV_ITEMS.find(item => item.id === section) ?? NAV_ITEMS[0]

  const renderSection = (): ReactNode => {
    switch (section) {
      case 'memories':
        return <MemoriesPage />
      case 'canvas':
        return (
          /* 复用 .canvas-workbench-host 的既有子元素规则（header/body 高度链），
             仅由 `.canvas-workbench-host--embedded` 把 fixed 全屏宿主改为撑满内容区 */
          <div className="canvas-workbench-host canvas-workbench-host--embedded">
            <CanvasWorkbenchPage
              workflowId={canvasWorkflowId}
              /* 内嵌画布的 ✕ = 回到工作流列表（整个设置中心由右上角关闭） */
              onClose={() => setSection('workflows')}
            />
          </div>
        )
      case 'workflows':
        return (
          <WorkflowPage
            onClose={onClose}
            onRunClick={onRunWorkflow}
            onCanvasClick={wf => {
              setCanvasWorkflowId(wf.id)
              setSection('canvas')
            }}
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
      case 'models':
        return <ModelsPage onClose={onClose} onModelChanged={onModelChanged} />
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
    <div className="settings-center-host">
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
                      onClick={() => setSection(item.id)}
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
  )
}
