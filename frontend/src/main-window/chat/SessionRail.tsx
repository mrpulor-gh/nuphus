import { useCallback, useEffect, useRef, useState } from 'react'
import { IconCheck, IconEdit3 } from '../../ui/Icons'
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

  // ── 整轨隐显：感应区进入即展开；移开 10s 后 0.6s 渐隐（无强制常驻条件）──
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
          常驻透明 DOM，z-index 低于轨道避免遮挡其交互 */}
      <div
        className="session-rail-hover-zone"
        onMouseEnter={handleZoneEnter}
        onMouseLeave={handleZoneLeave}
        aria-hidden
      />
      <div
        className={[
          'session-rail-zone',
          canSwitch ? '' : 'is-locked',
          shown ? (fading ? 'fading' : 'revealed') : 'concealed',
        ]
          .filter(Boolean)
          .join(' ')}
        onMouseEnter={handleZoneEnter}
        onMouseLeave={handleZoneLeave}
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
                    onChange={e => setDraftTitle(e.target.value)}
                    onKeyDown={e => {
                      if (e.key === 'Enter') void saveRename(it.id)
                      if (e.key === 'Escape') setEditingId(null)
                    }}
                  />
                  <button
                    type="button"
                    className="sr-edit-btn"
                    onClick={() => void saveRename(it.id)}
                    title={t('sessionRail.save')}
                    aria-label={t('sessionRail.save')}
                  >
                    <IconCheck size={13} />
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
