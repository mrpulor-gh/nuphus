/**
 * 顶部导航条：品牌（logo + 字标）| 状态 pill（居中锚点）| 操作按钮
 * - 空闲：中间显示连接状态（已连接/连接中/已断开）
 * - 执行中：中间显示当前工具调用 + 实时用时（用户能直接看到在执行中；
 *   长时间任务 WS 可能显示断开，但用时仍在走，避免误判）
 * - 连接状态居中：状态是用户高频关注的信息，固定中间形成记忆锚点，
 *   右侧仅保留操作按钮（主题切换等）
 * iOS HIG：44pt 高度、17px/600 标题、hairline 下沿分隔、安全区上沿适配
 */

import { useEffect, useRef, useState } from 'react'
import {
  ALargeSmall,
  Check,
  ChevronDown,
  ChevronLeft,
  Moon,
  Plus,
  RefreshCw,
  Sun,
  Wrench,
  X,
} from 'lucide-react'
import { getTheme, toggleTheme, type MobileTheme } from '../theme'
import { getCachedLanUrl, getCachedRelayUrl, type ConnectionMode } from '../connection'
import { getFontSize, setFontSize, type MobileFontSize } from '../fontsize'
import {
  fetchCustomAgents,
  fetchModelConfig,
  switchMobileMode,
  switchMobileModel,
  type CustomAgentBrief,
  type ModelConfig,
  type ShelfSessions,
} from '../api'
import type { ActivityState } from '../store'
import type { WsStatus } from '../ws'
import { t } from '../i18n'

interface Props {
  wsStatus: WsStatus
  activity: ActivityState
  /** 鉴权 token（拉取模型配置 / 切换模式模型） */
  token: string
  /** 当前执行模型（store.model，session_info 事件下发，只读展示） */
  model?: string
  /** 会话累计上下文用量（token_usage 事件实时累计；驱动 header 下模型信息卡 ctx 行） */
  tokenUsage?: { inputTokens: number; outputTokens?: number; cacheHitTokens?: number }
  /** 当前连接渠道（lan=局域网直连 / wan=中继）；header 状态 pill 显示渠道 + 状态色 */
  connMode?: ConnectionMode | null
  /** 手动重新拉取历史（网络/应用切换后历史不显示时一键刷新） */
  onReloadHistory?: () => void
  /** 新会话（设置抽屉 header 按钮 → 选 mode → 遥控 /new-chat 广播，双端回 welcome） */
  onNewChat?: () => void
  /** 重置连接（设置弹窗触发）：清除 token 回到配对页 */
  onDisconnect?: () => void
  /** 桌面展示台会话清单镜像（null = 未加载/不可用，隐藏入口） */
  sessions?: ShelfSessions | null
  /** 遥控切换桌面当前会话（切的就是电脑端正显示的视图） */
  onSwitchSession?: (id: string, mode?: string) => void
}

/** 设置抽屉视图：单面板内容切换（点选项 → 面板内刷新）
 *  主列表 main / 子视图 model（mode 改主视图手风琴直切，网络与连接走 header
 *  状态 pill 独立弹窗——均已从抽屉移除） */
type SettingsView = 'main' | 'model'

const STATUS_LABEL: Record<WsStatus, () => string> = {
  connecting: () => t('mobile.connecting'),
  online: () => t('mobile.connected'),
  offline: () => t('mobile.disconnected'),
}

/** 字号档位选项（label 供 aria/浮层展示；hint 为预览说明） */
const FONT_SIZE_OPTIONS: { value: MobileFontSize; label: string; hint: string }[] = [
  { value: 'standard', label: t('mobile.fontStandard'), hint: '16' },
  { value: 'large', label: t('mobile.fontLarge'), hint: '17' },
  { value: 'xlarge', label: t('mobile.fontXlarge'), hint: '18' },
]

function formatElapsed(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000))
  const mm = String(Math.floor(totalSec / 60)).padStart(2, '0')
  const ss = String(totalSec % 60).padStart(2, '0')
  return `${mm}:${ss}`
}

/** Nuphus 眨眼 logo（与桌面端 NuphusAvatar 同款 SVG 结构）：
 *  框 = 颅，双竖 = 目（眨眼动画），右下开口 = 嘴。
 *  stroke=currentColor 跟随主题（.mobile-nav-logo color） */
const SHELL_1 = 'M64 20 H192 A44 44 0 0 1 236 64 V156'
const SHELL_2 = 'M200 236 H64 A44 44 0 0 1 20 192 V64 A44 44 0 0 1 64 20'

function BlinkLogo() {
  return (
    <svg
      className="mobile-nav-logo"
      viewBox="0 0 256 256"
      fill="none"
      role="img"
      aria-label="Nuphus"
    >
      <g>
        <path
          d={SHELL_1}
          stroke="currentColor"
          strokeWidth={24}
          strokeLinecap="round"
          fill="none"
        />
        <path
          d={SHELL_2}
          stroke="currentColor"
          strokeWidth={24}
          strokeLinecap="round"
          fill="none"
        />
        <path
          className="nv-eye l"
          d="M80 180 L80 76"
          stroke="currentColor"
          strokeWidth={24}
          strokeLinecap="round"
          fill="none"
        />
        <path
          className="nv-eye r"
          d="M176 180 L176 76"
          stroke="currentColor"
          strokeWidth={24}
          strokeLinecap="round"
          fill="none"
        />
      </g>
    </svg>
  )
}

