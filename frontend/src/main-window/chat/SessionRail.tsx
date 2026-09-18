import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { IconCheck, IconEdit3, IconFolder, IconPlus, IconTrash2, IconX } from '../../ui/Icons'
import { playUiSound } from '../../ui/sound'
import { CompactModal } from '../layout/CompactModal'
import { useLanguage } from '../../locales'
import {
  listShelfSessions,
  switchSession,
  renameSession,
  archiveSession,
  type ShelfSessionItem,
} from '../lib/api'
import '../../styles/session-rail.css'

const POLL_INTERVAL_MS = 5000
/** 会话变更去抖：2s 内只触发一次 onSessionChanged，防轮询翻转连续触发风暴 */
const SWITCH_NOTICE_THROTTLE_MS = 2000
/** 列表默认展示条数，超出部分折叠为「展开其余 N 个会话」（对齐参考会话栏版式） */
const COLLAPSED_LIMIT = 6

/** updated_at（Unix 毫秒）→ 行尾相对时间（刚刚 / N分钟 / N小时 / N天） */
function relativeTime(ms: number, t: (key: string, ...args: string[]) => string): string {
  const minutes = Math.floor((Date.now() - ms) / 60_000)
  if (minutes < 1) return t('sessionRail.timeJustNow')
  if (minutes < 60) return t('sessionRail.timeMinutes', String(minutes))
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return t('sessionRail.timeHours', String(hours))
  return t('sessionRail.timeDays', String(Math.floor(hours / 24)))
}

/** mode → 标识首字母（方形小标只放一个字母：L=leader / W=workflow / C=custom） */
function modeToLetter(mode: string): string {
  if (mode === 'workflow') return 'W'
  if (mode === 'leader') return 'L'
  if (mode === 'custom') return 'C'
  return '·'
}

interface SessionRailProps {
  /** 切换成功后由父级重拉 get_chat_history 整体替换气泡 */
  onSessionChanged: () => void
  /** 新建对话（复用桌面统一入口 handleNewChat / Ctrl+N 同一逻辑源；执行中禁用） */
  onNewChat?: () => void
  /** 打开项目中心弹窗（复用输入框项目 chip 的同一入口：ChatPanel setDirOpen(true)） */
  onOpenProjectDir?: () => void
  /** 跨 mode 会话切换成功后同步前端 mode state（后端原子切换不单独广播 mode_changed，
   *  mode chip 依赖此回调保持一致） */
  onModeSwitched?: (mode: string) => void
  /** 后端执行态实时镜像（App isProcessing）：执行中条目禁用（后端 guard 双保险） */
  locked?: boolean
  /** 当前情绪（App mood）：执行错误（'error'）时结束翻转不播完成音效，
   *  避免与 execution_error 的错误音效重叠 */
  mood?: string
}

type NoticeTone = 'info' | 'warning' | 'error'

/** 错误码 → i18n key（按业务/系统错误拆分，业务等待有专属细分文案） */
function codeToI18n(code: string): string {
  if (code === 'busy') return 'sessionRail.switchFailBusy'
  if (code === 'append_pending') return 'sessionRail.switchFailAppend'
  if (code === 'mode_mismatch') return 'sessionRail.switchFailMode'
  if (code === 'archiveFailGeneric') return 'sessionRail.archiveFailGeneric'
  return 'sessionRail.switchFailGeneric'
}

/** 错误码 → 视觉 tone：业务等待用 info（蓝），模式不匹配用 warning（橙），真错误用 error（红） */
function codeToTone(code: string): NoticeTone {
  if (code === 'busy' || code === 'append_pending') return 'info'
  if (code === 'mode_mismatch') return 'warning'
  return 'error'
}

