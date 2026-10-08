// browser-frame 入口 —— 独立浏览器窗口的壳页面（44px 控制器）。
//
// 与 hud / capture_overlay 同为二级入口（vite.config.ts 的 rollup input）。
// 主题走 ThemeProvider（与主窗口共用 localStorage 的 nuphus_theme / 自定义主题，
// 壳页面是同源的另一个 webview，不带 Provider 就永远是 tokens.css 的暗色默认值）；
// 语言走 LangProvider（启动时经后端 getLanguage 校正，见 locales/index.ts）。
import React from 'react'
import ReactDOM from 'react-dom/client'
import { ThemeProvider } from '../hooks/useTheme'
import { LangProvider } from '../locales'
import { BrowserFrame } from './BrowserFrame'
import './browser-frame.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ThemeProvider>
      <LangProvider>
        <BrowserFrame />
      </LangProvider>
    </ThemeProvider>
  </React.StrictMode>,
)
