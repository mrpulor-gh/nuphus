/**
 * 顶部导航条：品牌（logo + 字标）| 状态 pill（居中锚点）| 操作按钮
 * - 空闲：中间显示连接状态（已连接/连接中/已断开）
 * - 执行中：中间显示当前工具调用 + 实时用时（用户能直接看到在执行中；
 *   长时间任务 WS 可能显示断开，但用时仍在走，避免误判）
 * - 连接状态居中：状态是用户高频关注的信息，固定中间形成记忆锚点，
 *   右侧仅保留操作按钮（主题切换等）
 * iOS HIG：44pt 高度、17px/600 标题、hairline 下沿分隔、安全区上沿适配
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { ALargeSmall as ALargeSmallIcon, Moon as MoonIcon, Sun as SunIcon } from 'lucide'
import {
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Plus,
  RefreshCw,
  Wifi,
  Wrench,
  X,
} from 'lucide-react'
import { MorphIcon } from 'morphicons/react'
import { getTheme, toggleTheme, type MobileTheme } from '../theme'
import { getCachedLanUrl, getCachedRelayUrl, type ConnectionMode } from '../connection'
import { getFontSize, setFontSize, type MobileFontSize } from '../fontsize'
import {
  fetchCustomAgents,
  fetchModelConfig,
  setMobileReasoningEffort,
  switchMobileMode,
  switchMobileModel,
  type CustomAgentBrief,
  type ModelConfig,
  type ModelInfo,
  type ShelfSessions,
} from '../api'
import type { ActivityState } from '../store'
import { formatTokens } from '../../ui/tokenFormat'
import type { WsStatus } from '../ws'
import {
  DEFAULT_GROUP_LIMIT,
  buildSessionGroups,
  normalizeGroupLimit,
  normalizeSessionSortPrefs,
  visibleGroupSessions,
} from '../../main-window/chat/sessionGroups'
import { t } from '../i18n'
import ServerAddressControl from './ServerAddressControl'

/** 移动端主导航页（底部 Tab 投影，页面本体由本组件渲染） */
export type MobilePage = 'chat' | 'sessions' | 'settings'

