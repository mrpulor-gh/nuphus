import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import { initTheme } from './theme'
import { initFontSize } from './fontsize'
import './mobile.css'

// 渲染前应用主题与文字大小（localStorage 记忆，默认深色/标准），避免首屏闪烁
initTheme()
initFontSize()

// 挂载标记：mobile.html 的加载兜底据此判断主入口是否成功执行（8s 未挂载则显示提示）
// @ts-expect-error 全局标记（mobile.html 内联脚本读写）
window.__nuphusMounted = true

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
