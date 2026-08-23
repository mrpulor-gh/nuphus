import { useCallback, useEffect, useRef, useState } from 'react'
import { IconCheck, IconEdit3, IconX } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import {
  listShelfSessions,
  switchSession,
  renameSession,
  type ShelfSessionItem,
} from '../lib/api'
import '../../styles/session-rail.css'

const POLL_INTERVAL_MS = 5000

interface SessionRailProps {
  /** 切换成功后由父级重拉 get_chat_history 整体替换气泡 */
  onSessionChanged: () => void
}

function errorCodeToI18n(code: string): string {
  if (code === 'busy') return 'sessionRail.switchFailBusy'
  if (code === 'append_pending') return 'sessionRail.switchFailAppend'
  if (code === 'mode_mismatch') return 'sessionRail.switchFailMode'
  return 'sessionRail.switchFailGeneric'
}

/**
 * 会话展示台（Session Rail）：输入框区域左缘的感应式竖排横杠。
 * - 数据源：list_shelf_sessions（5s 轮询 + 页面可见性刷新），active 置顶。
 * - 静止态仅微露短杠；hover 感应区阶梯展开（20ms/条 stagger）；单条 hover 弹出
 *   玻璃气泡显示标题，气泡内可重命名（Enter 保存 / Esc 取消）。
 * - 点击横杠切换会话（busy / 追加队列非空时后端拒绝 → 错误码映射文案短暂浮现，
 *   横杠同步 shake）。底部虚线杠为「新建对话」入口。
 * - 执行中整轨降透明度锁定（can_switch），与后端守卫双保险。
 */
export default function SessionRail({ onSessionChanged }: SessionRailProps) {
  const { t } = useLanguage()
  const [items, setItems] = useState<ShelfSessionItem[]>([])
  const [canSwitch, setCanSwitch] = useState(true)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draftTitle, setDraftTitle] = useState('')
  const [err, setErr] = useState<string | null>(null)
  const errTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const stoppedRef = useRef(false)
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

  const refresh = useCallback(async () => {
    try {
      const r = await listShelfSessions()
      if (!stoppedRef.current && r) {
        setItems(r.items || [])
        setCanSwitch(r.can_switch !== false)
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

  const flashError = useCallback(
    (code: string) => {
      setErr(t(errorCodeToI18n(code)))
      if (errTimer.current) clearTimeout(errTimer.current)
      errTimer.current = setTimeout(() => setErr(null), 2400)
    },
    [t],
  )

  const handleSwitch = useCallback(
    async (id: string, isActive: boolean) => {
      if (isActive) return
      try {
        await switchSession(id)
        onSessionChanged()
        void refresh()
      } catch (e) {
        flashError(typeof e === 'string' ? e : String(e))
      }
    },
    [onSessionChanged, refresh, flashError],
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
        flashError(typeof e === 'string' ? e : String(e))
      }
    },
    [draftTitle, refresh, flashError],
  )

  const shown = revealed || fading || editingId !== null

  return (
    <>
      {/* 左缘感应区：横向自左边框至内容气泡前，纵向覆盖 10 横条显示高度；
          pointer-events:none 纯几何判定（window mousemove），绝不拦截输入框点击 */}
      <div ref={zoneRef} className="session-rail-hover-zone" aria-hidden />
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
        {items.map((it, idx) => (
          <div
            key={it.id}
            className={`sr-item${it.is_active ? ' active' : ''}${
              editingId === it.id ? ' editing' : ''
            }`}
          >
            <button
              type="button"
              className="sr-bar"
              style={{ transitionDelay: `${idx * 0.02}s` }}
              onClick={() => void handleSwitch(it.id, it.is_active)}
              aria-label={it.title || t('sessionRail.untitled')}
              aria-current={it.is_active ? 'true' : undefined}
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
                  <span className="sr-title" title={it.title}>
                    {it.title || t('sessionRail.untitled')}
                  </span>
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
                </>
              )}
            </div>
          </div>
        ))}
      </div>

      {err && (
        <div className="sr-error" role="alert">
          {err}
        </div>
      )}
      </div>
    </>
  )
}
