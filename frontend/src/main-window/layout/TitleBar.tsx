import { useState, useEffect } from 'react'
import { getCurrentWindow, type Window } from '@tauri-apps/api/window'
import { getVersion } from '@tauri-apps/api/app'
import { NuphusAvatar, type NuphusAvatarState } from '../../ui/NuphusAvatar'
import { IconButton } from '../../ui/Button'
import { IconMinus, IconSquare, IconX, IconMenu, IconPlus } from '../../ui/Icons'
import { useLanguage } from '../../locales'

interface TitleBarProps {
  onNewChat?: () => void
  agentState?: NuphusAvatarState
  /**
   * 品牌 logo 弹层的「版本（检查更新）」入口：复用既有更新弹窗（App 的 setShowUpdate）
   */
  onOpenUpdate?: () => void
}

export function TitleBar({ onNewChat, agentState = 'idle', onOpenUpdate }: TitleBarProps) {
  const { t } = useLanguage()
  const [menuOpen, setMenuOpen] = useState(false)
  const [aboutOpen, setAboutOpen] = useState(false)
  const [win, setWin] = useState<Window | null>(null)
  const [version, setVersion] = useState('')
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

  // 品牌弹层「当前版本（vX.Y.Z）」：挂载时读取一次；非 Tauri 或读取失败时
  // 回退原文案（版本（检查更新）），不显示空括号。
  useEffect(() => {
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
    if (!isTauri) return
    void getVersion()
      .then(setVersion)
      .catch(() => {})
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
        {/* 品牌文字 logo 承接原「应用」入口：hover 弹出 反馈（GitHub 主页）/
            版本（检查更新）。触发器不做拖动区（交互元素冲突） */}
        <div
          className="title-bar-about"
          onMouseEnter={() => setAboutOpen(true)}
          onMouseLeave={() => setAboutOpen(false)}
        >
          <span
            className="title-bar-brand title-bar-about-trigger"
            role="button"
            tabIndex={0}
            aria-haspopup="true"
            aria-expanded={aboutOpen}
            onClick={() => setAboutOpen(v => !v)}
            onKeyDown={e => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                setAboutOpen(v => !v)
              }
            }}
          >
            Nuphus
          </span>
          {aboutOpen && (
            <div className="title-bar-about-menu">
              <a
                className="title-bar-about-item"
                href="https://www.nuphus.com"
                target="_blank"
                rel="noreferrer"
              >
                {t('titleBar.website')}
              </a>
              <a
                className="title-bar-about-item"
                href="https://github.com/mrpulor-gh/nuphus"
                target="_blank"
                rel="noreferrer"
              >
                {t('titleBar.feedback')}
              </a>
              <button
                type="button"
                className="title-bar-about-item"
                disabled={!onOpenUpdate}
                onClick={() => {
                  setAboutOpen(false)
                  onOpenUpdate?.()
                }}
              >
                {version ? t('titleBar.currentVersion', version) : t('titleBar.checkUpdate')}
              </button>
            </div>
          )}
        </div>
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
