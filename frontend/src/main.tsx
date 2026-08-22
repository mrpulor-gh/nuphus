import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './main-window/App'
import { ThemeProvider } from './hooks/useTheme'
import { LangProvider } from './locales'
import { ErrorBoundary } from './ui/ErrorBoundary'
import './styles/ink.css'
// PDF 渲染服务：注册 window.__nuphusRenderPdf（Rust 侧 eval 触发，扫描件 OCR 兜底用）
import './core/pdf-render'

// Initialize theme before first paint to avoid flash
;(() => {
  const stored = localStorage.getItem('nuphus_theme')
  const prefersLight = window.matchMedia('(prefers-color-scheme: light)').matches
  const theme = stored === 'light' || (!stored && prefersLight) ? 'light' : 'dark'
  document.documentElement.setAttribute('data-theme', theme)
})()

// Disable global context menu
window.addEventListener('contextmenu', e => e.preventDefault())

// Listen for shortcut events intercepted in index.html
window.addEventListener('nuphus-shortcut', (e: Event) => {
  const detail = (e as CustomEvent).detail
  if (detail?.key === 'shift+p') {
    // TODO: Open plugin search panel
  }
})

const root = document.getElementById('root')
if (!root) {
  document.body.innerHTML = '<div style="color:red;padding:20px">FATAL: #root not found</div>'
  throw new Error('#root not found')
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <ThemeProvider>
      <LangProvider>
        <ErrorBoundary onExit={() => window.close()}>
          <App />
        </ErrorBoundary>
      </LangProvider>
    </ThemeProvider>
  </React.StrictMode>,
)
