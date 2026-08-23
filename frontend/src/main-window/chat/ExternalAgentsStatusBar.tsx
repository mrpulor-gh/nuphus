import { useCallback, useEffect, useRef, useState } from 'react'
import { IconBot, IconTerminal, IconPlus, IconX, IconFile, IconEye } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import {
  listAgentStatuses,
  listAgentDeliverables,
  type ExternalAgentStatus,
  type AgentDeliverable,
} from '../lib/api'
import { PreviewOverlay } from './PreviewOverlay'
import '../../styles/external-agents.css'

const POLL_INTERVAL_MS = 3000

/** 单独关闭的 agent 列表持久化 key（仅隐藏状态栏显示，不影响 handoff 目录与门铃） */
const HIDDEN_KEY = 'nuphus.extAgents.hiddenAgents'
/** 用户在配置中心添加过的 agent：应用生命周期内常驻显示。
 *  ⚠️ 仅内存态（随应用启动清零）——跨重启的持久化由后端启动清零 +
 *  门铃真实上报接管，避免历史 pin 让 idle agent 永远占位 */
const PINNED_KEY = 'nuphus.extAgents.pinned'
/** 配置中心保存成功后广播的事件名 */
export const EXT_AGENT_PINNED_EVENT = 'nuphus:ext-agent-pinned'

/** 常驻可见的注意态：执行中/阻塞/错误——其余状态默认隐藏，hover 感应区临时展开 */
const ATTENTION_STATES = new Set(['in_progress', 'blocked', 'error'])

/** 生命周期内 pin 集合：不从 localStorage 恢复，每次启动为空 */
function loadPinnedAgents(): string[] {
  return []
}

function loadHiddenAgents(): string[] {
  try {
    const raw = localStorage.getItem(HIDDEN_KEY)
    const parsed = raw ? JSON.parse(raw) : []
    return Array.isArray(parsed) ? parsed.filter(x => typeof x === 'string') : []
  } catch {
    return []
  }
}

function saveHiddenAgents(list: string[]) {
  try {
    localStorage.setItem(HIDDEN_KEY, JSON.stringify(list))
  } catch {
    /* 存储不可用：仅本次会话生效 */
  }
}

/** 后端 state 原值 → 展示样式 class（未知状态统一 is-unknown，不拦截新状态） */
const STATE_CLASS: Record<string, string> = {
  idle: 'is-idle',
  in_progress: 'is-in-progress',
  ready: 'is-ready',
  done: 'is-done',
  blocked: 'is-blocked',
  error: 'is-error',
}

/** 后端 state 原值 → i18n key（前端只做显示层转换，不改写状态值） */
const STATE_I18N: Record<string, string> = {
  idle: 'extAgents.state.idle',
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
}

/**
 * 外部 Agent 运行时态面板：输入框外层右上角的悬浮胶囊。
 * - 数据源：listAgentStatuses()，轻量轮询（≈3s），仅组件挂载且页面可见时拉取；
 *   切后台自动暂停，回前台立即刷新（门铃事件由后端落 status.json，轮询兜底覆盖）。
 * - 每个已初始化 agent 渲染为圆形头像按钮：点击弹出该 agent 的交付物列表弹窗，
 *   条目点击走 PreviewOverlay 内联预览；hover 出 tooltip 看详情。
 * - 移除入口收敛在弹窗底部「从状态栏移除」文字按钮（localStorage 持久化，
 *   不删除配置/目录；被移除的 agent 收进「已隐藏」入口可随时恢复）。
 * - 末尾固定一个 "+" 配置入口，点击打开外部 Agent 配置中心（空列表时入口仍可见）。
 * - 状态值来自 status.json 原样映射，前端只加显示层。
 *
 * ── 可见性引擎（默认隐藏，按需浮现）──
 * - 后端启动清零 status.json：跨生命周期的陈旧 agent 不复存在；
 *   本轮内只有真实启动并经门铃上报验证的 agent 才有非 idle 状态。
 * - 常驻显示 = 注意态（in_progress/blocked/error）∪ 用户在配置中心添加过的
 *   pin 集合（EXT_AGENT_PINNED_EVENT 广播 + localStorage，应用生命周期内有效）。
 * - 其余 agent：鼠标悬停在胶囊附近 → 全部展开；移开 10s 后 0.6s 渐隐收起
 *   （常驻项不受影响）。
 */
