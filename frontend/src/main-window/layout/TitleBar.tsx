import { useState, useEffect } from 'react'
import { getCurrentWindow, type Window } from '@tauri-apps/api/window'
import { NuphusAvatar, type NuphusAvatarState } from '../../ui/NuphusAvatar'
import { IconButton } from '../../ui/Button'
import { IconMinus, IconSquare, IconX, IconMenu, IconPlus } from '../../ui/Icons'
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
          <IconMinus size={14} strokeWidth={1.5} />
        </IconButton>
        <IconButton variant="win-btn" label={t('titleBar.maximize')} onClick={toggleMaximize}>
          <IconSquare size={14} strokeWidth={1.5} />
        </IconButton>
        <IconButton variant="win-close" label={t('titleBar.close')} onClick={closeWindow}>
          <IconX size={14} strokeWidth={1.5} />
        </IconButton>
      </div>
      {/* 移动端：汉堡菜单 */}
      <div className="title-bar-right title-bar-mobile-controls">
        <IconButton
          variant="win-btn"
          label={t('titleBar.menu')}
          onClick={() => setMenuOpen(v => !v)}
        >
          <IconMenu size={18} />
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
                <IconPlus size={16} />
                <span>{t('titleBar.newChat')}</span>
              </button>
            </div>
          </>
        )}
      </div>
    </header>
  )
}