/** 通知浮层图标：按 tone 切换内嵌 SVG，避免引入额外 icon 包污染主图标库 */
function NoticeIcon({ tone }: { tone: NoticeTone }) {
  if (tone === 'warning') {
    // 三角警示
    return (
      <svg className="sr-notice-icon" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path
          d="M8 2 L14.5 13.5 L1.5 13.5 Z"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinejoin="round"
        />
        <path d="M8 6 V9.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        <circle cx="8" cy="11.5" r="0.9" fill="currentColor" />
      </svg>
    )
  }
  if (tone === 'error') {
    // 圆 + 叹号
    return (
      <svg className="sr-notice-icon" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <circle cx="8" cy="8" r="6.2" stroke="currentColor" strokeWidth="1.4" />
        <path d="M8 5 V9.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        <circle cx="8" cy="11.5" r="0.9" fill="currentColor" />
      </svg>
    )
  }
  // info：圆 + i
  return (
    <svg className="sr-notice-icon" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <circle cx="8" cy="8" r="6.2" stroke="currentColor" strokeWidth="1.4" />
      <circle cx="8" cy="4.6" r="0.9" fill="currentColor" />
      <path d="M8 7 V11.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  )
}

/**
 * 会话工作台（Session Rail）：聊天面板左缘的滑动抽屉。
 * - 收起态：左缘常驻一枚色块（会话图标 + 当前会话 mode 首字母），是唯一可见元素。
 * - 展开态：点击色块 → 左侧列表滑出（标题 + 预览 + 重命名 / 归档全在这里可见）。
 * - 开合入口只有三个：色块点击、面板外点击、Esc；**不做 hover 感应唤出，
 *   执行完成也不自动弹出**（2026-09-15 大王反馈：隐藏式选择看不到会话标题）。
 * - 数据源与切换逻辑完全沿用：list_shelf_sessions（5s 轮询 + 可见性刷新），
 *   busy / 追加队列非空时后端拒绝 → 错误码映射文案在抽屉底部短暂浮现。
 */
export default function SessionRail({
  onSessionChanged,
  onNewChat,
  onOpenProjectDir,
  onModeSwitched,
  locked = false,
  mood,
}: SessionRailProps) {
  const { t } = useLanguage()
  const [items, setItems] = useState<ShelfSessionItem[]>([])
  const [canSwitch, setCanSwitch] = useState(true)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draftTitle, setDraftTitle] = useState('')
  const [notice, setNotice] = useState<{ text: string; tone: NoticeTone } | null>(null)
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** 手动归档确认弹窗目标会话 id（null = 关闭） */
  const [confirmArchiveId, setConfirmArchiveId] = useState<string | null>(null)
  /** 抽屉开合态：默认收起（只露色块），点击色块才伸出 */
  const [open, setOpen] = useState(false)
  /** 列表展开态：默认只显示前 COLLAPSED_LIMIT 条，其余折叠（对齐参考会话栏） */
  const [expanded, setExpanded] = useState(false)
  /** 执行中点色块的轻提示（色块旁浮出，数秒后自动消失） */
  const [chipHint, setChipHint] = useState(false)
  const chipHintTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const chipRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLElement>(null)
  const stoppedRef = useRef(false)
  /** 外部会话变化检测基准：上轮轮询的 active 会话 id（null=无 active；首轮回填不触发） */
  const lastActiveIdRef = useRef<string | null>(null)
  const initializedRef = useRef(false)
  /** 上次 canSwitch 状态：检测「执行开始/结束」的状态翻转轮（只校准基准，不触发重拉） */
  const prevCanSwitchRef = useRef(true)
  /** 检测触发去抖：上次 fire 时间戳（2s 内不重复触发重拉） */
  const lastFireAtRef = useRef(0)
  /** 列表签名：上轮渲染数据指纹（id+active+标题+顺序），结构未变不重绘 */
  const listSigRef = useRef('')

  // ── 执行态锁定：canSwitch（轮询后端权威）或 locked（实时镜像）任一执行中 → 条目禁用 ──
  const hardLocked = !canSwitch || locked

  // 翻转方向检测：记录上一帧 hardLocked，区分「执行开始」（false→true）与
  // 「执行完成」（true→false）——初始挂载 false→false 不触发任何动作
  const prevHardLockedRef = useRef(hardLocked)
  useEffect(() => {
    const was = prevHardLockedRef.current
    prevHardLockedRef.current = hardLocked
    if (hardLocked) {
      // 执行开始：收起抽屉（执行期面板只读，不遮挡消息流）
      setOpen(false)
      setEditingId(null)
    } else if (was) {
      // 执行完成：只播完成音效，不再自动弹出工作台（大王 2026-09-15：完成也不自动弹出）。
      // 错误结束（mood='error'）不播完成音——execution_error 已播错误音效，避免重叠
      if (mood !== 'error') playUiSound('done')
      // 执行结束：清掉残留的「执行中不可切换」提示
      setChipHint(false)
    }
  }, [hardLocked, mood])

  // 收起抽屉：保留编辑态草稿（重新展开后仍在编辑中），不做静默丢弃
  const closeDrawer = useCallback(() => setOpen(false), [])

  /** 执行中点色块的轻提示：色块旁浮出，3s 自动消失（重复点击重置计时） */
  const flashChipHint = useCallback(() => {
    setChipHint(true)
    if (chipHintTimer.current) clearTimeout(chipHintTimer.current)
    chipHintTimer.current = setTimeout(() => {
      chipHintTimer.current = null
      setChipHint(false)
    }, 3000)
  }, [])

  // 抽屉展开期：Esc 收起（编辑中先退编辑）、点击面板与色块之外收起
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (editingId) {
        setEditingId(null)
        return
      }
      setOpen(false)
    }
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node | null
      if (!target) return
      if (panelRef.current?.contains(target)) return
      if (chipRef.current?.contains(target)) return
      setOpen(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onDown)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onDown)
    }
  }, [open, editingId])

  useEffect(
    () => () => {
      if (noticeTimer.current) clearTimeout(noticeTimer.current)
      if (chipHintTimer.current) clearTimeout(chipHintTimer.current)
    },
    [],
  )

  /** onSessionChanged 经 ref 间接持有：保持 refresh 引用稳定（deps 不加回调），
   *  否则父组件每次渲染重建回调会导致轮询 effect 反复重启 */
  const onSessionChangedRef = useRef(onSessionChanged)
  onSessionChangedRef.current = onSessionChanged

  const refresh = useCallback(async () => {
    try {
      const r = await listShelfSessions()
      if (!stoppedRef.current && r) {
        const list = r.items || []
        const canSwitch = r.can_switch !== false
        // 签名守卫：id+active+标题+分钟桶未变则不 setItems——提炼/追加等后台写入只改
        // 消息内容与 updated_at，列表视图零重绘（消除轮询期闪动）；activeId 检测
        // 仍基于本轮新数据，不受影响。签名含顺序（数组序），新建/归档必然变化。
        // ⚠️ updated_at 以「分钟桶」入签名（非原始毫秒）：行尾相对时间需随分钟自增，
        //    否则无其它变化时列表永不重绘、时间会僵在旧值；分钟粒度最多每分钟一次重绘。
        const sig = list
          .map(
            i =>
              `${i.id}|${i.is_active ? 1 : 0}|${i.title}|${i.preview || ''}|${Math.floor(
                (i.updated_at || 0) / 60_000,
              )}`,
          )
          .join(';')
        if (sig !== listSigRef.current) {
          listSigRef.current = sig
          setItems(list)
        }
        setCanSwitch(canSwitch)
        // ── 外部会话变化检测 ──
        // 手机端「新会话」/ 遥控切换只广播给移动 WS，桌面前端没有该事件通道；
        // 本轮询是桌面感知外部会话变化的唯一信息源。
        // ⚠️ 仅在空闲态（can_switch）检测：执行期后端 get_chat_history 的会话解析源
        // 会在 session_backup 与 live agent 间漂移（实测日志交替返回），active id
        // 随之抖动——若此时比对会把执行期快照切换误判为外部变更，每次收发都整表
        // 重拉聊天区（实测回归）。执行中守卫本就禁止任何切换，不存在外部变更，
        // 直接冻结检测与基准更新。
        // ⚠️ 状态翻转轮（busy↔idle）：执行开始/结束时 active id 天然变化（agent
        // take/放回），此轮只校准基准不触发——否则「新会话/切换后第一轮回复完成」
        // 会因基准过期误判为外部变更，必然整表重拉（实测 2026-08-25）。
        const flip = prevCanSwitchRef.current !== canSwitch
        prevCanSwitchRef.current = canSwitch
        // 执行完成不再有弹窗/弹层动作；此处轮询只做整表重拉判定，flip 轮只校准基准
        const activeId = list.find(i => i.is_active)?.id ?? null
        if (flip) {
          lastActiveIdRef.current = activeId
          return
        }
        if (!canSwitch) return
        if (!initializedRef.current) {
          initializedRef.current = true
          lastActiveIdRef.current = activeId
        } else if (list.length > 0 && activeId !== lastActiveIdRef.current) {
          // 去抖：2s 内只触发一次，防连续变更风暴
          const now = Date.now()
          if (now - lastFireAtRef.current >= SWITCH_NOTICE_THROTTLE_MS) {
            lastFireAtRef.current = now
            lastActiveIdRef.current = activeId
            onSessionChangedRef.current()
          } else {
            lastActiveIdRef.current = activeId
          }
        } else {
          lastActiveIdRef.current = activeId
        }
      }
    } catch {
      /* 后端不可达：保留当前数据 */
    }
  }, [])

  useEffect(() => {
    stoppedRef.current = false
    let timer: ReturnType<typeof setTimeout> | null = null

    const poll = async () => {
      await refresh()
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
    if (document.visibilityState === 'visible') void poll()
    return () => {
      stoppedRef.current = true
      if (timer) clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [refresh])

  const flashNotice = useCallback(
    (code: string) => {
      const tone = codeToTone(code)
      setNotice({ text: t(codeToI18n(code)), tone })
      if (noticeTimer.current) clearTimeout(noticeTimer.current)
      // 业务等待（info）延长阅读时间；模式不匹配/真错误保留短促反馈
      const duration = tone === 'info' ? 3500 : 2400
      noticeTimer.current = setTimeout(() => setNotice(null), duration)
    },
    [t],
  )

  const handleSwitch = useCallback(
    async (id: string, isActive: boolean) => {
      if (isActive) return
      try {
        // 输入框 mode 以 session 选择为准：点击列表后，session 存储的 mode 是哪个，
        // 输入框 mode 就切到哪个。目标 mode 直接取条目存储归属（list 已保证
        // mode 来自 SQLite 快照，不依赖 currentMode 推断），无条件传给后端原子切换
        // （归档原槽→切 current_mode→安装目标槽；同 mode 点击走同槽切换，无副作用）。
        // 后端是唯一权威：mode_mismatch 安全失败，不污染状态。
        const target = items.find(i => i.id === id)
        if (!target) return
        await switchSession(id, target.mode)
        // 会话切换成功：轻触反馈（导航定位）
        playUiSound('session')
        // 同步前端输入框 mode（后端原子切换后前端 mode chip 保持一致）
        onModeSwitched?.(target.mode)
        // 主动切换：先登记基准，避免下轮轮询把这次变化再判成外部变更重复触发重拉
        lastActiveIdRef.current = id
        onSessionChanged()
        void refresh()
        // 切换完成即收起抽屉：立刻让出消息区视野（唯一自动收起场景，非弹出）
        setOpen(false)
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
      }
    },
    [items, onSessionChanged, onModeSwitched, refresh, flashNotice],
  )

  /** 手动归档：确认弹窗后移出展示台（元数据+文本记忆保留可查）；失败映射稳定错误码 */
  const handleArchive = useCallback(
    async (id: string) => {
      setConfirmArchiveId(null)
      try {
        await archiveSession(id)
        void refresh()
      } catch {
        flashNotice('archiveFailGeneric')
      }
    },
    [refresh, flashNotice],
  )

  const handleNewChat = useCallback(() => {
    setOpen(false)
    onNewChat?.()
  }, [onNewChat])

  /** 项目中心：复用输入框项目 chip 的同一入口（ChatPanel 的 setDirOpen(true)），
   *  打开前先收起抽屉，避免抽屉叠在弹窗后面 */
  const handleOpenProjectDir = useCallback(() => {
    setOpen(false)
    onOpenProjectDir?.()
  }, [onOpenProjectDir])

  const saveRename = useCallback(
    async (id: string) => {
      const draft = draftTitle.trim()
      setEditingId(null)
      if (!draft) return
      try {
        await renameSession(id, draft)
        void refresh()
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
      }
    },
    [draftTitle, refresh, flashNotice],
  )

  // hover 音效节流：鼠标扫过列表时避免连续爆音（120ms 内只响一次）
  const hoverSoundAt = useRef(0)
  const playHoverSound = useCallback(() => {
    const now = Date.now()
    if (now - hoverSoundAt.current < 120) return
    hoverSoundAt.current = now
    playUiSound('session')
  }, [])

  const visibleItems = expanded ? items : items.slice(0, COLLAPSED_LIMIT)
  const restCount = Math.max(0, items.length - COLLAPSED_LIMIT)

  return (
    <>
      {/* ── 收起色块（常驻）：沿用原「引导标志块」样式 —— 8×58 竖条，左缘贴边、
          右侧圆帽、实心 fg-5，hover 转 accent + 微光。点击展开/收起左侧抽屉，
          是收起态唯一可见元素（无 hover 感应唤出、无执行完成自动弹出）。
          执行中不可开合：点击只浮出「执行中不可切换」轻提示。 ── */}
      <button
        ref={chipRef}
        type="button"
        className="session-rail-chip"
        onClick={() => {
          // 执行中不可开合：给轻提示，不弹抽屉（后端 guard 之外再给个明确反馈）
          if (hardLocked) {
            flashChipHint()
            return
          }
          setOpen(o => !o)
        }}
        aria-expanded={open ? true : undefined}
        aria-disabled={hardLocked || undefined}
        tabIndex={open ? -1 : undefined}
        aria-label={t('sessionRail.title')}
        title={t('sessionRail.title')}
      />

      {/* 执行中点色块的轻提示：色块右侧浮出，3s 自动消失 */}
      {chipHint && (
        <div className="sr-chip-hint" role="status" aria-live="polite">
          <NoticeIcon tone="info" />
          <span>{t('sessionRail.busyHint')}</span>
        </div>
      )}

      {/* 点击面板外收起：透明承接层，不压暗消息流 */}
      <div
        className={`session-rail-scrim${open ? ' is-open' : ''}`}
        aria-hidden="true"
        onMouseDown={closeDrawer}
      />

      {/* ── 左侧滑动栏（抽屉）：默认平移出面板左缘外，点击色块后滑出。
          常驻挂载以保留滑动过渡；visibility 断开关闭态的 tab 焦点与命中测试。 ── */}
      <aside
        ref={panelRef}
        className={`session-rail-drawer${open ? ' is-open' : ''}`}
        role="navigation"
        aria-label={t('sessionRail.title')}
        aria-hidden={open ? undefined : true}
      >
        <div className="sr-drawer-head">
          <span className="sr-drawer-title">{t('sessionRail.title')}</span>
          {/* 项目目录：与输入框项目 chip 同一入口（ChatPanel setDirOpen(true) → ProjectCenter） */}
          {onOpenProjectDir && (
            <button
              type="button"
              className="sr-head-btn"
              onClick={handleOpenProjectDir}
              title={t('sessionRail.projectDir')}
              aria-label={t('sessionRail.projectDir')}
            >
              <IconFolder size={14} />
            </button>
          )}
          <button
            type="button"
            className="sr-head-btn"
            onClick={closeDrawer}
            title={t('sessionRail.collapse')}
            aria-label={t('sessionRail.collapse')}
          >
            <IconX size={14} />
          </button>
        </div>
        {/* 新会话：整行浅色实心按钮（与 Ctrl+N / TitleBar 同一逻辑源） */}
        {onNewChat && (
          <div className="sr-new-chat-wrap">
            <button
              type="button"
              className="sr-new-chat-btn"
              onClick={handleNewChat}
              disabled={hardLocked}
              title={t('sessionRail.newChat')}
            >
              <IconPlus size={15} />
              <span>{t('sessionRail.newChat')}</span>
            </button>
          </div>
        )}
        <div className="sr-list">
          {visibleItems.map(it => {
            const modeText =
              it.mode === 'workflow'
                ? t('input.mode.workflow')
                : it.mode === 'leader'
                  ? t('input.mode.leader')
                  : it.mode === 'custom'
                    ? t('input.mode.custom')
                    : ''
            return (
              <div
                key={it.id}
                className={`sr-item${it.is_active ? ' active' : ''}${
                  editingId === it.id ? ' editing' : ''
                }`}
                onMouseEnter={() => {
                  // hover 可切换项（非当前、非编辑态、未锁定）响轻音反馈
                  if (!it.is_active && editingId !== it.id && !hardLocked) playHoverSound()
                }}
              >
                {editingId === it.id ? (
                  <div className="sr-head">
                    <input
                      className="sr-rename-input"
                      value={draftTitle}
                      autoFocus
                      maxLength={60}
                      placeholder={it.title || t('sessionRail.untitled')}
                      onChange={e => setDraftTitle(e.target.value)}
                      onKeyDown={e => {
                        if (e.key === 'Enter' && draftTitle.trim()) void saveRename(it.id)
                        if (e.key === 'Escape') {
                          // 只退编辑、不收起抽屉（拦下冒泡给 document 的 Esc 收起监听）
                          e.stopPropagation()
                          setEditingId(null)
                        }
                      }}
                    />
                    <button
                      type="button"
                      className="sr-edit-btn"
                      onClick={() => void saveRename(it.id)}
                      disabled={!draftTitle.trim()}
                      title={t('sessionRail.save')}
                      aria-label={t('sessionRail.save')}
                    >
                      <IconCheck size={13} />
                    </button>
                    <button
                      type="button"
                      className="sr-edit-btn"
                      onClick={() => setEditingId(null)}
                      title={t('common.cancel')}
                      aria-label={t('common.cancel')}
                    >
                      <IconX size={12} />
                    </button>
                  </div>
                ) : (
                  <>
                    <span
                      className={`sr-mode-badge mode-${it.mode}`}
                      title={modeText}
                      aria-label={modeText}
                    >
                      {modeToLetter(it.mode)}
                    </span>
                    <button
                      type="button"
                      className={`sr-title-btn${it.is_active ? ' active' : ''}`}
                      disabled={it.is_active || hardLocked}
                      aria-current={it.is_active ? 'true' : undefined}
                      title={it.title || t('sessionRail.untitled')}
                      onClick={() => void handleSwitch(it.id, it.is_active)}
                    >
                      {it.title || t('sessionRail.untitled')}
                    </button>
                    {it.is_active && (
                      <span className="sr-current-badge" aria-hidden="true">
                        {t('sessionRail.current')}
                      </span>
                    )}
                    {/* 行尾相对时间：hover 时淡出让位给操作按钮，避免按钮挤动布局 */}
                    <span className="sr-time">{relativeTime(it.updated_at, t)}</span>
                    <span className="sr-actions">
                      <button
                        type="button"
                        className="sr-edit-btn"
                        onClick={() => {
                          setDraftTitle(it.title)
                          setEditingId(it.id)
                        }}
                        title={t('sessionRail.rename')}
                        aria-label={t('sessionRail.rename')}
                      >
                        <IconEdit3 size={12} />
                      </button>
                      {!it.is_active && (
                        <button
                          type="button"
                          className="sr-edit-btn sr-archive-btn"
                          onClick={() => setConfirmArchiveId(it.id)}
                          disabled={!canSwitch}
                          title={t('sessionRail.archive')}
                          aria-label={t('sessionRail.archive')}
                        >
                          <IconTrash2 size={12} />
                        </button>
                      )}
                    </span>
                  </>
                )}
              </div>
            )
          })}
          {/* 折叠行：默认只列 COLLAPSED_LIMIT 条，其余点开后展开（对齐参考会话栏） */}
          {!expanded && restCount > 0 && (
            <button type="button" className="sr-more-btn" onClick={() => setExpanded(true)}>
              {t('sessionRail.expandMore', String(restCount))}
            </button>
          )}
        </div>

        {notice && (
          <div
            className={`sr-notice sr-notice--${notice.tone}`}
            role={notice.tone === 'error' ? 'alert' : 'status'}
            aria-live={notice.tone === 'error' ? 'assertive' : 'polite'}
          >
            <NoticeIcon tone={notice.tone} />
            <span>{notice.text}</span>
          </div>
        )}
      </aside>

      {confirmArchiveId &&
        createPortal(
          <CompactModal
            open
            onClose={() => setConfirmArchiveId(null)}
            title={t('sessionRail.archiveConfirmTitle')}
            size="sm"
            className="compact-modal--fit"
            footer={
              <>
                <button
                  type="button"
                  className="btn btn-ghost"
                  onClick={() => setConfirmArchiveId(null)}
                >
                  {t('common.cancel')}
                </button>
                <button
                  type="button"
                  className="btn btn-danger"
                  onClick={() => void handleArchive(confirmArchiveId)}
                >
                  {t('sessionRail.archive')}
                </button>
              </>
            }
          >
            <div className="sr-confirm-desc">{t('sessionRail.archiveConfirmDesc')}</div>
          </CompactModal>,
          document.body,
        )}
    </>
  )
}
