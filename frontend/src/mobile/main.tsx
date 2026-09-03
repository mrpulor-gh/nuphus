import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import MobileErrorBoundary from './components/MobileErrorBoundary'
import { initTheme } from './theme'
import { initFontSize } from './fontsize'
import './mobile.css'

// 渲染前应用主题与文字大小（localStorage 记忆，默认深色/标准），避免首屏闪烁
initTheme()
initFontSize()

// 挂载标记：mobile.html 的加载兜底据此判断主入口是否成功执行（8s 未挂载则显示提示）
// @ts-expect-error 全局标记（mobile.html 内联脚本读写）
window.__nuphusMounted = true

// 最外层渲染错误兜底：App 内部 MobileErrorBoundary 只包了 ChatScreen——
// boot/guide 阶段（loading 判定、配对引导）无边界，任何渲染/副作用异常都会让
// React 整树卸载 → 手机永久白屏且无出口。外层兜底把全部阶段纳入，
// 异常时显示可恢复错误卡（重试=整页刷新），白屏变可恢复提示。
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <MobileErrorBoundary>
      <App />
    </MobileErrorBoundary>
  </React.StrictMode>,
)
