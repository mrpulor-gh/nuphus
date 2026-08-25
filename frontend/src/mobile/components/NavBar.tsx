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
import { ALargeSmall, Check, Layers, Moon, RotateCcw, Settings, Sun, Wrench, X } from 'lucide-react'
import { getTheme, toggleTheme, type MobileTheme } from '../theme'
import { getCachedLanUrl, getCachedRelayUrl } from '../connection'
import { getFontSize, setFontSize, type MobileFontSize } from '../fontsize'
import type { ShelfSessions } from '../api'
import type { ActivityState } from '../store'
import type { WsStatus } from '../ws'
import { t } from '../i18n'

interface Props {
  wsStatus: WsStatus
  activity: ActivityState
  /** 新会话（点击 logo 菜单触发）：清空前端消息 */
  onNewChat?: () => void
  /** 重置连接（设置弹窗触发）：清除 token 回到配对页 */
  onDisconnect?: () => void
  /** 桌面展示台会话清单镜像（null = 未加载/不可用，隐藏入口） */
  sessions?: ShelfSessions | null
  /** 遥控切换桌面当前会话（切的就是电脑端正显示的视图） */
  onSwitchSession?: (id: string) => void
}

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
  // 品牌菜单（点击 logo 弹出：新会话 / 会话 / 设置）与设置弹窗
  const [logoMenuOpen, setLogoMenuOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [sessionsOpen, setSessionsOpen] = useState(false)
  const logoWrapRef = useRef<HTMLDivElement>(null)
  const settingsSheetRef = useRef<HTMLDivElement>(null)
  const sessionsSheetRef = useRef<HTMLDivElement>(null)

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

  // 品牌菜单 / 设置弹窗 / 会话清单：点击外部关闭（轻量 popover，不叠加遮罩）
  useEffect(() => {
    if (!logoMenuOpen && !settingsOpen && !sessionsOpen) return
    const onDown = (e: MouseEvent | TouchEvent) => {
      const t = e.target as Node
      if (logoWrapRef.current?.contains(t)) return
      if (settingsSheetRef.current?.contains(t)) return
      if (sessionsSheetRef.current?.contains(t)) return
      setLogoMenuOpen(false)
      setSettingsOpen(false)
      setSessionsOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('touchstart', onDown)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('touchstart', onDown)
    }
  }, [logoMenuOpen, settingsOpen, sessionsOpen])

  const pickFontSize = (size: MobileFontSize) => {
    setFs(setFontSize(size))
    setFsOpen(false)
  }

  /** 品牌区：点击 logo 弹出菜单（新会话 / 设置），弹窗样式复用字号浮层（mobile-nav-fs-pop） */
  const renderBrand = () => (
    <div className="mobile-nav-brand-wrap" ref={logoWrapRef}>
      <button
        type="button"
        className="mobile-nav-brand"
        onClick={() => setLogoMenuOpen(o => !o)}
        aria-label="菜单"
        aria-haspopup="menu"
        aria-expanded={logoMenuOpen}
      >
        <BlinkLogo />
        <span className="mobile-nav-title">Nuphus</span>
      </button>
      {logoMenuOpen && (
        <div className="mobile-nav-fs-pop mobile-nav-logo-pop" role="menu" aria-label="菜单">
          <div className="mobile-nav-fs-title">菜单</div>
          <button
            type="button"
            role="menuitem"
            className="mobile-nav-fs-opt"
            disabled={activity.running}
            onClick={() => {
              setLogoMenuOpen(false)
              onNewChat?.()
            }}
          >
            <RotateCcw size={15} aria-hidden="true" />
            <span className="mobile-nav-fs-label">新会话</span>
            {activity.running && <span className="mobile-nav-fs-lock">执行中</span>}
          </button>
          <button
            type="button"
            role="menuitem"
            className="mobile-nav-fs-opt"
            onClick={() => {
              setLogoMenuOpen(false)
              setSessionsOpen(true)
            }}
          >
            <Layers size={15} aria-hidden="true" />
            <span className="mobile-nav-fs-label">会话</span>
          </button>
          <button
            type="button"
            role="menuitem"
            className="mobile-nav-fs-opt"
            onClick={() => {
              setLogoMenuOpen(false)
              setSettingsOpen(true)
            }}
          >
            <Settings size={15} aria-hidden="true" />
            <span className="mobile-nav-fs-label">设置</span>
          </button>
        </div>
      )}
    </div>
  )

  /** 设置弹窗：连接信息 / 重置 / 关于（点击 logo → 设置 打开；fixed 底部弹出） */
  const renderSettings = () =>
    settingsOpen ? (
      <div className="mobile-settings-sheet" role="dialog" aria-label="设置" ref={settingsSheetRef}>
        <div className="mobile-mode-head">
          <span className="mobile-mode-title">设置</span>
          <button
            type="button"
            className="mobile-model-card-x"
            onClick={() => setSettingsOpen(false)}
            aria-label="关闭"
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="mobile-settings-info">
          <div className="mobile-settings-row">
            <span className="mobile-settings-label">连接</span>
            <span className="mobile-settings-value">{getCachedLanUrl() || '局域网直连'}</span>
          </div>
          {getCachedRelayUrl() && (
            <div className="mobile-settings-row">
              <span className="mobile-settings-label">中继</span>
              <span className="mobile-settings-value">{getCachedRelayUrl()}</span>
            </div>
          )}
          <div className="mobile-settings-row">
            <span className="mobile-settings-label">关于</span>
            <span className="mobile-settings-value">Nuphus 移动端</span>
          </div>
        </div>
        <button
          type="button"
          className="mobile-settings-disconnect"
          onClick={() => {
            setSettingsOpen(false)
            onDisconnect?.()
          }}
        >
          重置
        </button>
        <div className="mobile-settings-reset-hint">清除配对信息并断开连接，需重新扫码配对</div>
      </div>
    ) : null

  /** 会话清单弹层：桌面展示台镜像（切的就是电脑端正显示的视图）。
   *  复用设置弹层样式（mobile-settings-sheet），条目高亮当前会话。 */
  const renderSessions = () =>
    sessionsOpen ? (
      <div className="mobile-settings-sheet" role="dialog" aria-label="会话" ref={sessionsSheetRef}>
        <div className="mobile-mode-head">
          <span className="mobile-mode-title">会话</span>
          <button
            type="button"
            className="mobile-model-card-x"
            onClick={() => setSessionsOpen(false)}
            aria-label="关闭"
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>
        {sessions && sessions.items.length > 0 ? (
          <div className="mobile-sess-list">
            {sessions.items.map(item => (
              <button
                key={item.id}
                type="button"
                className={[
                  'mobile-sess-item',
                  item.is_active ? 'is-active' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                onClick={() => {
                  if (item.is_active) {
                    setSessionsOpen(false)
                    return
                  }
                  setSessionsOpen(false)
                  onSwitchSession?.(item.id)
                }}
              >
                <span className="mobile-sess-title">{item.title || item.id}</span>
                <span className="mobile-sess-meta">
                  {item.is_active ? '当前 · ' : ''}
                  {item.message_count} 条
                </span>
              </button>
            ))}
          </div>
        ) : (
          <div className="mobile-sess-empty">暂无会话记录</div>
        )}
        <div className="mobile-settings-reset-hint">切换的是电脑端正在显示的对话，两端同步</div>
      </div>
    ) : null

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
        {renderSessions()}
      </>
    )
  }

  // 空闲：连接状态（居中锚点）
  return (
    <>
      <header className="mobile-nav">
        {renderBrand()}
        <div className="mobile-nav-center">
          <span className={`mobile-status-pill is-${wsStatus}`} role="status">
            <span className="mobile-status-dot" aria-hidden="true" />
            {STATUS_LABEL[wsStatus]()}
          </span>
        </div>
        {renderActions()}
      </header>
      {renderSettings()}
      {renderSessions()}
    </>
  )
}