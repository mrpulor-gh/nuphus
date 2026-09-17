import { useCallback, useEffect, useRef, useState } from 'react'
import { IconPlus, IconX, IconFile, IconEye, IconTrash2 } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import {
  listAgentStatuses,
  listAgentDeliverables,
  deleteAgentDeliverable,
  listExternalAgents,
  notifyExtAgentRemoved,
  type ExternalAgentStatus,
  type ExternalAgentConfig,
  type AgentDeliverable,
} from '../lib/api'
import { AgentIconAuto } from '../components/AgentIconAuto'
import { PreviewOverlay } from './PreviewOverlay'
import '../../styles/external-agents.css'

const POLL_INTERVAL_MS = 3000

/** 旧版本遗留的持久化 key（历史「隐藏为数字」/ pin 语义）：新语义下
 *  「从列表栏移除」与 pin 均为内存态，启动时清理，避免历史隐藏记录被误当成
 *  「本轮已移出」而让 agent 复活成幽灵条目 */
const LEGACY_HIDDEN_KEY = 'nuphus.extAgents.hiddenAgents'
const LEGACY_PINNED_KEY = 'nuphus.extAgents.pinned'
/** 配置中心保存成功后广播的事件名 */
export const EXT_AGENT_PINNED_EVENT = 'nuphus:ext-agent-pinned'

/** 生命周期内 pin 集合：不从 localStorage 恢复，每次启动为空 */
function loadPinnedAgents(): string[] {
  return []
}

/** 门铃上报时刻（RFC3339）→ epoch ms；缺失/非法返回 0（视为「无活动」） */
function updatedAtMs(a?: ExternalAgentStatus): number {
  const t = a?.updated_at ? Date.parse(a.updated_at) : NaN
  return Number.isNaN(t) ? 0 : t
}

/** 后端 state 原值 → 展示样式 class（未知状态统一 is-unknown，不拦截新状态） */
const STATE_CLASS: Record<string, string> = {
  idle: 'is-idle',
  dispatched: 'is-dispatched',
  in_progress: 'is-in-progress',
  ready: 'is-ready',
  done: 'is-done',
  blocked: 'is-blocked',
  error: 'is-error',
}

/** 后端 state 原值 → i18n key（前端只做显示层转换，不改写状态值） */
const STATE_I18N: Record<string, string> = {
  idle: 'extAgents.state.idle',
  dispatched: 'extAgents.state.dispatched',
  in_progress: 'extAgents.state.inProgress',
  ready: 'extAgents.state.ready',
  done: 'extAgents.state.done',
  blocked: 'extAgents.state.blocked',
  error: 'extAgents.state.error',
  uninitialized: 'extAgents.state.uninitialized',
}