interface Props {
  wsStatus: WsStatus
  activity: ActivityState
  /** 当前主导航页 */
  page: MobilePage
  /** 切换主导航页（TabBar / 页内跳转共用） */
  onPageChange: (page: MobilePage) => void
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
  /** 中继通道手动「切换到本地网络」：整页跳转 http 局域网（NavBar 网络弹窗中展示，仅 wan） */
  onSwitchToLanManual?: () => void
  /** header wifi 图标：打开切本地网络确认弹窗（仅 wan 展示；样式同主题/字体图标钮） */
  onLanSwitchRequest?: () => void
  /** 手动切本地请求进行中（跳转解析中；按钮 disabled + busy 提示） */
  lanSwitching?: boolean
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

/** updated_at（Unix 毫秒）→ 行尾相对时间；与桌面 SessionRail.relativeTime 同文案同阈值 */
function relativeTime(ms?: number): string {
  if (!ms) return ''
  const minutes = Math.floor((Date.now() - ms) / 60_000)
  if (minutes < 1) return t('sessionRail.timeJustNow')
  if (minutes < 60) return t('sessionRail.timeMinutes', String(minutes))
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return t('sessionRail.timeHours', String(hours))
  return t('sessionRail.timeDays', String(Math.floor(hours / 24)))
}

/** mode → 单字母标识（与桌面 modeToLetter 同一语义：L / W / C） */
function modeToLetter(mode: string): string {
  if (mode === 'workflow') return 'W'
  if (mode === 'leader') return 'L'
  if (mode === 'custom') return 'C'
  return '·'
}

/** Nuphus 眨眼 logo（与桌面端 NuphusAvatar 同款 SVG 结构）：
 *  框 = 颅，双竖 = 目（眨眼动画），右下开口 = 嘴。
 *  stroke=currentColor 跟随主题（.mobile-nav-logo color）
 *
 *  ⚠ 品牌图形，非语义图标 → 保留手写 SVG，不迁 lucide（lucide 无此形状）。 */
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
  page,
  onPageChange,
  token,
  model,
  tokenUsage,
  connMode,
  onReloadHistory,
  onNewChat,
  onDisconnect,
  sessions,
  onSwitchSession,
  onSwitchToLanManual,
  onLanSwitchRequest,
  lanSwitching,
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
  // 会话切换锁定：单一来源 = 后端执行态 stage（/sessions 下发，与桌面 rail/输入栏同源）。
  // 收敛前是 `activity.running || !can_switch`——把「WS 事件镜像」与「后端切换守卫结论」
  // 两个不同来源 OR 起来（append_pending 残留会让列表永久锁死）。锁定时列表可看、点选禁用。
  // sessions 未加载时退回事件通道的 activity.running：它是**同一后端执行态**的另一条推送
  // 通道投影，不是第二个来源（can_switch 已不参与锁定）。
  const sessStage = sessions?.stage ?? (activity.running ? 'running' : 'idle')
  const sessLocked = sessStage !== 'idle'
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
    // 执行中（含收尾期）不可「先切 mode 再新建」：切 mode 会被后端执行态判定拒绝，
    // 保持与前一处 disabled 同一判定源（sessions.stage / 同源 WS 投影）。
    if (sessLocked) return
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
    setModelSearch('')
    setModelError(null)
    setEffortTouched(false)
    modelDirtyRef.current = false
    const cur = findCurrentModel(modelConfig)
    setPendingModel(cur ? { id: cur.id, provider: cur.provider } : null)
    setPendingEffort(
      cur
        ? (modelConfig?.providerEfforts?.[cur.provider] ?? modelConfig?.reasoningEffort ?? null)
        : null,
    )
    setExpandedProvider(cur?.provider ?? null)
    // 每次进入都重新拉取（桌面端可能刚改过配置/模型列表）；旧数据先展示，
    // 返回后无用户操作才用最新配置覆盖暂存。
    if (!modelConfig) setModelLoading(true)
    const seq = ++modelFetchSeqRef.current
    fetchModelConfig(token, currentMode)
      .then(cfg => {
        if (seq !== modelFetchSeqRef.current) return
        setModelConfig(cfg)
        if (modelDirtyRef.current) return
        const c = findCurrentModel(cfg)
        setExpandedProvider(c?.provider ?? null)
        setPendingModel(c ? { id: c.id, provider: c.provider } : null)
        setPendingEffort(
          c ? (cfg.providerEfforts?.[c.provider] ?? cfg.reasoningEffort ?? null) : null,
        )
      })
      .catch(() => {
        if (seq !== modelFetchSeqRef.current) return
        if (!modelConfig) setModelConfig(null)
        setModelError('模型配置读取失败，请重试')
      })
      .finally(() => setModelLoading(false))
  }

  /** 当前生效模型匹配：同名模型可能跨 provider，优先用 currentProvider 精确匹配 */
  function findCurrentModel(cfg: ModelConfig | null): ModelInfo | undefined {
    if (!cfg) return undefined
    return cfg.models?.find(
      m => m.id === cfg.current && (!cfg.currentProvider || m.provider === cfg.currentProvider),
    )
  }

  /** 选择暂存模型：跨 provider 时同步该 provider 的已持久化强度（未修改则不覆盖） */
  const choosePendingModel = (m: ModelInfo) => {
    setModelError(null)
    modelDirtyRef.current = true
    if (pendingModel?.provider !== m.provider) {
      setPendingEffort(modelConfig?.providerEfforts?.[m.provider] ?? null)
      setEffortTouched(false)
    }
    setPendingModel({ id: m.id, provider: m.provider })
  }

  /** 模型切换：provider-driven（后端读 config.toml），成功后回主列表并刷新配置。
   *  先切模型再写 effort：模型切换失败时不会留下半套 effort 状态。 */
  const switchToModel = async (
    id: string,
    provider: string,
    effort: string | null,
    saveEffort: boolean,
  ) => {
    if (modelSaving) return
    const seq = ++modelFetchSeqRef.current
    setModelSaving(true)
    setModelError(null)
    let switched = false
    try {
      const sameModel =
        modelConfig?.current === id &&
        (!modelConfig?.currentProvider || modelConfig.currentProvider === provider)
      if (!sameModel) {
        await switchMobileModel(token, id, provider, currentMode)
        switched = true
      }
      if (saveEffort) {
        await setMobileReasoningEffort(token, provider, effort)
      }
      const cfg = await fetchModelConfig(token, currentMode)
      if (seq !== modelFetchSeqRef.current) return
      setModelConfig(cfg)
      setSettingsView('main')
      setPendingModel(null)
      setEffortTouched(false)
      setModelSearch('')
      modelDirtyRef.current = false
    } catch (e) {
      if (seq !== modelFetchSeqRef.current) return
      // 模型切换已成功、仅 effort 写入失败：仍刷新配置让卡片反映真实模型，
      // 错误留在 sheet 内供重试（不把「切了但强度没保存」伪装成完全失败）。
      if (switched) {
        try {
          const cfg = await fetchModelConfig(token, currentMode)
          if (seq === modelFetchSeqRef.current) setModelConfig(cfg)
        } catch {
          /* 保留原配置，错误提示已足够 */
        }
      }
      setModelError(e instanceof Error ? e.message : '切换失败，请重试')
    } finally {
      setModelSaving(false)
    }
  }
  // 主导航页由 props.page 驱动（底部 Tab）；设置页内部保留子视图切换：
  // main（模式卡 + 模型卡 + 网络与关于）/ model（模型选择子页）
  const settingsOpen = page === 'settings'
  const [settingsView, setSettingsView] = useState<SettingsView>('main')
  /** 会话分组：组头折叠态（默认全展开，运行时状态不持久化） */
  const [sessCollapsed, setSessCollapsed] = useState<Record<string, boolean>>({})
  /** 会话分组：组内「展开其余 N 个」态 */
  const [sessExpanded, setSessExpanded] = useState<Record<string, boolean>>({})

  // 页面切换没有横向拖拽手势：底部 Tab 是显式导航，返回由「Tab 点击 / 系统返回键」承担，
  // 不再有抽屉式跟手滑出（旧实现右滑 60% 关闭，方向与 iOS 返回手势相反且只支持关闭）。

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

  // 模型搜索：非空时平铺展示匹配结果（分组在搜索态下不再有意义）
  const [modelSearch, setModelSearch] = useState('')

  // 推理强度暂存：pendingEffort=null 表示 provider 默认；effortTouched 区分
  // 「用户主动改过」与「仅切换 provider」——后者不覆盖目标 provider 的已有配置。
  const [pendingEffort, setPendingEffort] = useState<string | null>(null)
  const [effortTouched, setEffortTouched] = useState(false)
  const [modelSaving, setModelSaving] = useState(false)
  const [modelError, setModelError] = useState<string | null>(null)
  /** 用户在模型视图内是否已操作过（拉取返回后不覆盖其选择） */
  const modelDirtyRef = useRef(false)
  /** 模型配置请求序号：慢链路下先发后至的响应不得覆盖新选择/新配置 */
  const modelFetchSeqRef = useRef(0)

  /** 回聊天页：设置页统一重置回主视图（避免下次进入停留在子页） */
  const closeSettings = () => {
    setSettingsView('main')
    setModelSearch('')
    setModelError(null)
    setEffortTouched(false)
    modelDirtyRef.current = false
    onPageChange('chat')
  }

  // 离开设置页即重置子视图：避免下次回来停在模型选择页（暂存选择一并丢弃）
  useEffect(() => {
    if (page !== 'settings') setSettingsView('main')
  }, [page])

  /** 返回主视图（header ← / 底部「返回」共用）：放弃子视图未确认的暂存选择 */
  const backToMain = () => {
    setPendingModel(null)
    setModelSearch('')
    setModelError(null)
    setEffortTouched(false)
    modelDirtyRef.current = false
    setSettingsView('main')
  }

  /** 子视图底部「确定」：按视图提交暂存选择（模型走后端权威入口） */
  const confirmView = () => {
    if (settingsView === 'model' && pendingModel) {
      void switchToModel(pendingModel.id, pendingModel.provider, pendingEffort, effortTouched)
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

  // 页面是常驻导航目标，没有「点击外部关闭」：切页只由底部 Tab 或页内动作触发。

  const pickFontSize = (size: MobileFontSize) => {
    setFs(setFontSize(size))
    setFsOpen(false)
  }

  /** 品牌区：纯展示——导航入口收敛到底部 Tab，品牌不再承担菜单职责 */
  const renderBrand = () => (
    <div className="mobile-nav-brand-wrap">
      <div className="mobile-nav-brand is-static">
        <BlinkLogo />
        <span className="mobile-nav-title">Nuphus</span>
      </div>
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
  // 当前生效模型元数据（用于只在支持 reasoning effort 时展示强度）
  const currentModelInfo = findCurrentModel(modelConfig)
  // step（工具调用步数）= 当前执行累计工具调用（tool_call_start 事件累积）；
  // time（执行用时）= 优先后端权威起点 metaStartedAtMs（刷新/重连不归零），
  // 旧后端缺省时退化为前端 startedAt；暂停冻结在 pausedAt。
  const execSteps = activity.tools.length
  const elapsedBase = activity.pausedAt ?? now
  const execOrigin = activity.metaStartedAtMs ?? activity.startedAt
  const execDuration = execOrigin ? elapsedBase - execOrigin : 0

  // ── 会话分组（与桌面同一纯函数 + 同一返回体）──
  // 组序维度 / 组内键来自返回体 sort_prefs（桌面 ⋯ 菜单设置，落 preferences）；
  // 「未分组」固定末位；归档文件夹整组隐藏（其会话不落未分组）。
  // ⚠️ 移动端**只读跟随**，不提供排序设置入口（排序是低频偏好，收敛在桌面一处）。
  const sessGroups = useMemo(
    () =>
      buildSessionGroups(
        sessions?.items ?? [],
        sessions?.projects ?? [],
        sessions?.archived_projects ?? [],
        normalizeSessionSortPrefs(sessions?.sort_prefs),
      ),
    [sessions],
  )
  const sessLimit = normalizeGroupLimit(sessions?.collapsed_limit ?? DEFAULT_GROUP_LIMIT)

  // ── 模型子视图：搜索 / 元数据 / 推理强度（与桌面 list_models 同一投影）──
  const modelSearchTerm = modelSearch.trim().toLowerCase()
  const modelSearchResults = modelSearchTerm
    ? (modelConfig?.models ?? []).filter(m =>
        [m.id, m.provider, ...(m.alias ?? [])].some(v => v.toLowerCase().includes(modelSearchTerm)),
      )
    : []
  const pendingModelInfo = pendingModel
    ? modelConfig?.models.find(
        m => m.id === pendingModel.id && m.provider === pendingModel.provider,
      )
    : undefined
  // 已持久化值可能不在模型声明列表里（配置被手改/模型元数据变更）——并入选项，
  // 保证当前值可见、可切回默认，而不是出现「没有任何选中项」。
  const pendingEfforts = pendingModelInfo
    ? Array.from(
        new Set([
          ...(pendingModelInfo.reasoning_efforts ?? []),
          ...(pendingEffort ? [pendingEffort] : []),
        ]),
      )
    : []

  const modelMeta = (m: ModelInfo) => {
    const meta: React.ReactNode[] = []
    if (m.context_window && m.context_window > 0) {
      meta.push(
        <span key="ctx" className="mobile-model-ctx">
          {formatTokens(m.context_window)} ctx
        </span>,
      )
    }
    if (m.supports_vision) {
      meta.push(
        <span key="vision" className="mobile-model-tag">
          vision
        </span>,
      )
    }
    if (m.supports_audio) {
      meta.push(
        <span key="audio" className="mobile-model-tag">
          audio
        </span>,
      )
    }
    if (m.supports_image_generation) {
      meta.push(
        <span key="image" className="mobile-model-tag">
          image
        </span>,
      )
    }
    if ((m.reasoning_efforts?.length ?? 0) > 0) {
      meta.push(
        <span key="effort" className="mobile-model-tag">
          effort
        </span>,
      )
    }
    return meta
  }

  const renderModelRow = (m: ModelInfo) => {
    const selected = pendingModel?.id === m.id && pendingModel?.provider === m.provider
    const meta = modelMeta(m)
    return (
      <button
        key={`${m.provider}:${m.id}`}
        type="button"
        className={`mobile-settings-sub-item mobile-model-item${selected ? ' is-active' : ''}`}
        onClick={() => choosePendingModel(m)}
      >
        <span className="mobile-model-item-main">
          <span className="mobile-model-item-name">{m.alias?.[0] || m.id}</span>
          {m.alias?.[0] && m.alias[0] !== m.id && (
            <span className="mobile-model-item-id">{m.id}</span>
          )}
          {meta.length > 0 && <span className="mobile-model-item-meta">{meta}</span>}
        </span>
        {selected && <Check size={15} aria-hidden="true" />}
      </button>
    )
  }

  /** 主导航页内容：会话页 / 设置页（聊天页由 ChatScreen 渲染，不在此处） */
  const renderPage = () => {
    if (page === 'chat') return null
    // 设置页子视图（模型选择）只在设置页生效；切走再回来会重置（见下方 effect）
    const subView = page === 'settings' && settingsView !== 'main'
    return (
      <div className="mobile-page" role="dialog" aria-label={page === 'sessions' ? '会话' : '设置'}>
        {/* 头部：主视图=页面标题（设置页带连接状态）| 子视图=返回 + 标题 */}
        <div className="mobile-mode-head">
          {!subView ? (
            <span className="mobile-settings-brand">
              {page === 'settings' && <BlinkLogo />}
              <span className="mobile-settings-brand-name">
                {page === 'sessions' ? '会话' : '设置'}
              </span>
              {page === 'settings' && (
                <span className={`mobile-settings-badge is-${wsStatus}`} role="status">
                  <span className="mobile-settings-badge-dot" aria-hidden="true" />
                  {STATUS_LABEL[wsStatus]()}
                </span>
              )}
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
            {/* 新会话入口只属于会话页（高频动作放在列表页顶部） */}
            {page === 'sessions' && (
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
            )}
            {/* 子视图保留 X：一步回聊天页（主视图靠底部 Tab 切走） */}
            {subView && (
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
          {!subView ? (
            <>
              {page === 'settings' && (
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
                        {/* 执行中锁定模式切换（与桌面 mode chip 一致，同一后端执行态投影）：
                        主指令受理时已按发送 mode 绑定当前会话，执行中改写会让后端
                        set_mode 拒绝（set_mode_impl 执行态判定），这里同步禁用并说明，
                        避免「点了没反应」。 */}
                        <button
                          type="button"
                          role="radio"
                          aria-checked={currentMode === 'leader'}
                          disabled={sessLocked}
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
                          disabled={sessLocked}
                          className={[
                            'mobile-mode-info-opt',
                            currentMode === 'workflow' ? 'is-active' : '',
                          ]
                            .filter(Boolean)
                            .join(' ')}
                          onClick={() => selectMode('workflow')}
                        >
                          <span className="mobile-mode-info-opt-name">Workflow</span>
                          <span className="mobile-mode-info-opt-desc">
                            解析模板生成可执行工作流
                          </span>
                          {currentMode === 'workflow' && <Check size={15} aria-hidden="true" />}
                        </button>
                        {activeCustomName ? (
                          <button
                            type="button"
                            role="radio"
                            aria-checked={currentMode === 'custom'}
                            disabled={sessLocked}
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
                        {sessLocked && (
                          <div className="mobile-mode-info-note">
                            执行中不可切换模式，结束后可切换
                          </div>
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
                      {ctxCap > 0 && (
                        <span className="mobile-model-info-cap">{formatTokens(ctxCap)}</span>
                      )}
                    </div>
                    <div className="mobile-model-info-ctx">
                      <span className="mobile-model-info-ctx-label">ctx</span>
                      <span className="mobile-model-info-ctx-gauge" aria-hidden>
                        {Array.from({ length: 5 }).map((_, i) => (
                          <span
                            key={i}
                            className="mobile-model-info-ctx-cell"
                            style={
                              i < Math.round(ctxPct * 5) ? { background: ctxColor } : undefined
                            }
                          />
                        ))}
                      </span>
                      <span className="mobile-model-info-ctx-usage">
                        {ctxCap > 0
                          ? `${formatTokens(ctxUsed)}/${formatTokens(ctxCap)}`
                          : `${formatTokens(ctxUsed)}/--`}
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
                        {currentModelInfo && currentModelInfo.reasoning_efforts.length > 0 && (
                          <span>effort {modelConfig?.reasoningEffort || 'default'}</span>
                        )}
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

                  {/* 常规设置项：此前散落在 header 图标与网络弹窗，收进设置页给它们归属 */}
                  <div className="mobile-settings-group">连接</div>
                  <div className="mobile-settings-card">
                    <button
                      type="button"
                      className="mobile-settings-card-row"
                      onClick={() => setNetworkOpen(true)}
                    >
                      <Wifi size={16} className="mobile-settings-card-icon" aria-hidden="true" />
                      <span className="mobile-settings-card-title">网络与连接</span>
                      <span className="mobile-settings-card-value">
                        {connMode === 'lan'
                          ? t('mobile.connLan')
                          : connMode === 'wan'
                            ? t('mobile.connRelay')
                            : ''}
                      </span>
                      <ChevronRight size={15} aria-hidden="true" />
                    </button>
                  </div>

                  <div className="mobile-settings-group">外观</div>
                  <div className="mobile-settings-card">
                    <button
                      type="button"
                      className="mobile-settings-card-row"
                      onClick={() => setTheme(toggleTheme())}
                    >
                      <span className="mobile-settings-card-title">主题</span>
                      <span className="mobile-settings-card-value">
                        {theme === 'dark' ? '深色' : '浅色'}
                      </span>
                    </button>
                    <div className="mobile-settings-card-row">
                      <span className="mobile-settings-card-title">字号</span>
                      <span
                        className="mobile-model-effort-options"
                        role="radiogroup"
                        aria-label="字号"
                      >
                        {FONT_SIZE_OPTIONS.map(opt => (
                          <button
                            key={opt.value}
                            type="button"
                            role="radio"
                            aria-checked={fontSize === opt.value}
                            className={`mobile-model-effort-option${fontSize === opt.value ? ' is-active' : ''}`}
                            onClick={() => pickFontSize(opt.value)}
                          >
                            {opt.label}
                          </button>
                        ))}
                      </span>
                    </div>
                  </div>

                  <div className="mobile-settings-group">关于</div>
                  <div className="mobile-settings-card">
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
                        onDisconnect?.()
                      }}
                    >
                      重置客户端配对
                    </button>
                  </div>
                </>
              )}
              {page === 'sessions' && (
                <>
                  {/* 会话列表：主视图直接展示（不再子视图）——**按项目文件夹分组**渲染，
                与桌面 rail 同一返回体 + 同一分组纯函数；点选即切换电脑端视图。
                执行中/后端守卫锁定时禁用。 */}
                  <div className="mobile-sess-section">
                    <div className="mobile-settings-group">{t('sessionRail.projectsTitle')}</div>
                    {/* 执行中锁定：整表提示一次（旧实现每行重复「执行中锁定」，扫读噪声大） */}
                    {sessLocked && (
                      <div className="mobile-sess-locked" role="status">
                        执行中，暂不可切换会话
                      </div>
                    )}
                    {sessGroups.length > 0 ? (
                      sessGroups.map(group => {
                        const collapsed = !!sessCollapsed[group.key]
                        const slice = visibleGroupSessions(
                          group,
                          sessLimit,
                          !!sessExpanded[group.key],
                        )
                        return (
                          <div className="mobile-sess-group" key={group.key || '__ungrouped__'}>
                            {/* 组头：chevron + 文件夹名（flex）+ 会话数；点击收起/展开整组。
                                chevron 放左侧，与桌面会话工作台组头同序（先箭头后名称） */}
                            <button
                              type="button"
                              className="mobile-sess-group-head"
                              onClick={() =>
                                setSessCollapsed(m => ({ ...m, [group.key]: !m[group.key] }))
                              }
                              aria-expanded={!collapsed}
                            >
                              <span className="mobile-sess-group-chevron" aria-hidden="true">
                                {collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                              </span>
                              <span
                                className={`mobile-sess-group-name${
                                  group.path === null ? ' is-ungrouped' : ''
                                }`}
                              >
                                {group.name || t('sessionRail.ungrouped')}
                              </span>
                              <span className="mobile-sess-group-count">
                                {group.sessions.length}
                              </span>
                            </button>
                            {!collapsed &&
                              (group.sessions.length > 0 ? (
                                <>
                                  <div className="mobile-sess-list">
                                    {slice.sessions.map(item => (
                                      <button
                                        key={item.id}
                                        type="button"
                                        className={[
                                          'mobile-sess-item',
                                          item.is_active ? 'is-active' : '',
                                        ]
                                          .filter(Boolean)
                                          .join(' ')}
                                        disabled={sessLocked}
                                        onClick={() => {
                                          onSwitchSession?.(item.id, item.mode)
                                          // 切会话的意图是「去看这段对话」：切完直接回聊天页
                                          onPageChange('chat')
                                        }}
                                      >
                                        {item.mode && (
                                          <span
                                            className={`mobile-sess-mode mode-${item.mode}`}
                                            aria-hidden="true"
                                          >
                                            {modeToLetter(item.mode)}
                                          </span>
                                        )}
                                        <span className="mobile-sess-title">
                                          {item.title || t('sessionRail.untitled')}
                                        </span>
                                        {item.is_active && (
                                          <span className="mobile-sess-current">
                                            {t('sessionRail.current')}
                                          </span>
                                        )}
                                        <span className="mobile-sess-time">
                                          {relativeTime(item.updated_at)}
                                        </span>
                                      </button>
                                    ))}
                                  </div>
                                  {/* 折叠行：与桌面同一上限语义（全局 collapsed_limit） */}
                                  {slice.hiddenCount > 0 && (
                                    <button
                                      type="button"
                                      className="mobile-sess-more"
                                      onClick={() =>
                                        setSessExpanded(m => ({ ...m, [group.key]: true }))
                                      }
                                    >
                                      {t('sessionRail.expandMore', String(slice.hiddenCount))}
                                    </button>
                                  )}
                                </>
                              ) : (
                                // 空文件夹（书签存在但无会话）：弱提示，组仍显示
                                <div className="mobile-sess-empty">
                                  {t('sessionRail.groupEmpty')}
                                </div>
                              ))}
                          </div>
                        )
                      })
                    ) : (
                      <div className="mobile-sess-empty">{t('sessionRail.emptySessions')}</div>
                    )}
                    <div className="mobile-settings-reset-hint">
                      切换的是电脑端正在显示的对话，两端同步
                    </div>
                  </div>
                </>
              )}
            </>
          ) : (
            <>
              <div className="mobile-settings-sub-note">
                模型与桌面端同源配置；选择后点「确定」生效
              </div>
              {modelError && (
                <div className="mobile-model-error" role="alert">
                  {modelError}
                </div>
              )}
              {modelLoading && <div className="mobile-settings-sub-note">加载中…</div>}
              {!modelLoading && (!modelConfig?.models || modelConfig.models.length === 0) && (
                <div className="mobile-settings-sub-note">暂无模型，请先在桌面端配置</div>
              )}
              {!modelLoading && modelConfig && modelConfig.models.length > 0 && (
                <>
                  <input
                    className="mobile-model-search"
                    type="search"
                    value={modelSearch}
                    onChange={e => setModelSearch(e.target.value)}
                    placeholder="搜索模型 / 提供商"
                    aria-label="搜索模型"
                    autoComplete="off"
                    spellCheck={false}
                  />
                  {modelSearchTerm ? (
                    modelSearchResults.length > 0 ? (
                      <div className="mobile-model-group-body" role="group" aria-label="搜索结果">
                        {modelSearchResults.map(m => renderModelRow(m))}
                      </div>
                    ) : (
                      <div className="mobile-model-search-empty">没有匹配的模型</div>
                    )
                  ) : (
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
                              <span className="mobile-model-group-name">
                                {provider || '未分类'}
                              </span>
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
                                {models.map(m => renderModelRow(m))}
                              </div>
                            )}
                          </div>
                        )
                      },
                    )
                  )}
                  {pendingModelInfo && pendingModelInfo.reasoning_efforts.length > 0 && (
                    <div className="mobile-model-effort">
                      <div className="mobile-model-effort-head">
                        <span className="mobile-model-effort-title">推理强度</span>
                        <span className="mobile-model-effort-sub">
                          {pendingModelInfo.alias?.[0] || pendingModelInfo.id}
                        </span>
                      </div>
                      <div
                        className="mobile-model-effort-options"
                        role="radiogroup"
                        aria-label="推理强度"
                      >
                        <button
                          type="button"
                          role="radio"
                          aria-checked={pendingEffort == null}
                          className={`mobile-model-effort-option${pendingEffort == null ? ' is-active' : ''}`}
                          onClick={() => {
                            setPendingEffort(null)
                            setEffortTouched(true)
                            modelDirtyRef.current = true
                          }}
                        >
                          默认
                        </button>
                        {pendingEfforts.map(e => (
                          <button
                            key={e}
                            type="button"
                            role="radio"
                            aria-checked={pendingEffort === e}
                            className={`mobile-model-effort-option${pendingEffort === e ? ' is-active' : ''}`}
                            onClick={() => {
                              setPendingEffort(e)
                              setEffortTouched(true)
                              modelDirtyRef.current = true
                            }}
                          >
                            {e}
                          </button>
                        ))}
                      </div>
                      <div className="mobile-model-effort-note">
                        按服务商保存，影响之后发送的消息
                      </div>
                    </div>
                  )}
                  {pendingModelInfo && pendingModelInfo.reasoning_efforts.length === 0 && (
                    <div className="mobile-settings-sub-note">
                      该模型未声明推理强度，使用服务商默认。
                    </div>
                  )}
                </>
              )}
            </>
          )}
        </div>
        {/* 子视图底部操作栏：只保留「确定」（提交暂存选择）。
            返回已由 header 左上箭头 + 系统返回键承担，底部不再重复一个返回入口。 */}
        {subView && (
          <div className="mobile-settings-footer">
            <button
              type="button"
              className="mobile-settings-footer-btn is-confirm"
              disabled={(settingsView === 'model' && !pendingModel) || modelSaving}
              onClick={confirmView}
            >
              {modelSaving ? '保存中…' : '确定'}
            </button>
          </div>
        )}
      </div>
    )
  }

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
          {/* 执行中（含收尾期）不可「先切 mode 再新建」：切 mode 会被后端执行态判定拒绝。
              判定源与模式列表 / 会话列表同一（sessions.stage，未加载时回退同源 WS 投影）。 */}
          <button
            type="button"
            className="mobile-newchat-btn is-start"
            disabled={sessLocked}
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
            <ServerAddressControl defaultExpanded onNavigated={() => setNetworkOpen(false)} />
            {/* 中继通道手动「切换到本地网络」：整页跳转 http 局域网（top-level 不受 mixed
                content 限制）。仅 wan（经中继打开）展示；局域网 http 页已直连无需切。 */}
            {connMode === 'wan' && onSwitchToLanManual && (
              <button
                type="button"
                className="mobile-settings-card-row is-secondary"
                disabled={lanSwitching}
                onClick={() => {
                  setNetworkOpen(false)
                  onSwitchToLanManual?.()
                }}
              >
                <RefreshCw size={14} className="mobile-settings-card-icon" aria-hidden="true" />
                <span className="mobile-settings-card-title is-secondary">
                  {t('mobile.networkSwitchLan')}
                </span>
                <span className="mobile-settings-card-value is-action">
                  {lanSwitching ? t('mobile.networkSwitchLanBusy') : t('mobile.networkSwitchLanGo')}
                </span>
              </button>
            )}
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
                  <span className="mobile-settings-conn-label is-relay">
                    {t('mobile.connRelay')}
                  </span>
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
          <MorphIcon icon={ALargeSmallIcon} size={16} spring="snappy" aria-hidden="true" />
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
      {/* 中继通道手动「切换到本地网络」快捷入口（wifi 图标，仅 wan；点击弹确认框展示直连地址）。
          样式与字体/主题按钮一致（32px 圆形图标钮，44pt 热区）；lan 已直连无需切换故隐藏。 */}
      {connMode === 'wan' && onLanSwitchRequest && (
        <button
          type="button"
          className="mobile-nav-theme"
          aria-label={t('mobile.networkSwitchLan')}
          disabled={lanSwitching}
          onClick={onLanSwitchRequest}
        >
          <Wifi size={16} aria-hidden="true" />
        </button>
      )}
      <button
        type="button"
        className="mobile-nav-theme"
        aria-label={theme === 'dark' ? t('mobile.switchToLight') : t('mobile.switchToDark')}
        onClick={() => setTheme(toggleTheme())}
      >
        <MorphIcon
          icon={theme === 'dark' ? SunIcon : MoonIcon}
          size={16}
          spring="snappy"
          aria-hidden="true"
        />
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
        {page !== 'chat' && createPortal(renderPage(), document.body)}
      </>
    )
  }

  // 空闲：连接状态（居中锚点）。pill 即网络与连接入口——显示当前渠道（局域网/服务端）
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
      {page !== 'chat' && createPortal(renderPage(), document.body)}
      {newChatOpen && renderNewChatModal()}
      {networkOpen && renderNetworkModal()}
    </>
  )
}