export default function ExternalAgentsStatusBar({
  visible = true,
  onOpenConfig,
}: ExternalAgentsStatusBarProps) {
  const { t } = useLanguage()
  const [agents, setAgents] = useState<ExternalAgentStatus[]>([])
  const [hidden, setHidden] = useState<string[]>(loadHiddenAgents)
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
  const [showHiddenPanel, setShowHiddenPanel] = useState(false)
  const [previewPath, setPreviewPath] = useState<string | null>(null)
  const stoppedRef = useRef(false)
  /** 当前弹窗对应的 agent 名（ref：异步响应回来时校验弹窗未被切换/关闭） */
  const openAgentRef = useRef<string | null>(null)

  useEffect(() => {
    stoppedRef.current = false
    let timer: ReturnType<typeof setTimeout> | null = null

    const poll = async () => {
      try {
        const list = await listAgentStatuses()
        if (!stoppedRef.current) setAgents(list || [])
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

  // ── pin 集合：配置中心保存成功后广播事件 → 加入常驻集合（应用生命周期） ──
  useEffect(() => {
    // 清理历史版本遗留的持久化 pin（现语义为内存态）
    localStorage.removeItem(PINNED_KEY)
    const onPinned = (e: Event) => {
      const key = (e as CustomEvent<string>).detail
      if (!key) return
      setPins(prev => (prev.includes(key) ? prev : [...prev, key]))
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
    setShowHiddenPanel(false)
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

  /** 单独关闭一个 agent 的状态栏显示（不删配置；可从「已隐藏」恢复） */
  const hideAgent = useCallback((name: string) => {
    setHidden(prev => {
      const next = prev.includes(name) ? prev : [...prev, name]
      saveHiddenAgents(next)
      return next
    })
    if (openAgentRef.current === name) {
      openAgentRef.current = null
      setOpenAgent(null)
    }
    setShowHiddenPanel(false)
  }, [])

  const restoreAgent = useCallback((name: string) => {
    setHidden(prev => {
      const next = prev.filter(n => n !== name)
      saveHiddenAgents(next)
      return next
    })
  }, [])

  const closePopover = useCallback(() => {
    openAgentRef.current = null
    setOpenAgent(null)
    setShowHiddenPanel(false)
  }, [])

  if (!visible) return null

  const known = agents.filter(
    a =>
      !hidden.includes(a.agent) &&
      // idle 且未 pin = 本轮未经门铃验证的历史残留（启动清零只重置为 idle 骨架，
      // 目录仍在），不渲染——状态栏只出现真实启动过的 agent 或用户 pin 的配置
      ((a.state && a.state !== 'idle') || pins.includes(a.agent)),
  )
  /** 常驻条件：注意态（执行中/阻塞/错误）或用户添加过的 pin —— 存在即整条胶囊强制可见 */
  const forced = known.some(
    a => ATTENTION_STATES.has(a.state || '') || pins.includes(a.agent),
  )
  forcedRef.current = forced
  const hiddenKnown = hidden.filter(name => agents.some(a => a.agent === name))
  const reports = (deliverables || []).filter(d => d.kind === 'report')
  const artifacts = (deliverables || []).filter(d => d.kind === 'artifact')
  /** 胶囊整体可见：强制常驻 ∪ hover 展开 ∪ 渐隐中 ∪ 有弹窗 */
  const shown = forced || revealed || fading || openAgent !== null || showHiddenPanel

  /** 单个 agent 头像 */
  const renderAvatar = (a: ExternalAgentStatus) => {
    const state = a.state || 'unknown'
    const stateLabel = t(STATE_I18N[state] || 'extAgents.state.unknown')
    const kind = agentKind(a.agent)
    const title = `${a.agent} · ${stateLabel}${a.task_id ? ` · ${a.task_id}` : ''}`
    return (
      <div
        key={a.agent}
        className={['agent-avatar-btn', STATE_CLASS[state] || 'is-unknown'].filter(Boolean).join(' ')}
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
            {kind === 'cli' ? <IconTerminal size={16} /> : <IconBot size={16} />}
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
      {(openAgent || showHiddenPanel) && (
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
        {hiddenKnown.length > 0 && (
          <button
            type="button"
            className={`hidden-agents-entry${showHiddenPanel ? ' active' : ''}`}
            onClick={() => {
              setOpenAgent(null)
              setShowHiddenPanel(v => !v)
            }}
            title={t('extAgents.hiddenCount', String(hiddenKnown.length))}
            aria-label={t('extAgents.hiddenCount', String(hiddenKnown.length))}
          >
            {hiddenKnown.length}
          </button>
        )}
        <button
          type="button"
          className="add-agent-entry"
          onClick={() => onOpenConfig?.()}
          title={t('extAgents.cfg.add')}
          aria-label={t('extAgents.cfg.add')}
        >
          <IconPlus size={16} />
        </button>

        {/* ── 交付物弹窗：锚定在胶囊上方，点遮罩 / Esc 关闭 ── */}
        {openAgent && (
          <div className="ext-agent-popover" role="dialog" aria-label={openAgent.agent}>
            <div className="ext-agent-popover-head">
              <span className={`agent-avatar-icon is-${agentKind(openAgent.agent)}`}>
                {agentKind(openAgent.agent) === 'cli' ? (
                  <IconTerminal size={14} />
                ) : (
                  <IconBot size={14} />
                )}
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
              {loadingDeliv ? (
                <div className="ext-agent-popover-empty">{t('extAgents.deliver.loading')}</div>
              ) : (deliverables || []).length === 0 ? (
                <div className="ext-agent-popover-empty">{t('extAgents.deliver.empty')}</div>
              ) : (
                [
                  reports.length > 0 ? (
                    <div key="reports" className="ext-agent-dgroup">
                      <div className="ext-agent-dgroup-label">
                        {t('extAgents.deliver.reports')}
                      </div>
                      {reports.map(d => (
                        <DeliverableRow key={d.path} d={d} onOpen={setPreviewPath} t={t} />
                      ))}
                    </div>
                  ) : null,
                  artifacts.length > 0 ? (
                    <div key="artifacts" className="ext-agent-dgroup">
                      <div className="ext-agent-dgroup-label">
                        {t('extAgents.deliver.artifacts')}
                      </div>
                      {artifacts.map(d => (
                        <DeliverableRow key={d.path} d={d} onOpen={setPreviewPath} t={t} />
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
                onClick={() => hideAgent(openAgent.agent)}
                title={t('extAgents.hide')}
              >
                {t('extAgents.hide')}
              </button>
            </div>
          </div>
        )}

        {/* ── 已隐藏 agent 恢复面板 ── */}
        {showHiddenPanel && hiddenKnown.length > 0 && (
          <div className="ext-agent-popover" role="dialog" aria-label={t('extAgents.restore')}>
            <div className="ext-agent-popover-head">
              <span className="ext-agent-popover-title">
                {t('extAgents.hiddenCount', String(hiddenKnown.length))}
              </span>
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
            <div className="ext-agent-popover-body">
              {hiddenKnown.map(name => (
                <div key={name} className="ext-agent-hidden-row">
                  <span className="ext-agent-hidden-name">{name}</span>
                  <button
                    type="button"
                    className="ext-agent-restore-btn"
                    onClick={() => restoreAgent(name)}
                  >
                    {t('extAgents.restore')}
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
      {previewPath && (
        <PreviewOverlay path={previewPath} onClose={() => setPreviewPath(null)} />
      )}
    </>
  )
}

interface DeliverableRowProps {
  d: AgentDeliverable
  onOpen: (path: string) => void
  t: (key: string, ...args: string[]) => string
}

function DeliverableRow({ d, onOpen, t }: DeliverableRowProps) {
  return (
    <button
      type="button"
      className="ext-agent-deliver-row"
      onClick={() => onOpen(d.path)}
      title={`${d.rel_path}\n${t('extAgents.deliver.preview')}`}
    >
      <IconFile size={14} />
      <span className="ext-agent-deliver-info">
        <span className="ext-agent-deliver-name">{d.name}</span>
        <span className="ext-agent-deliver-meta">
          {formatTime(d.modified)} · {formatSize(d.size)}
        </span>
      </span>
      <IconEye size={13} className="ext-agent-deliver-eye" />
    </button>
  )
}