export default function NavBar({
  wsStatus,
  activity,
  token,
  model,
  tokenUsage,
  connMode,
  onReloadHistory,
  onNewChat,
  onDisconnect,
  sessions,
  onSwitchSession,
}: Props) {
  const [now, setNow] = useState(Date.now())
  const [theme, setTheme] = useState<MobileTheme>(getTheme)
  const [fontSize, setFs] = useState<MobileFontSize>(getFontSize)
  const [fsOpen, setFsOpen] = useState(false)
  const fsWrapRef = useRef<HTMLDivElement>(null)

  // ── 设置抽屉数据：模式 / 模型（全部走后端，与桌面端同源） ──
  const [modelConfig, setModelConfig] = useState<ModelConfig | null>(null)
  const [modelLoading, setModelLoading] = useState(false)
  const [customAgents, setCustomAgents] = useState<CustomAgentBrief[]>([])
  const [activeCustomName, setActiveCustomName] = useState<string | null>(null)
  const currentMode = activity.mode || 'leader'
  // 会话切换锁定：与桌面 rail hardLocked 同一逻辑（canSwitch || locked 双源镜像）——
  // can_switch = 后端 guard_switch 权威守卫（busy/追加挂起，随 /sessions 下发）；
  // activity.running = 执行态实时镜像（对应桌面 locked prop）。锁定时列表可看、
  // 点选禁用（主视图会话列表点选即切换）；sessions 未加载时退回 running 单源，后端守卫仍兜底。
  const sessLocked = activity.running || (sessions ? !sessions.can_switch : false)
  const modeLabel =
    currentMode === 'workflow'
      ? 'Workflow'
      : currentMode === 'custom'
        ? activeCustomName || 'Custom'
        : 'Leader'
  // mode 卡片描述（与 mode 子视图选项 desc 文案一致）
  const modeDesc =
    currentMode === 'workflow'
      ? '解析模板生成可执行工作流'
      : currentMode === 'custom'
        ? '我的专属 Agent'
        : '自主判断路径'

  // 模型配置：进入即主动拉取（不依赖 session_info 事件，与桌面端 config.toml 同源）；
  // mode 变化后也重拉（selectMode 内已处理，此处兜底 token/首次）
  useEffect(() => {
    if (!token) return
    fetchModelConfig(token, currentMode)
      .then(cfg => setModelConfig(cfg))
      .catch(() => setModelConfig(null))
  }, [token, currentMode])

  /** 模式切换：走后端 set_mode（唯一权威源），WS 广播 ModeChanged 双端同步；
   *  主视图手风琴直接切换（不再子视图先选后确定），成功后收起列表。 */
  const selectMode = (m: string) => {
    switchMobileMode(token, m)
      .then(() => {
        setModeOpen(false)
        // 切 mode 后重拉该 mode 生效模型
        fetchModelConfig(token, m)
          .then(cfg => setModelConfig(cfg))
          .catch(() => {})
      })
      .catch(() => {})
  }

  /** 新会话弹窗（设置抽屉 header「+」）：屏幕中心弹窗选 mode → 切 mode → 回 welcome。
   *  后端链路与电脑端统一：set_mode（广播 ModeChanged）→ POST /new-chat（纯意图广播，
   *  桌面 handleNewChat 跟随回欢迎页，后端零会话创建）。失败静默关闭（与 selectMode 一致）。 */
  const openNewChatModal = () => {
    setPickedMode(currentMode)
    setNewChatOpen(true)
  }

  const startNewChatWithMode = (m: string) => {
    if (activity.running) return
    switchMobileMode(token, m)
      .then(() => {
        setNewChatOpen(false)
        closeSettings()
        onNewChat?.()
      })
      .catch(() => setNewChatOpen(false))
  }

  /** 打开模型视图：拉取配置（不缓存——桌面端改配置后立即可见），
   *  并初始化暂存为当前生效模型（先选后确定），手风琴默认展开当前模型所在提供商组 */
  const openModelView = () => {
    setSettingsView('model')
    const cur = modelConfig?.models?.find(m => m.id === modelConfig.current)
    setPendingModel(cur ? { id: cur.id, provider: cur.provider } : null)
    setExpandedProvider(cur?.provider ?? null)
    if (!modelConfig) {
      setModelLoading(true)
      fetchModelConfig(token, currentMode)
        .then(cfg => {
          setModelConfig(cfg)
          const c = cfg.models?.find(m => m.id === cfg.current)
          setExpandedProvider(prev => prev ?? c?.provider ?? null)
        })
        .catch(() => setModelConfig(null))
        .finally(() => setModelLoading(false))
    }
  }

  /** 模型切换：provider-driven（后端读 config.toml），成功后回主列表并刷新配置 */
  const switchToModel = async (id: string, provider: string) => {
    try {
      await switchMobileModel(token, id, provider, currentMode)
      const cfg = await fetchModelConfig(token, currentMode)
      setModelConfig(cfg)
      setSettingsView('main')
    } catch {
      /* toast 由上层反馈（模型行点击后可见状态） */
    }
  }
  // 设置抽屉（点击 logo 直接侧滑）——单面板 + 视图切换：
  // main（mode 手风琴卡 + 模型卡 + 会话列表）/ model（子视图）/ network（header 弹窗）
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [settingsView, setSettingsView] = useState<SettingsView>('main')
  const logoWrapRef = useRef<HTMLDivElement>(null)
  const settingsSheetRef = useRef<HTMLDivElement>(null)

  // 右滑关闭手势：抽屉为右侧滑出，向右滑动（水平位移 > 60px 且垂直位移小，
  // 防纵向滚动误触发）→ 关闭。主视图无 X，右滑是主关闭手势之一（外部点击亦可）。
  const swipeStartXRef = useRef<number | null>(null)
  const swipeStartYRef = useRef<number | null>(null)
  const onSheetTouchStart = (e: React.TouchEvent) => {
    swipeStartXRef.current = e.touches[0].clientX
    swipeStartYRef.current = e.touches[0].clientY
  }
  const onSheetTouchEnd = (e: React.TouchEvent) => {
    if (swipeStartXRef.current == null || swipeStartYRef.current == null) return
    const dx = e.changedTouches[0].clientX - swipeStartXRef.current
    const dy = e.changedTouches[0].clientY - swipeStartYRef.current
    swipeStartXRef.current = null
    swipeStartYRef.current = null
    if (dx > 60 && Math.abs(dy) < 40) closeSettings()
  }

  // mode 手风琴（主视图直接切换，不再子视图）：展开显示 Leader/Workflow/Custom
  const [modeOpen, setModeOpen] = useState(false)

  // 自定义 Agent：拉取列表 + 激活卡片名（卡片管理在桌面端）。
  // 刷新时机三触发：token（挂载/重连）、settingsOpen（打开设置抽屉——桌面端换卡后
  // 手机端打开即取最新名，双端一致）、currentMode（ModeChanged 切 mode 场景）。
  // 桌面端 set_active_custom_agent 无事件广播（custom_agent.rs 只刷新 runtime），
  // 故以「抽屉打开 + mode 变化」两个高频交互点兜底，避免 stale 卡片名显示。
  useEffect(() => {
    if (!token) return
    fetchCustomAgents(token)
      .then(info => {
        setCustomAgents(info.agents || [])
        setActiveCustomName(info.active?.name ?? null)
      })
      .catch(() => {})
  }, [token, settingsOpen, currentMode])

  // 新会话弹窗（header「+」→ 中心弹窗选 mode）：pickedMode 先选后确定
  const [newChatOpen, setNewChatOpen] = useState(false)
  const [pickedMode, setPickedMode] = useState<string>('leader')
  // 网络与连接弹窗（header 状态 pill 点击 → 独立中心弹窗，已从抽屉移除）
  const [networkOpen, setNetworkOpen] = useState(false)

  // 子视图「先选后确定」暂存值：model 子视图先选后确定（mode 已改主视图直切，无暂存）
  const [pendingModel, setPendingModel] = useState<{ id: string; provider: string } | null>(null)

  // 模型子视图：当前展开的提供商组（手风琴——同一时刻只展开一组；null=全部收起）
  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)

  /** 关闭抽屉：统一重置回主列表（避免下次打开停留在子视图） */
  const closeSettings = () => {
    setSettingsView('main')
    setSettingsOpen(false)
  }

  /** 返回主视图（header ← / 底部「返回」共用）：放弃子视图未确认的暂存选择 */
  const backToMain = () => {
    setPendingModel(null)
    setSettingsView('main')
  }

  /** 子视图底部「确定」：按视图提交暂存选择（模型走后端权威入口） */
  const confirmView = () => {
    if (settingsView === 'model' && pendingModel) {
      void switchToModel(pendingModel.id, pendingModel.provider)
    }
  }

  // 执行中每秒 tick 刷新用时；空闲/暂停时不 tick
  const running = activity.running && !activity.paused
  useEffect(() => {
    if (!running) return
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [running])

  // 点击字号浮层外部时关闭（轻量 popover：点外部即收起，不叠加遮罩层）
  useEffect(() => {
    if (!fsOpen) return
    const onDown = (e: MouseEvent | TouchEvent) => {
      if (fsWrapRef.current && !fsWrapRef.current.contains(e.target as Node)) {
        setFsOpen(false)
      }
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('touchstart', onDown)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('touchstart', onDown)
    }
  }, [fsOpen])

  // 设置抽屉：点击外部关闭（轻量 popover，不叠加遮罩）
  useEffect(() => {
    if (!settingsOpen) return
    const onDown = (e: MouseEvent | TouchEvent) => {
      const t = e.target as Node
      // logo 区域排除：抽屉开着时点 logo 走 toggle 关闭（不先触发外部关闭）
      if (logoWrapRef.current?.contains(t)) return
      if (settingsSheetRef.current?.contains(t)) return
      closeSettings()
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('touchstart', onDown)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('touchstart', onDown)
    }
  }, [settingsOpen])

  const pickFontSize = (size: MobileFontSize) => {
    setFs(setFontSize(size))
    setFsOpen(false)
  }

  /** 品牌区：点击 logo 直接侧滑设置抽屉（不再弹中间菜单——大王裁定多余步骤） */
  const renderBrand = () => (
    <div className="mobile-nav-brand-wrap" ref={logoWrapRef}>
      <button
        type="button"
        className="mobile-nav-brand"
        onClick={() => {
          if (settingsOpen) setSettingsView('main')
          setSettingsOpen(o => !o)
        }}
        aria-label="菜单"
        aria-haspopup="dialog"
        aria-expanded={settingsOpen}
      >
        <BlinkLogo />
        <span className="mobile-nav-title">Nuphus</span>
      </button>
    </div>
  )

  /** 设置抽屉：点击 logo 直接侧滑；单面板 + 视图切换（点选项 → 面板内刷新内容）。
   *  结构：头部（logo 信息 / 返回+标题 / 关闭）+ 中间设置选项 + 下方特别设置。
   *  后期登录入口：加在「特别设置」上方（mobile-settings-group「账号」），
   *  数据走后端会话/用户 API，与桌面端同源。 */

  // ── 实时模型信息卡（设置抽屉主视图 header 下；反色舒展，实时同步）──
  // 与桌面 ChatInputBar ctx 弹窗同源同字段：ctxUsed=token_usage 事件实时累计 input；
  // cap=model-config.contextWindow（0=未知显示 --，不伪装 128000 假数）；
  // cache=命中率三档色（>60 绿 / >30 黄 / 红）。模型名=modelConfig.current
  // （后端 effective_model 单点解析）或 store.model（session_info 事件下发）。
  const ctxUsed = tokenUsage?.inputTokens || 0
  const ctxCap = modelConfig?.contextWindow || 0
  const ctxPct = ctxCap > 0 ? Math.min(ctxUsed / ctxCap, 1) : 0
  const ctxColor = ctxPct > 0.8 ? '#ef4444' : ctxPct > 0.6 ? '#f59e0b' : '#22c55e'
  const cacheTotal = tokenUsage?.inputTokens || 0
  const cacheRate =
    cacheTotal > 0 && typeof tokenUsage?.cacheHitTokens === 'number'
      ? (tokenUsage.cacheHitTokens / cacheTotal) * 100
      : -1
  const cacheColor = cacheRate > 60 ? '#22c55e' : cacheRate > 30 ? '#f59e0b' : '#ef4444'
  const fmtTokens = (n: number): string => {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
    return String(n)
  }
  // step（工具调用步数）= 当前执行累计工具调用（tool_call_start 事件累积）；
  // time（执行用时）= startedAt 起算，暂停冻结在 pausedAt（与执行状态条同款）。
  const execSteps = activity.tools.length
  const elapsedBase = activity.pausedAt ?? now
  const execDuration = activity.startedAt ? elapsedBase - activity.startedAt : 0

  const renderSettings = () =>
    settingsOpen ? (
      <div
        className="mobile-settings-sheet"
        role="dialog"
        aria-label="设置"
        ref={settingsSheetRef}
        onTouchStart={onSheetTouchStart}
        onTouchEnd={onSheetTouchEnd}
      >
        {/* 头部：主列表=logo 信息（品牌）| 子视图=返回 + 标题；右侧统一关闭 */}
        <div className="mobile-mode-head">
          {settingsView === 'main' ? (
            <span className="mobile-settings-brand">
              <BlinkLogo />
              <span className="mobile-settings-brand-name">Nuphus</span>
              <span className={`mobile-settings-badge is-${wsStatus}`} role="status">
                <span className="mobile-settings-badge-dot" aria-hidden="true" />
                {STATUS_LABEL[wsStatus]()}
              </span>
            </span>
          ) : (
            <button
              type="button"
              className="mobile-settings-back"
              onClick={backToMain}
              aria-label="返回"
            >
              <ChevronLeft size={18} aria-hidden="true" />
              <span className="mobile-mode-title">模型</span>
            </button>
          )}
          <div className="mobile-mode-head-actions">
            <button
              type="button"
              className="mobile-settings-newchat"
              onClick={openNewChatModal}
              disabled={activity.running}
              aria-label={t('mobile.newChat')}
            >
              <Plus size={15} aria-hidden="true" />
              <span>{t('mobile.newChat')}</span>
            </button>
            {/* 主视图无 X 关闭（右滑关闭 + 外部点击关闭）；子视图保留 X */}
            {settingsView !== 'main' && (
              <button
                type="button"
                className="mobile-model-card-x"
                onClick={closeSettings}
                aria-label="关闭"
              >
                <X size={16} aria-hidden="true" />
              </button>
            )}
          </div>
        </div>
        {/* 内容区：唯一可滚动区域（header/footer 固定不跳动） */}
        <div className="mobile-settings-body">
          {settingsView === 'main' ? (
            <>
              {/* 实时模型信息卡（反色舒展）：header 下方第一块。数据源=桌面输入框模型
                + ctx hover 同源同字段：模型名=effective_model 单点；ctx 进度条=桌面
                5 格 gauge 同款，已用/容量=token_usage WS 实时累计 input + model-config
                contextWindow（0=未知显示--）；cache/step/time=命中率/工具步数/执行用时。
                WS 事件驱动实时同步。 */}
              {/* 运行模式卡（模型卡上方；样式仿模型信息卡）：手风琴直接切换——
                头部点开/收起模式列表（Leader/Workflow/Custom 点击即切，与桌面 mode
                铭牌同源）；「当前模式」徽章放在介绍前做引导，箭头下标即展开入口 */}
              <div className="mobile-mode-info">
                <button
                  type="button"
                  className="mobile-mode-info-head"
                  onClick={() => setModeOpen(o => !o)}
                  aria-expanded={modeOpen}
                >
                  <span className="mobile-mode-info-name">{modeLabel}</span>
                  <ChevronDown
                    size={16}
                    className={['mobile-mode-info-arrow', modeOpen ? 'is-open' : '']
                      .filter(Boolean)
                      .join(' ')}
                    aria-hidden="true"
                  />
                </button>
                {modeOpen && (
                  <div className="mobile-mode-info-list" role="radiogroup" aria-label="模式">
                    <button
                      type="button"
                      role="radio"
                      aria-checked={currentMode === 'leader'}
                      className={[
                        'mobile-mode-info-opt',
                        currentMode === 'leader' ? 'is-active' : '',
                      ]
                        .filter(Boolean)
                        .join(' ')}
                      onClick={() => selectMode('leader')}
                    >
                      <span className="mobile-mode-info-opt-name">Leader</span>
                      <span className="mobile-mode-info-opt-desc">自主判断路径</span>
                      {currentMode === 'leader' && <Check size={15} aria-hidden="true" />}
                    </button>
                    <button
                      type="button"
                      role="radio"
                      aria-checked={currentMode === 'workflow'}
                      className={[
                        'mobile-mode-info-opt',
                        currentMode === 'workflow' ? 'is-active' : '',
                      ]
                        .filter(Boolean)
                        .join(' ')}
                      onClick={() => selectMode('workflow')}
                    >
                      <span className="mobile-mode-info-opt-name">Workflow</span>
                      <span className="mobile-mode-info-opt-desc">解析模板生成可执行工作流</span>
                      {currentMode === 'workflow' && <Check size={15} aria-hidden="true" />}
                    </button>
                    {activeCustomName ? (
                      <button
                        type="button"
                        role="radio"
                        aria-checked={currentMode === 'custom'}
                        className={[
                          'mobile-mode-info-opt',
                          currentMode === 'custom' ? 'is-active' : '',
                        ]
                          .filter(Boolean)
                          .join(' ')}
                        onClick={() => selectMode('custom')}
                      >
                        <span className="mobile-mode-info-opt-name">{activeCustomName}</span>
                        <span className="mobile-mode-info-opt-desc">我的专属 Agent</span>
                        {currentMode === 'custom' && <Check size={15} aria-hidden="true" />}
                      </button>
                    ) : (
                      <div className="mobile-mode-info-note">自定义 Agent 请在桌面端创建</div>
                    )}
                  </div>
                )}
                <div className="mobile-mode-info-desc-row">
                  <span className="mobile-mode-info-badge">{t('mobile.currentMode')}</span>
                  <span className="mobile-mode-info-desc">{modeDesc}</span>
                </div>
              </div>
              <div className="mobile-model-info">
                <div className="mobile-model-info-head">
                  <span className="mobile-model-info-name">
                    {model || modelConfig?.current || '—'}
                  </span>
                  {ctxCap > 0 && <span className="mobile-model-info-cap">{fmtTokens(ctxCap)}</span>}
                </div>
                <div className="mobile-model-info-ctx">
                  <span className="mobile-model-info-ctx-label">ctx</span>
                  <span className="mobile-model-info-ctx-gauge" aria-hidden>
                    {Array.from({ length: 5 }).map((_, i) => (
                      <span
                        key={i}
                        className="mobile-model-info-ctx-cell"
                        style={i < Math.round(ctxPct * 5) ? { background: ctxColor } : undefined}
                      />
                    ))}
                  </span>
                  <span className="mobile-model-info-ctx-usage">
                    {ctxCap > 0
                      ? `${fmtTokens(ctxUsed)}/${fmtTokens(ctxCap)}`
                      : `${fmtTokens(ctxUsed)}/--`}
                  </span>
                  <span className="mobile-model-info-pct" style={{ color: ctxColor }}>
                    {ctxCap > 0 ? `${Math.round(ctxPct * 100)}%` : '--'}
                  </span>
                </div>
                <div className="mobile-model-info-foot">
                  <div className="mobile-model-info-meta">
                    {cacheRate >= 0 && (
                      <span style={{ color: cacheColor }}>cache {cacheRate.toFixed(0)}%</span>
                    )}
                    <span>step {execSteps}</span>
                    <span>time {formatElapsed(execDuration)}</span>
                  </div>
                  <button
                    type="button"
                    className="mobile-model-info-switch"
                    onClick={openModelView}
                  >
                    {t('mobile.switchModel')}
                  </button>
                </div>
              </div>
              {/* 会话列表：主视图直接展示（不再子视图）——点选即切换电脑端视图，
                与桌面 rail 同语义；执行中/后端守卫锁定时禁用 */}
              <div className="mobile-sess-section">
                <div className="mobile-settings-group">会话</div>
                {sessions && sessions.items.length > 0 ? (
                  <div className="mobile-sess-list">
                    {sessions.items.map(item => (
                      <button
                        key={item.id}
                        type="button"
                        className={['mobile-sess-item', item.is_active ? 'is-active' : '']
                          .filter(Boolean)
                          .join(' ')}
                        disabled={sessLocked}
                        onClick={() => onSwitchSession?.(item.id, item.mode)}
                      >
                        <span className="mobile-sess-title">
                          {item.mode && (
                            <span
                              className={`mobile-sess-mode mode-${item.mode}`}
                              aria-hidden="true"
                            >
                              {item.mode.toUpperCase()}
                            </span>
                          )}
                          {item.title || item.id}
                        </span>
                        <span className="mobile-sess-meta">
                          {item.is_active ? '当前 · ' : ''}
                          {activity.running && !item.is_active ? '执行中锁定 · ' : ''}
                          {item.message_count} 条
                        </span>
                      </button>
                    ))}
                  </div>
                ) : (
                  <div className="mobile-sess-empty">暂无会话记录</div>
                )}
                <div className="mobile-settings-reset-hint">
                  切换的是电脑端正在显示的对话，两端同步
                </div>
              </div>
            </>
          ) : (
            <>
              <div className="mobile-settings-sub-note">
                模型与桌面端同源配置；选择后点「确定」生效
              </div>
              {modelLoading && <div className="mobile-settings-sub-note">加载中…</div>}
              {!modelLoading && (!modelConfig?.models || modelConfig.models.length === 0) && (
                <div className="mobile-settings-sub-note">暂无模型，请先在桌面端配置</div>
              )}
              {!modelLoading &&
                modelConfig &&
                modelConfig.models.length > 0 &&
                // 按 provider 分组（保持首次出现顺序）；手风琴——同一时刻只展开一组
                [...new Map(modelConfig.models.map(m => [m.provider, m.provider])).keys()].map(
                  provider => {
                    const models = modelConfig.models.filter(m => m.provider === provider)
                    const open = expandedProvider === provider
                    return (
                      <div key={provider} className="mobile-model-group">
                        <button
                          type="button"
                          className={['mobile-model-group-head', open ? 'is-open' : '']
                            .filter(Boolean)
                            .join(' ')}
                          onClick={() => setExpandedProvider(open ? null : provider)}
                          aria-expanded={open}
                        >
                          <span className="mobile-model-group-name">{provider || '未分类'}</span>
                          <span className="mobile-model-group-count">{models.length}</span>
                          <ChevronDown
                            size={15}
                            className={['mobile-model-group-arrow', open ? 'is-open' : '']
                              .filter(Boolean)
                              .join(' ')}
                            aria-hidden="true"
                          />
                        </button>
                        {open && (
                          <div
                            className="mobile-model-group-body"
                            role="group"
                            aria-label={provider}
                          >
                            {models.map(m => (
                              <button
                                key={m.id}
                                type="button"
                                className={`mobile-settings-sub-item${(pendingModel?.id ?? modelConfig.current) === m.id ? ' is-active' : ''}`}
                                onClick={() => setPendingModel({ id: m.id, provider: m.provider })}
                              >
                                <span className="mobile-settings-sub-name">
                                  {m.alias?.[0] || m.id}
                                </span>
                                {(pendingModel?.id ?? modelConfig.current) === m.id && (
                                  <Check size={15} aria-hidden="true" />
                                )}
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                    )
                  },
                )}
            </>
          )}
        </div>
        {/* 子视图底部操作栏：返回（回主视图，放弃未确认选择）+ 确定（提交暂存选择） */}
        {settingsView !== 'main' && (
          <div className="mobile-settings-footer">
            <button
              type="button"
              className="mobile-settings-footer-btn is-back"
              onClick={backToMain}
            >
              <ChevronLeft size={16} aria-hidden="true" /> 返回
            </button>
            <button
              type="button"
              className="mobile-settings-footer-btn is-confirm"
              disabled={settingsView === 'model' && !pendingModel}
              onClick={confirmView}
            >
              确定
            </button>
          </div>
        )}
      </div>
    ) : null

  /** 新会话弹窗：屏幕中心 modal——选 mode（先选后确定）→「开始」= 切 mode（set_mode 权威源）
   *  + 遥控 /new-chat 广播（桌面 handleNewChat 跟随回 welcome）。与桌面 Ctrl+N 统一语义：
   *  不创建空会话，只回欢迎页；后端零会话创建，会话只在 welcome 直发时 force_new。 */
  const renderNewChatModal = () => (
    <div className="mobile-newchat-overlay" onClick={() => setNewChatOpen(false)}>
      <div
        className="mobile-newchat-modal"
        role="dialog"
        aria-modal="true"
        aria-label={t('mobile.newChat')}
        onClick={e => e.stopPropagation()}
      >
        <div className="mobile-newchat-title">{t('mobile.newChat')}</div>
        <div className="mobile-newchat-sub">{t('mobile.newChatPickMode')}</div>
        <div className="mobile-newchat-modes" role="radiogroup" aria-label="模式">
          <button
            type="button"
            role="radio"
            aria-checked={pickedMode === 'leader'}
            className={`mobile-newchat-mode${pickedMode === 'leader' ? ' is-active' : ''}`}
            onClick={() => setPickedMode('leader')}
          >
            <span className="mobile-newchat-mode-name">Leader</span>
            <span className="mobile-newchat-mode-desc">自主判断路径</span>
            {pickedMode === 'leader' && <Check size={16} aria-hidden="true" />}
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={pickedMode === 'workflow'}
            className={`mobile-newchat-mode${pickedMode === 'workflow' ? ' is-active' : ''}`}
            onClick={() => setPickedMode('workflow')}
          >
            <span className="mobile-newchat-mode-name">Workflow</span>
            <span className="mobile-newchat-mode-desc">解析模板生成可执行工作流</span>
            {pickedMode === 'workflow' && <Check size={16} aria-hidden="true" />}
          </button>
          {activeCustomName ? (
            <button
              type="button"
              role="radio"
              aria-checked={pickedMode === 'custom'}
              className={`mobile-newchat-mode${pickedMode === 'custom' ? ' is-active' : ''}`}
              onClick={() => setPickedMode('custom')}
            >
              <span className="mobile-newchat-mode-name">{activeCustomName}</span>
              <span className="mobile-newchat-mode-desc">我的专属 Agent</span>
              {pickedMode === 'custom' && <Check size={16} aria-hidden="true" />}
            </button>
          ) : (
            <div className="mobile-newchat-mode-note">自定义 Agent 请在桌面端创建</div>
          )}
        </div>
        <div className="mobile-newchat-actions">
          <button
            type="button"
            className="mobile-newchat-btn is-cancel"
            onClick={() => setNewChatOpen(false)}
          >
            {t('mobile.newChatCancel')}
          </button>
          <button
            type="button"
            className="mobile-newchat-btn is-start"
            disabled={activity.running}
            onClick={() => startNewChatWithMode(pickedMode)}
          >
            {t('mobile.newChatStart')}
          </button>
        </div>
      </div>
    </div>
  )

  /** 网络与连接弹窗：header 状态 pill 点击打开（已从设置抽屉移除）——
   *  历史拉取即时生效；连接信息只读 + 一键复制；重置配对为危险操作（二次确认） */
  const renderNetworkModal = () => (
    <div className="mobile-newchat-overlay" onClick={() => setNetworkOpen(false)}>
      <div
        className="mobile-newchat-modal"
        role="dialog"
        aria-modal="true"
        aria-label="网络与连接"
        onClick={e => e.stopPropagation()}
      >
        <div className="mobile-newchat-title">网络与连接</div>
        <div className="mobile-newchat-sub">连接信息与网络操作；历史拉取即时生效</div>
        <div className="mobile-network-body">
          <div className="mobile-settings-card">
            <button
              type="button"
              className="mobile-settings-card-row is-secondary"
              onClick={() => {
                setNetworkOpen(false)
                onReloadHistory?.()
              }}
            >
              <RefreshCw size={14} className="mobile-settings-card-icon" aria-hidden="true" />
              <span className="mobile-settings-card-title is-secondary">历史记录</span>
              <span className="mobile-settings-card-value is-action">拉取同步</span>
            </button>
            <div className="mobile-settings-conn-list">
              <div className="mobile-settings-conn-item">
                <span className="mobile-settings-conn-label is-direct">直连</span>
                <span className="mobile-settings-conn-ip">{getCachedLanUrl() || '局域网直连'}</span>
                <button
                  type="button"
                  className="mobile-settings-conn-copy"
                  onClick={() => void navigator.clipboard?.writeText(getCachedLanUrl() || '')}
                >
                  复制
                </button>
              </div>
              {getCachedRelayUrl() && (
                <div className="mobile-settings-conn-item">
                  <span className="mobile-settings-conn-label is-relay">中继</span>
                  <span className="mobile-settings-conn-ip">{getCachedRelayUrl()}</span>
                  <button
                    type="button"
                    className="mobile-settings-conn-copy"
                    onClick={() => void navigator.clipboard?.writeText(getCachedRelayUrl() || '')}
                  >
                    复制
                  </button>
                </div>
              )}
            </div>
          </div>

          {/* 危险区：重置配对（误触即断连回配对页）——独立弹窗仍加二次确认双保险 */}
          <div className="mobile-settings-danger">
            <button
              type="button"
              className="mobile-settings-reset-btn"
              onClick={() => {
                if (
                  !window.confirm(
                    '确定重置客户端配对？将清除本地配对并断开连接，需重新扫码关联设备',
                  )
                )
                  return
                setNetworkOpen(false)
                onDisconnect?.()
              }}
            >
              重置客户端配对
            </button>
            <p className="mobile-settings-tip">清除本地配对后需重新扫码关联设备</p>
          </div>
        </div>
        <div className="mobile-newchat-actions">
          <button
            type="button"
            className="mobile-newchat-btn is-cancel"
            onClick={() => setNetworkOpen(false)}
          >
            关闭
          </button>
        </div>
      </div>
    </div>
  )

  const renderActions = () => (
    <div className="mobile-nav-right">
      {/* 文字大小：点击弹出三档选择（标准 16 / 大 17 / 特大 18），
          localStorage 记忆，作用于消息正文（对齐主题切换模式） */}
      <div className="mobile-nav-fs-wrap" ref={fsWrapRef}>
        <button
          type="button"
          className="mobile-nav-theme"
          aria-label={t('mobile.fontSize')}
          aria-haspopup="menu"
          aria-expanded={fsOpen}
          onClick={() => setFsOpen(o => !o)}
        >
          <ALargeSmall size={16} aria-hidden="true" />
        </button>
        {fsOpen && (
          <div className="mobile-nav-fs-pop" role="menu" aria-label={t('mobile.fontSize')}>
            <div className="mobile-nav-fs-title">{t('mobile.fontSize')}</div>
            {FONT_SIZE_OPTIONS.map(opt => (
              <button
                key={opt.value}
                type="button"
                role="menuitemradio"
                aria-checked={fontSize === opt.value}
                className={['mobile-nav-fs-opt', fontSize === opt.value ? 'is-active' : '']
                  .filter(Boolean)
                  .join(' ')}
                onClick={() => pickFontSize(opt.value)}
              >
                <span className="mobile-nav-fs-sample" aria-hidden="true">
                  Aa
                </span>
                <span className="mobile-nav-fs-label">{opt.label}</span>
                <span className="mobile-nav-fs-hint">{opt.hint}</span>
                {fontSize === opt.value && (
                  <Check size={14} className="mobile-nav-fs-check" aria-hidden="true" />
                )}
              </button>
            ))}
          </div>
        )}
      </div>
      <button
        type="button"
        className="mobile-nav-theme"
        aria-label={theme === 'dark' ? t('mobile.switchToLight') : t('mobile.switchToDark')}
        onClick={() => setTheme(toggleTheme())}
      >
        {theme === 'dark' ? (
          <Sun size={16} aria-hidden="true" />
        ) : (
          <Moon size={16} aria-hidden="true" />
        )}
      </button>
    </div>
  )

  // 执行中：工具调用 + 实时用时（暂停冻结在 pausedAt）
  if (activity.running) {
    const currentTool =
      activity.tools.length > 0 ? activity.tools[activity.tools.length - 1].name : ''
    const elapsedBase = activity.pausedAt ?? now
    const elapsedMs = activity.startedAt ? elapsedBase - activity.startedAt : 0
    const label = activity.paused
      ? `${t('mobile.paused')} ${formatElapsed(elapsedMs)}`
      : `${currentTool || t('mobile.executing')} ${formatElapsed(elapsedMs)}`
    return (
      <>
        <header className="mobile-nav">
          {renderBrand()}
          <div className="mobile-nav-center">
            <span className="mobile-status-pill is-running" role="status">
              <Wrench size={12} aria-hidden="true" />
              {label}
            </span>
          </div>
          {renderActions()}
        </header>
        {renderSettings()}
      </>
    )
  }

  // 空闲：连接状态（居中锚点）。pill 即网络与连接入口——显示当前渠道（局域网/中继）
  // + 状态色（绿=在线 / 黄=连接中 / 红=断开），点击进入设置抽屉 network 子视图。
  return (
    <>
      <header className="mobile-nav">
        {renderBrand()}
        <div className="mobile-nav-center">
          <button
            type="button"
            className={`mobile-status-pill is-${wsStatus}${connMode ? ` is-${connMode}` : ''}`}
            onClick={() => setNetworkOpen(true)}
            aria-label="网络与连接"
          >
            <span className="mobile-status-dot" aria-hidden="true" />
            {connMode === 'lan'
              ? `${t('mobile.connLan')} · `
              : connMode === 'wan'
                ? `${t('mobile.connRelay')} · `
                : ''}
            {STATUS_LABEL[wsStatus]()}
          </button>
        </div>
        {renderActions()}
      </header>
      {renderSettings()}
      {newChatOpen && renderNewChatModal()}
      {networkOpen && renderNetworkModal()}
    </>
  )
}