/** agent 名 → 图标类型（CLI 类终端图标 / 其余 bot 图标） */
function agentKind(name: string): 'cli' | 'agent' {
  const n = name.toLowerCase()
  if (n.includes('claude') || n.includes('code') || n.includes('cli')) return 'cli'
  return 'agent'
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function formatTime(rfc3339: string): string {
  const d = new Date(rfc3339)
  if (Number.isNaN(d.getTime())) return ''
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

interface ExternalAgentsStatusBarProps {
  /** 面板关闭时置 false（挂载/卸载由父组件控制，此处默认始终挂载） */
  visible?: boolean
  /** "+" 按钮：打开外部 Agent 配置中心 */
  onOpenConfig?: () => void
  /** 移出时的用户面反馈（父组件注入 HUD 轻提示：`showToast` → `hud_update`）。
   *  ⛔ 绝不可用会话消息（addMessage）承载：执行期 messages 末尾是「正在流式的 agent
   *  气泡」，插入任何消息都会把回答隔断（详见 App.tsx chat-area 处的说明） */
  onNotice?: (text: string) => void
}

/**
 * 外部 Agent 运行时态面板：输入框外层右上角的悬浮胶囊。
 * - 数据源：listAgentStatuses()，轻量轮询（≈3s），仅组件挂载且页面可见时拉取；
 *   切后台自动暂停，回前台立即刷新（门铃事件由后端落 status.json，轮询兜底覆盖）。
 * - 每个处于「被调用」状态的 agent 渲染为圆形头像按钮：点击弹出该 agent 的交付物列表弹窗，
 *   条目点击走 PreviewOverlay 内联预览；hover 出 tooltip 看详情。
 * - 「从列表栏移除」= 把该 agent 的头像从 DOM 真移除（本轮会话内存态，不落盘、
 *   不动后端配置与 team.toml），并以 HUD 轻提示反馈一句（`onNotice` → showToast，
 *   **不进会话消息数组**）；该 agent 再次被调用（门铃有新上报）或用户在配置中心
 *   重新保存时，自动回到列表栏。
 * - 末尾固定一个 "+" 配置入口，点击打开外部 Agent 配置中心（空列表时入口仍可见）。
 * - 状态值来自 status.json 原样映射，前端只加显示层。
 *
 * ── 可见性引擎（被调用即常驻，空闲按需浮现）──
 * - 后端启动清零 status.json：跨生命周期的陈旧 agent 不复存在；
 *   本轮内只有真实启动并经门铃上报验证的 agent 才有非 idle 状态。
 * - 列表栏内容 =（本轮被调用过的 agent：非 idle）∪（用户在本轮配置中心保存过的 agent：pin）
 *   −（用户已从列表栏移出的 agent）。有内容 → 整条胶囊常驻，不随 hover 隐藏；
 *   无内容 → 默认隐藏，鼠标悬停感应区临时浮现 "+" 配置入口。
 * - 配置中心是唯一配置源：本面板的移除只影响显示，删除配置一律由配置中心发起
 *   （写 team.toml），二者互不越权。
 */
export default function ExternalAgentsStatusBar({
  visible = true,
  onOpenConfig,
  onNotice,
}: ExternalAgentsStatusBarProps) {
  const { t } = useLanguage()
  const [agents, setAgents] = useState<ExternalAgentStatus[]>([])
  /** 用户已从列表栏移出的 agent（agent → 移出时刻 ms）。仅本轮会话内存态：
   *  不落盘、不影响后端配置/team.toml；该 agent 有新门铃活动即自动回归 */
  const [removed, setRemoved] = useState<Record<string, number>>({})
  const [pins, setPins] = useState<string[]>(loadPinnedAgents)
  /** hover 展开：额外（非常驻）agent 可见 */
  const [revealed, setRevealed] = useState(false)
  /** 移开鼠标后 10s 进入渐隐窗口 */
  const [fading, setFading] = useState(false)
  const leaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const fadeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** forced 的 ref 镜像（定时器回调里读取最新值，避免闭包过期） */
  const forcedRef = useRef(false)
  const [openAgent, setOpenAgent] = useState<ExternalAgentStatus | null>(null)
  const [deliverables, setDeliverables] = useState<AgentDeliverable[] | null>(null)
  const [loadingDeliv, setLoadingDeliv] = useState(false)
  const [previewPath, setPreviewPath] = useState<string | null>(null)
  /** 处于「确认删除」态的交付物路径（同时最多一行，行内二次确认防误删） */
  const [confirmDelPath, setConfirmDelPath] = useState<string | null>(null)
  /** 配置中心映射（key → icon/open/process）：头像按配置渲染真实图标，与设置页同源 */
  const [cfgMap, setCfgMap] = useState<Record<string, ExternalAgentConfig>>({})
  /** 删除失败提示（弹窗内一行红字，点击关闭） */
  const [delError, setDelError] = useState<string | null>(null)
  const stoppedRef = useRef(false)
  /** 当前弹窗对应的 agent 名（ref：异步响应回来时校验弹窗未被切换/关闭） */
  const openAgentRef = useRef<string | null>(null)

  useEffect(() => {
    stoppedRef.current = false
    let timer: ReturnType<typeof setTimeout> | null = null

    const poll = async () => {
      try {
        const list = await listAgentStatuses()
        if (!stoppedRef.current) {
          setAgents(list || [])
          // 移出后又有新的门铃上报（= 被再次调用）→ 该 agent 自动回到列表栏
          setRemoved(prev => {
            const keys = Object.keys(prev)
            if (keys.length === 0) return prev
            const next: Record<string, number> = {}
            let changed = false
            for (const k of keys) {
              const ts = updatedAtMs((list || []).find(a => a.agent === k))
              if (ts > prev[k]) {
                changed = true // 新活动 → 撤销移出
              } else {
                next[k] = prev[k]
              }
            }
            return changed ? next : prev
          })
        }
      } catch {
        /* 后端不可达：保留当前数据，下轮重试 */
      }
      if (!stoppedRef.current && document.visibilityState === 'visible') {
        timer = setTimeout(poll, POLL_INTERVAL_MS)
      }
    }

    const onVisibility = () => {
      if (document.visibilityState === 'visible') {
        if (!timer) void poll()
      } else if (timer) {
        clearTimeout(timer)
        timer = null
      }
    }

    document.addEventListener('visibilitychange', onVisibility)
    if (visible && document.visibilityState === 'visible') void poll()
    return () => {
      stoppedRef.current = true
      if (timer) clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [visible])

  // ── pin 集合：配置中心保存成功后广播事件 → 加入常驻集合（应用生命周期）──
  useEffect(() => {
    // 头像配置映射：初始加载 + 配置保存事件时刷新（新增/改图标后状态栏立即跟进）
    const refreshCfg = () => {
      listExternalAgents()
        .then(list => {
          const map: Record<string, ExternalAgentConfig> = {}
          for (const c of list || []) map[c.key] = c
          setCfgMap(map)
        })
        .catch(() => {
          /* 后端不可达：保留当前映射，下个事件重试 */
        })
    }
    refreshCfg()
    // 清理历史版本遗留的持久化状态（现语义均为内存态：启动即空）
    localStorage.removeItem(LEGACY_PINNED_KEY)
    localStorage.removeItem(LEGACY_HIDDEN_KEY)
    const onPinned = (e: Event) => {
      const key = (e as CustomEvent<string>).detail
      if (!key) return
      // 配置中心显式保存 = 用户主动纳入 → 撤销该 agent 的「已移出」标记
      setRemoved(prev => {
        if (!(key in prev)) return prev
        const next = { ...prev }
        delete next[key]
        return next
      })
      setPins(prev => (prev.includes(key) ? prev : [...prev, key]))
      refreshCfg()
    }
    window.addEventListener(EXT_AGENT_PINNED_EVENT, onPinned)
    return () => window.removeEventListener(EXT_AGENT_PINNED_EVENT, onPinned)
  }, [])

  // 卸载时清理渐隐定时器
  useEffect(
    () => () => {
      if (leaveTimer.current) clearTimeout(leaveTimer.current)
      if (fadeTimer.current) clearTimeout(fadeTimer.current)
    },
    [],
  )

  const handleZoneEnter = useCallback(() => {
    if (leaveTimer.current) {
      clearTimeout(leaveTimer.current)
      leaveTimer.current = null
    }
    if (fadeTimer.current) {
      clearTimeout(fadeTimer.current)
      fadeTimer.current = null
    }
    setRevealed(true)
    setFading(false)
  }, [])

  const handleZoneLeave = useCallback(() => {
    // 有常驻条件（执行中/阻塞/错误/pin）时整条胶囊强制可见，无需渐隐
    if (forcedRef.current) return
    // 移开 10s 后开始 0.6s 渐隐收起
    if (leaveTimer.current) clearTimeout(leaveTimer.current)
    leaveTimer.current = setTimeout(() => {
      leaveTimer.current = null
      setFading(true)
      fadeTimer.current = setTimeout(() => {
        fadeTimer.current = null
        setRevealed(false)
        setFading(false)
      }, 600)
    }, 10_000)
  }, [])

  /** 点击头像：打开交付物弹窗并拉取列表 */
  const openDeliverables = useCallback((a: ExternalAgentStatus) => {
    openAgentRef.current = a.agent
    setOpenAgent(a)
    setDeliverables(null)
    setConfirmDelPath(null)
    setDelError(null)
    setLoadingDeliv(true)
    listAgentDeliverables(a.agent)
      .then(list => {
        // 弹窗可能已被切换/关闭：只接受仍匹配的响应
        if (openAgentRef.current === a.agent) setDeliverables(list || [])
      })
      .catch(() => {
        if (openAgentRef.current === a.agent) setDeliverables([])
      })
      .finally(() => {
        if (openAgentRef.current === a.agent) setLoadingDeliv(false)
      })
  }, [])

  /** ── 生成物删除：行内两段式确认（trash → 行内「确认/取消」）。
   *  不用 window.confirm：Tauri WebView 中被屏蔽不弹窗，必须应用内 UI 防误触 ── */
  const askDeleteDeliverable = useCallback((d: AgentDeliverable) => {
    setDelError(null)
    setConfirmDelPath(d.path)
  }, [])

  const cancelDeleteDeliverable = useCallback(() => {
    setConfirmDelPath(null)
  }, [])

  const doDeleteDeliverable = useCallback((d: AgentDeliverable) => {
    const agent = openAgentRef.current
    if (!agent) return
    deleteAgentDeliverable(agent, d.rel_path)
      .then(() => {
        if (openAgentRef.current === agent) {
          // 列表即视图：本地移除条目即可，无需整表重拉
          setDeliverables(prev => (prev ? prev.filter(x => x.path !== d.path) : prev))
          // 正在预览的就是被删文件 → 关闭预览，避免继续展示已不存在的内容
          setPreviewPath(p => (p === d.path ? null : p))
        }
        setConfirmDelPath(null)
        setDelError(null)
      })
      .catch(e => {
        setConfirmDelPath(null)
        setDelError(String(e))
      })
  }, [])

  /** 从列表栏移除该 agent：本轮会话内不再渲染其头像（内存态，不落盘），
   *  不动后端配置与 team.toml。该 agent 再次被调用（门铃新上报）或在配置中心
   *  重新保存时自动回归。
   *
   *  用户面反馈走 `onNotice`（HUD 轻提示）——⛔ 绝不可用 addMessage 写会话消息：
   *  执行期 messages 末尾是正在流式的 agent 气泡，插入消息会隔断输出。 */
  const removeAgent = useCallback(
    (name: string) => {
      setRemoved(prev => ({ ...prev, [name]: Date.now() }))
      if (openAgentRef.current === name) {
        openAgentRef.current = null
        setOpenAgent(null)
      }
      setConfirmDelPath(null)
      setDelError(null)
      onNotice?.(t('extAgents.removedNotice', name))
      // 通知 agent：该外部 Agent 已被移出，后续需用户显式指定才可调用
      // （后端写一句提示，下一轮带进上下文；失败不阻断 UI）
      notifyExtAgentRemoved(name).catch(() => {})
    },
    [onNotice, t],
  )

  const closePopover = useCallback(() => {
    openAgentRef.current = null
    setOpenAgent(null)
    setConfirmDelPath(null)
    setDelError(null)
  }, [])

  if (!visible) return null

  const known = agents.filter(
    a =>
      !(a.agent in removed) &&
      // idle 且未 pin = 本轮未经门铃验证的历史残留（启动清零只重置为 idle 骨架，
      // 目录仍在），不渲染——列表栏只出现被调用过的 agent 或用户 pin 的配置
      ((a.state && a.state !== 'idle') || pins.includes(a.agent)),
  )
  /** 恒显条件：列表栏有内容即常驻（被调用的外部 agent 就一直在，不随 hover 隐藏）；
   *  内容为空（全部已移出 / 本轮无调用）则默认隐藏，hover 感应区临时浮现 "+"。 */
  const forced = known.length > 0
  forcedRef.current = forced
  const reports = (deliverables || []).filter(d => d.kind === 'report')
  const artifacts = (deliverables || []).filter(d => d.kind === 'artifact')
  /** 胶囊整体可见：强制常驻 ∪ hover 展开 ∪ 渐隐中 ∪ 有弹窗 */
  const shown = forced || revealed || fading || openAgent !== null

  /** 单个 agent 头像 */
  const renderAvatar = (a: ExternalAgentStatus) => {
    const state = a.state || 'unknown'
    const stateLabel = t(STATE_I18N[state] || 'extAgents.state.unknown')
    const kind = agentKind(a.agent)
    const title = `${a.agent} · ${stateLabel}${a.task_id ? ` · ${a.task_id}` : ''}`
    return (
      <div
        key={a.agent}
        className={['agent-avatar-btn', STATE_CLASS[state] || 'is-unknown']
          .filter(Boolean)
          .join(' ')}
        title={title}
      >
        <button
          type="button"
          className="agent-avatar-main"
          onClick={() => openDeliverables(a)}
          aria-label={t('extAgents.deliver.title', a.agent)}
        >
          <span className={`ext-agent-dot ${STATE_CLASS[state] || 'is-unknown'}`} aria-hidden />
          <span className={`agent-avatar-icon is-${kind}`}>
            <AgentIconAuto
              icon={cfgMap[a.agent]?.icon || 'auto'}
              size={22}
              avatarSize={30}
              name={a.agent}
              open={cfgMap[a.agent]?.open || ''}
              process={cfgMap[a.agent]?.process || ''}
            />
          </span>
        </button>
        <div className="agent-tooltip">
          <span className="agent-tooltip-name">{a.agent}</span>
          <span className="agent-tooltip-state">{stateLabel}</span>
          {a.task_id ? <span className="agent-tooltip-task">{a.task_id}</span> : null}
        </div>
      </div>
    )
  }

  return (
    <>
      {/* 遮罩在胶囊外层：bar 有 backdrop-filter，fixed 子元素会被其改变定位基准 */}
      {openAgent !== null && (
        <div className="ext-agent-popover-backdrop" onClick={closePopover} aria-hidden />
      )}
      {/* 感应区：与胶囊同高同右缘，向左延伸 200px；胶囊隐身时由它唤醒。
          常驻 DOM（透明），z-index 低于胶囊避免遮挡其交互 */}
      <div
        className="ext-agents-hover-zone"
        onMouseEnter={handleZoneEnter}
        onMouseLeave={handleZoneLeave}
        aria-hidden
      />
      <div
        className={[
          'external-agents-bar',
          forced ? 'forced' : '',
          shown ? (fading ? 'fading' : 'revealed') : 'concealed',
        ]
          .filter(Boolean)
          .join(' ')}
        role="status"
        aria-label={t('extAgents.title')}
        onMouseEnter={handleZoneEnter}
        onMouseLeave={handleZoneLeave}
      >
        {known.map(renderAvatar)}
        <button
          type="button"
          className="add-agent-entry"
          onClick={() => onOpenConfig?.()}
          title={t('extAgents.cfg.add')}
          aria-label={t('extAgents.cfg.add')}
        >
          <IconPlus size={18} />
        </button>

        {/* ── 交付物弹窗：锚定在胶囊上方，点遮罩 / Esc 关闭 ── */}
        {openAgent && (
          <div className="ext-agent-popover" role="dialog" aria-label={openAgent.agent}>
            <div className="ext-agent-popover-head">
              <span className={`agent-avatar-icon is-${agentKind(openAgent.agent)}`}>
                <AgentIconAuto
                  icon={cfgMap[openAgent.agent]?.icon || 'auto'}
                  size={18}
                  name={openAgent.agent}
                  open={cfgMap[openAgent.agent]?.open || ''}
                  process={cfgMap[openAgent.agent]?.process || ''}
                />
              </span>
              <span className="ext-agent-popover-title">{openAgent.agent}</span>
              <span
                className={`ext-agent-dot ${STATE_CLASS[openAgent.state || 'unknown'] || 'is-unknown'}`}
                aria-hidden
              />
              <button
                type="button"
                className="ext-agent-popover-close"
                onClick={closePopover}
                title={t('common.close')}
                aria-label={t('common.close')}
              >
                <IconX size={13} />
              </button>
            </div>
            {openAgent.last_event?.summary ? (
              <div className="ext-agent-popover-summary">{openAgent.last_event.summary}</div>
            ) : null}
            <div className="ext-agent-popover-body">
              {delError && (
                <button
                  type="button"
                  className="ext-agent-del-error"
                  onClick={() => setDelError(null)}
                  title={t('common.close')}
                >
                  {t('extAgents.deliver.deleteFail')}: {delError}
                </button>
              )}
              {loadingDeliv ? (
                <div className="ext-agent-popover-empty">{t('extAgents.deliver.loading')}</div>
              ) : (deliverables || []).length === 0 ? (
                <div className="ext-agent-popover-empty">{t('extAgents.deliver.empty')}</div>
              ) : (
                [
                  reports.length > 0 ? (
                    <div key="reports" className="ext-agent-dgroup">
                      <div className="ext-agent-dgroup-label">{t('extAgents.deliver.reports')}</div>
                      {reports.map(d => (
                        <DeliverableRow
                          key={d.path}
                          d={d}
                          onOpen={setPreviewPath}
                          t={t}
                          confirming={confirmDelPath === d.path}
                          onAskDelete={askDeleteDeliverable}
                          onConfirmDelete={doDeleteDeliverable}
                          onCancelDelete={cancelDeleteDeliverable}
                        />
                      ))}
                    </div>
                  ) : null,
                  artifacts.length > 0 ? (
                    <div key="artifacts" className="ext-agent-dgroup">
                      <div className="ext-agent-dgroup-label">
                        {t('extAgents.deliver.artifacts')}
                      </div>
                      {artifacts.map(d => (
                        <DeliverableRow
                          key={d.path}
                          d={d}
                          onOpen={setPreviewPath}
                          t={t}
                          confirming={confirmDelPath === d.path}
                          onAskDelete={askDeleteDeliverable}
                          onConfirmDelete={doDeleteDeliverable}
                          onCancelDelete={cancelDeleteDeliverable}
                        />
                      ))}
                    </div>
                  ) : null,
                ]
              )}
            </div>
            <div className="ext-agent-popover-foot">
              <button
                type="button"
                className="ext-agent-hide-row"
                onClick={() => removeAgent(openAgent.agent)}
                title={t('extAgents.removeFromBar')}
              >
                {t('extAgents.removeFromBar')}
              </button>
            </div>
          </div>
        )}
      </div>
      {previewPath && <PreviewOverlay path={previewPath} onClose={() => setPreviewPath(null)} />}
    </>
  )
}

interface DeliverableRowProps {
  d: AgentDeliverable
  onOpen: (path: string) => void
  t: (key: string, ...args: string[]) => string
  /** 该行处于「确认删除」警示态 */
  confirming?: boolean
  onAskDelete?: (d: AgentDeliverable) => void
  onConfirmDelete?: (d: AgentDeliverable) => void
  onCancelDelete?: () => void
}

/**
 * 单条交付物：默认整行点击预览，hover 显现 eye/trash 操作区；
 * trash 进入行内二次确认（hint + 删除/取消），确认后由父级调后端删除并移除条目。
 * 外层用 div[role=button] 替代原 button：行内要嵌真正的按钮元素
 * （HTML 禁止 button 嵌套），onKeyDown 维持 Enter/Space 键盘可操作性。
 */
function DeliverableRow({
  d,
  onOpen,
  t,
  confirming = false,
  onAskDelete,
  onConfirmDelete,
  onCancelDelete,
}: DeliverableRowProps) {
  const hasActions = Boolean(onAskDelete && onConfirmDelete && onCancelDelete)
  return (
    <div
      role="button"
      tabIndex={0}
      className={`ext-agent-deliver-row${confirming ? ' confirming' : ''}`}
      onClick={() => !confirming && onOpen(d.path)}
      onKeyDown={e => {
        if (!confirming && (e.key === 'Enter' || e.key === ' ')) onOpen(d.path)
      }}
      title={`${d.rel_path}\n${t('extAgents.deliver.preview')}`}
    >
      <IconFile size={14} />
      <span className="ext-agent-deliver-info">
        <span className="ext-agent-deliver-name">{d.name}</span>
        <span className="ext-agent-deliver-meta">
          {formatTime(d.modified)} · {formatSize(d.size)}
        </span>
      </span>
      {confirming ? (
        <>
          <span className="ext-agent-del-confirm-hint">{t('extAgents.deliver.confirmHint')}</span>
          {hasActions && (
            <>
              <button
                type="button"
                className="ext-agent-del-btn danger"
                onClick={e => {
                  e.stopPropagation()
                  onConfirmDelete?.(d)
                }}
              >
                {t('extAgents.deliver.confirmBtn')}
              </button>
              <button
                type="button"
                className="ext-agent-del-btn"
                onClick={e => {
                  e.stopPropagation()
                  onCancelDelete?.()
                }}
              >
                {t('extAgents.deliver.cancelBtn')}
              </button>
            </>
          )}
        </>
      ) : (
        <>
          <IconEye size={13} className="ext-agent-deliver-eye" />
          {hasActions && (
            <button
              type="button"
              className="ext-agent-del-btn"
              onClick={e => {
                e.stopPropagation()
                onAskDelete?.(d)
              }}
              title={t('extAgents.deliver.delete')}
              aria-label={`${t('extAgents.deliver.delete')} · ${d.name}`}
            >
              <IconTrash2 size={13} />
            </button>
          )}
        </>
      )}
    </div>
  )
}
