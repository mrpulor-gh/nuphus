import { useState, useEffect } from 'react'
import { getCurrentWindow, type Window } from '@tauri-apps/api/window'
import { NuphusAvatar, type NuphusAvatarState } from '../../ui/NuphusAvatar'
import { IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'

interface TitleBarProps {
  onNewChat?: () => void
  agentState?: NuphusAvatarState
}

export function TitleBar({ onNewChat, agentState = 'idle' }: TitleBarProps) {
  const { t } = useLanguage()
  const [menuOpen, setMenuOpen] = useState(false)
  const [win, setWin] = useState<Window | null>(null)
  useEffect(() => {
    // 浏览器调试环境无 Tauri 运行时，跳过窗口 API，避免整棵组件树崩溃
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
    if (isTauri) {
      try {
        setWin(getCurrentWindow())
      } catch {
        setWin(null)
      }
    }
  }, [])

  const minimize = () => win?.minimize()
  const toggleMaximize = async () => {
    const w = win
    if (!w) return
    const isMax = await w.isMaximized()
    if (isMax) await w.unmaximize()
    else await w.maximize()
  }
  const closeWindow = () => win?.close()

  return (
    <header className="title-bar">
      <div className="title-bar-left" data-tauri-drag-region>
        <div className="title-bar-icon">
          <NuphusAvatar state={agentState} size={20} />
        </div>
        <span className="title-bar-brand">Nuphus</span>
      </div>
      <span className="title-bar-spacer" data-tauri-drag-region />
      {/* 桌面端：窗口控制按钮 */}
      <div className="title-bar-right title-bar-desktop-controls">
        <IconButton variant="win-btn" label={t('titleBar.minimize')} onClick={minimize}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 14 14"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          >
            <line x1="3" y1="7" x2="11" y2="7" />
          </svg>
        </IconButton>
        <IconButton variant="win-btn" label={t('titleBar.maximize')} onClick={toggleMaximize}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 14 14"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <rect x="2.5" y="2.5" width="9" height="9" rx="1.5" />
          </svg>
        </IconButton>
        <IconButton variant="win-close" label={t('titleBar.close')} onClick={closeWindow}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 14 14"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          >
            <line x1="3.5" y1="3.5" x2="10.5" y2="10.5" />
            <line x1="10.5" y1="3.5" x2="3.5" y2="10.5" />
          </svg>
        </IconButton>
      </div>
      {/* 移动端：汉堡菜单 */}
      <div className="title-bar-right title-bar-mobile-controls">
        <IconButton
          variant="win-btn"
          label={t('titleBar.menu')}
          onClick={() => setMenuOpen(v => !v)}
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
          >
            <line x1="4" y1="6" x2="20" y2="6" />
            <line x1="4" y1="12" x2="20" y2="12" />
            <line x1="4" y1="18" x2="20" y2="18" />
          </svg>
        </IconButton>
        {menuOpen && (
          <>
            <div className="hamburger-backdrop" onClick={() => setMenuOpen(false)} />
            <div className="hamburger-menu">
              <button
                className="hamburger-menu-item"
                onClick={() => {
                  onNewChat?.()
                  setMenuOpen(false)
                }}
              >
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                >
                  <line x1="12" y1="5" x2="12" y2="19" />
                  <line x1="5" y1="12" x2="19" y2="12" />
                </svg>
                <span>{t('titleBar.newChat')}</span>
              </button>
            </div>
          </>
        )}
      </div>
    </header>
  )
}
