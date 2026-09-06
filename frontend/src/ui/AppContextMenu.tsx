import { useCallback, useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { useLanguage } from '../locales'
import '../styles/context-menu.css'

/**
 * 自绘右键菜单（复制能力专用）。
 *
 * 背景：桌面 WebView2 的原生右键菜单附带浏览器导航项（刷新/后退/检查元素），
 * 误点刷新会重载 SPA 中断运行中任务。因此非输入区一律拦截原生菜单，
 * 仅当目标可复制时弹出自绘「复制」菜单 —— 干净、无浏览器功能泄漏。
 *
 * 行为：
 * - 文本输入框/可编辑区 → 放行系统原生菜单（粘贴/剪切/全选依赖它，无导航项）
 * - 其余区域（含消息正文/代码块/终端输出）→ 拦截原生；有选区复制选区，
 *   无选区复制命中的最近可复制块（消息正文 / pre / code / 终端输出等）全文
 */
export default function AppContextMenu() {
  const { t } = useLanguage()
  const [menu, setMenu] = useState<{ x: number; y: number; text: string } | null>(null)
  const [copied, setCopied] = useState(false)

  const close = useCallback(() => {
    setMenu(null)
    setCopied(false)
  }, [])

  useEffect(() => {
    const onCtx = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null
      // 可编辑区放行原生菜单（浏览器导航项不会出现在输入框文本菜单里）
      if (el && el.closest('textarea, input, [contenteditable="true"], [contenteditable=""]')) {
        return
      }
      e.preventDefault() // 其余区域一律拦截浏览器原生菜单

      // 1) 有选区 → 复制选中文字
      const sel = window.getSelection()
      const selectionText = sel && sel.toString().trim() ? sel.toString() : ''
      // 2) 无选区 → 复制命中的最近可复制块全文
      let blockText = ''
      if (!selectionText && el) {
        const block = el.closest(
          '.message-content, pre, code, .term-out, .term-diff-block, blockquote, td, th, li, p, .markdown-body, [data-copyable]',
        ) as HTMLElement | null
        if (block) {
          blockText = (block.innerText || '').trim()
        }
        if (!blockText && el.childElementCount === 0) {
          blockText = (el.textContent || '').trim()
        }
      }
      const text = selectionText || blockText
      if (!text) return // 无可复制内容：右键无动作（原生已拦截）

      // 边界防溢出（菜单约 180×84）
      const menuW = 180
      const menuH = 84
      const x = Math.min(e.clientX, window.innerWidth - menuW - 8)
      const y = Math.min(e.clientY, window.innerHeight - menuH - 8)
      setMenu({ x: Math.max(4, x), y: Math.max(4, y), text })
    }
    const onDown = (e: MouseEvent) => {
      if ((e.target as HTMLElement | null)?.closest?.('.ctx-menu')) return
      close()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    const onScroll = () => close()

    document.addEventListener('contextmenu', onCtx)
    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('keydown', onKey)
    window.addEventListener('scroll', onScroll, true)
    return () => {
      document.removeEventListener('contextmenu', onCtx)
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('scroll', onScroll, true)
    }
  }, [close])

  const doCopy = async () => {
    if (!menu) return
    try {
      await navigator.clipboard.writeText(menu.text)
    } catch {
      // fallback: 隐藏 textarea + execCommand（与 ChatPanel.handleCopy 一致）
      const ta = document.createElement('textarea')
      ta.value = menu.text
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
    }
    setCopied(true)
    window.setTimeout(close, 900)
  }

  /** 选中/右键文本加入对话输入框（复用 nuphus:append-to-chat 通道，用户可编辑后发送） */
  const doAskNuphus = () => {
    if (!menu) return
    window.dispatchEvent(new CustomEvent('nuphus:append-to-chat', { detail: { text: menu.text } }))
    close()
  }

  if (!menu) return null
  return createPortal(
    <div
      className="ctx-menu"
      style={{ left: menu.x, top: menu.y }}
      onContextMenu={e => {
        e.preventDefault()
        e.stopPropagation()
      }}
    >
      <button type="button" className="ctx-menu-item" onClick={doCopy} autoFocus>
        {copied ? t('common.copied') : t('common.copy')}
      </button>
      <div className="ctx-menu-divider" />
      <button type="button" className="ctx-menu-item" onClick={doAskNuphus}>
        {t('common.askNuphus')}
      </button>
      <div className="ctx-menu-preview">
        {menu.text.slice(0, 60)}
        {menu.text.length > 60 ? '…' : ''}
      </div>
    </div>,
    document.body,
  )
}
