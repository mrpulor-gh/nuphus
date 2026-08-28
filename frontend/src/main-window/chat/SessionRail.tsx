import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { IconCheck, IconEdit3, IconX, IconTrash2, IconPlus } from '../../ui/Icons'
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

interface SessionRailProps {
  /** 切换成功后由父级重拉 get_chat_history 整体替换气泡 */
  onSessionChanged: () => void
  /** 新建对话（复用桌面统一入口 handleNewChat / Ctrl+N 同一逻辑源；执行中禁用） */
  onNewChat?: () => void
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
 * 会话展示台（Session Rail）：输入框区域左缘的感应式竖排横杠。
 * - 数据源：list_shelf_sessions（5s 轮询 + 页面可见性刷新），active 置顶。
 * - 静止态仅微露短杠；hover 感应区阶梯展开（20ms/条 stagger）；单条 hover 弹出
 *   玻璃气泡显示标题，气泡内可重命名（Enter 保存 / Esc 取消）。
 * - 点击横杠切换会话（busy / 追加队列非空时后端拒绝 → 错误码映射文案短暂浮现，
 *   横杠同步 shake）。新建对话入口在 TitleBar 与 Ctrl+N（底部无虚线杠）。
 * - 执行中整轨降透明度锁定（can_switch），与后端守卫双保险。
 */
export default function SessionRail({ onSessionChanged, onNewChat }: SessionRailProps) {
  const { t } = useLanguage()
  const [items, setItems] = useState<ShelfSessionItem[]>([])
  const [canSwitch, setCanSwitch] = useState(true)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draftTitle, setDraftTitle] = useState('')
  const [notice, setNotice] = useState<{ text: string; tone: NoticeTone } | null>(null)
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** 手动归档确认弹窗目标会话 id（null = 关闭） */
  const [confirmArchiveId, setConfirmArchiveId] = useState<string | null>(null)
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
  /** 整轨隐显：默认隐身，左缘感应区唤醒；移开 10s 后渐隐 */
  const [revealed, setRevealed] = useState(false)
  const [fading, setFading] = useState(false)
  const leaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const fadeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** 编辑中镜像：渐隐定时器回调里读最新值，避免闭包过期 */
  const editingRef = useRef(false)

  useEffect(() => {
    editingRef.current = editingId !== null
  }, [editingId])

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
        // 签名守卫：id+active+标题+顺序未变则不 setItems——提炼/追加等后台写入只改
        // 消息内容与 updated_at，列表视图零重绘（消除轮询期闪动）；activeId 检测
        // 仍基于本轮新数据，不受影响。签名含顺序（数组序），新建/归档必然变化。
        const sig = list.map(i => `${i.id}|${i.is_active ? 1 : 0}|${i.title}|${i.preview || ''}`).join(';')
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
        // 执行完成不再自动弹出整条 rail（2026-08-26）：UI 只响应用户显式交互或
        // 外部会话变更；执行/提炼的后台写入对 rail 完全静默，唤醒走左缘感应区。
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
        await switchSession(id)
        // 主动切换：先登记基准，避免下轮轮询把这次变化再判成外部变更重复触发重拉
        lastActiveIdRef.current = id
        onSessionChanged()
        void refresh()
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
      }
    },
    [onSessionChanged, refresh, flashNotice],
  )

  /** 手动归档：确认弹窗后移出展示台（元数据+文本记忆保留可查）；失败映射稳定错误码 */
  const handleArchive = useCallback(
    async (id: string) => {
      setConfirmArchiveId(null)
      try {
        await archiveSession(id)
        void refresh()
      } catch (e) {
        const code = typeof e === 'string' ? e : String(e)
        // busy / 追加队列非空复用切换守卫文案（业务等待=info 蓝）；其他走归档失败兜底（error 红）
        if (code === 'busy' || code === 'append_pending') {
          flashNotice(code)
        } else {
          flashNotice('archiveFailGeneric')
        }
      }
    },
    [refresh, flashNotice, t],
  )

  // ── 整轨隐显：感应区为纯几何判定（pointer-events:none），不拦截任何点击；
  //    window mousemove + rAF 节流做矩形包含检测，移开 10s 后 0.6s 渐隐 ──
  const zoneRef = useRef<HTMLDivElement>(null)
  const rafId = useRef(0)
  const insideRef = useRef(false)

  const startReveal = useCallback(() => {
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

  const scheduleHide = useCallback(() => {
    // 编辑标题期间绝不启动渐隐倒计时
    if (editingRef.current) return
    if (leaveTimer.current) return
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

  const cancelHide = useCallback(() => {
    if (leaveTimer.current) {
      clearTimeout(leaveTimer.current)
      leaveTimer.current = null
    }
    if (fadeTimer.current) {
      clearTimeout(fadeTimer.current)
      fadeTimer.current = null
    }
  }, [])

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (rafId.current) return
      rafId.current = requestAnimationFrame(() => {
        rafId.current = 0
        const el = zoneRef.current
        if (!el) return
        const r = el.getBoundingClientRect()
        const inside =
          e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom
        if (inside === insideRef.current) return
        insideRef.current = inside
        if (inside) {
          startReveal()
        } else {
          scheduleHide()
        }
      })
    }
    window.addEventListener('mousemove', onMove)
    return () => {
      window.removeEventListener('mousemove', onMove)
      if (rafId.current) cancelAnimationFrame(rafId.current)
    }
  }, [startReveal, scheduleHide])

  // 卸载清理
  useEffect(
    () => () => {
      if (leaveTimer.current) clearTimeout(leaveTimer.current)
      if (fadeTimer.current) clearTimeout(fadeTimer.current)
    },
    [],
  )

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

  // 执行中（!canSwitch）强制隐藏：此时 rail 锁定不可切换，显示出来只会让用户
  // 「看得见摸不着」（旧逻辑 hover 会 reveal 但切换被禁）。完成后经 can_switch
  // 翻转强制 reveal（见 refresh）。其余隐藏/渐隐逻辑保持现状。
  const shown = canSwitch && (revealed || fading || editingId !== null)

  return (
    <>
      {/* 左缘感应区：横向自左边框至内容气泡前，纵向覆盖 10 横条显示高度；
          pointer-events:none 纯几何判定（window mousemove），绝不拦截输入框点击 */}
      <div ref={zoneRef} className="session-rail-hover-zone" aria-hidden />
      {/* 引导标志块：工作台隐藏（concealed）时在左上角露出小竖条，暗示此处有内容可展开；
          左缘贴应用边框无间距，右侧上下椭圆角（圆帽）。hover 标志块弹出标题气泡
          「会话工作台」（复用 sr-bubble 样式，双向 hover 桥接防间隙丢失）。 */}
      {!shown && (
        <div className="session-rail-guide" aria-hidden>
          <span className="session-rail-guide-bar" />
          <span className="session-rail-guide-title">{t('sessionRail.guideTitle')}</span>
        </div>
      )}
      <div
        className={[
          'session-rail-zone',
          canSwitch ? '' : 'is-locked',
          shown ? (fading ? 'fading' : 'revealed') : 'concealed',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        <div className="session-rail" role="navigation" aria-label={t('sessionRail.title')}>
          {/* 顶部新建对话：复用外部 Agent 状态栏 + 按钮（add-agent-entry 视觉），
            与 Ctrl+N / TitleBar 同一逻辑源（onNewChat 由 App 注入 handleNewChat）。
            执行中（!canSwitch）禁用：后端 guard_switch 也会拒绝，UI 双保险。 */}
          {onNewChat && (
            <div className="sr-new-chat">
              <button
                type="button"
                className="add-agent-entry sr-new-chat-btn"
                onClick={onNewChat}
                disabled={!canSwitch}
                title={t('sessionRail.newChat')}
                aria-label={t('sessionRail.newChat')}
              >
                <IconPlus size={15} />
              </button>
            </div>
          )}
          {items.map((it, idx) => (
            <div
              key={it.id}
              className={`sr-item${it.is_active ? ' active' : ''}${
                editingId === it.id ? ' editing' : ''
              }`}
            >
              <span
                className="sr-bar"
                style={{ transitionDelay: `${idx * 0.02}s` }}
                aria-hidden="true"
              />
              <div className="sr-bubble">
                {editingId === it.id ? (
                  <>
                    <input
                      className="sr-rename-input"
                      value={draftTitle}
                      autoFocus
                      maxLength={60}
                      placeholder={it.title || t('sessionRail.untitled')}
                      onChange={e => setDraftTitle(e.target.value)}
                      onKeyDown={e => {
                        if (e.key === 'Enter' && draftTitle.trim()) void saveRename(it.id)
                        if (e.key === 'Escape') setEditingId(null)
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
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      className={`sr-title-btn${it.is_active ? ' active' : ''}`}
                      disabled={it.is_active}
                      onClick={() => void handleSwitch(it.id, it.is_active)}
                    >
                      {it.title || t('sessionRail.untitled')}
                    </button>
                    {/* hover 长预览：agent 最终回复（脱敏截断），与标题「话题 ↔ 结果」互补；
                        DOM 呈现不受条目空间限制（原生 title 属性长文本体验差） */}
                    {it.preview ? (
                      <div className="sr-tip" role="tooltip">
                        {it.preview}
                      </div>
                    ) : null}
                    <button
                      type="button"
                      className="sr-edit-btn"
                      onClick={() => {
                        setDraftTitle(it.title)
                        setEditingId(it.id)
                        // 编辑期间冻结隐显：取消在途倒计时并强制常亮
                        cancelHide()
                        setFading(false)
                        setRevealed(true)
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
                  </>
                )}
              </div>
            </div>
          ))}
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
      </div>
    </>
  )
}