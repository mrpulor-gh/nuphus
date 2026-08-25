// splash.js — 监听后端 `splash:progress` 事件驱动 splash 进度条与状态文案。
//
// 载荷 { pct?: number, text: string }：
//   pct 为 0..=100 时渲染定宽进度；pct 缺失（null）时切到不定宽滑动条。
// 事件走 Tauri IPC（app.emit），不受本页 CSP `script-src 'self'` 限制。
;(function () {
  'use strict'

  var hint = document.querySelector('.hint')
  var bar = document.getElementById('bar')
  var fill = document.getElementById('barFill')
  var skipWrap = document.getElementById('skipWrap')
  var skipBtn = document.getElementById('skipBtn')

  function setIndeterminate(on) {
    bar.classList.toggle('indeterminate', on)
    if (on) fill.style.width = '30%'
  }

  function setPct(pct) {
    setIndeterminate(false)
    var p = Math.max(0, Math.min(100, Math.round(pct)))
    lastPct = p
    fill.style.width = p + '%'
  }

  function setText(text) {
    if (!hint || hint.textContent === text) return
    hint.style.opacity = '0'
    setTimeout(function () {
      hint.textContent = text
      hint.style.opacity = '1'
    }, 120)
  }

  // 「后台下载」出口仅在真正下载时出现：下载开始 10s 未完成才显示
  // （短下载不闪烁）；非下载阶段隐藏加载条与按钮，splash 回归纯文字状态。
  // 缓存路径下 splash 秒关，定时器无副作用；只有真正卡在下载/启动才触发。
  var SKIP_DELAY_MS = 10000
  var skipTimer = null
  // 最近一次 pct（0..=100）：pct=100（就绪/非下载）时绝不亮出「后台下载」——
  // 双保险（后端已保证已存在文件不发 pct，此处兜底防止任何 pct 事件误亮按钮）。
  var lastPct = -1

  function showBar() {
    if (bar) bar.hidden = false
  }

  function hideBar() {
    if (bar) {
      bar.hidden = true
      setIndeterminate(true) // 复位不定宽动画，供下次下载从头开始
    }
    if (skipWrap) skipWrap.hidden = true
    if (skipTimer) {
      clearTimeout(skipTimer)
      skipTimer = null
    }
    lastPct = -1 // 纯文字阶段：复位下载态标记
  }

  function ensureSkipTimer() {
    if (skipTimer || !skipWrap) return
    skipTimer = setTimeout(function () {
      skipTimer = null
      // 仅仍在下载（加载条可见）且未完成（pct<100）时才亮出按钮
      if ((!bar || !bar.hidden) && lastPct >= 0 && lastPct < 100) {
        skipWrap.hidden = false
      }
    }, SKIP_DELAY_MS)
  }

  function start() {
    var tauri = window.__TAURI__
    // withGlobalTauri 未注入（异常环境）：纯文字静态状态，不抛错。
    if (!tauri || !tauri.event) {
      setText('正在启动…')
      return
    }
    tauri.event
      .listen('splash:progress', function (ev) {
        var d = ev.payload || {}
        if (typeof d.pct === 'number') {
          // pct 只由真实下载发出 → 此刻才亮出加载条与（延时后）后台下载按钮
          showBar()
          setPct(d.pct)
          ensureSkipTimer()
        } else {
          // 纯文字阶段：隐藏加载条/按钮
          hideBar()
        }
        if (d.text) setText(d.text)
      })
      .catch(function () {
        // listen 注册失败：静默回退到静态状态
      })

    // 点击「后台下载」→ 后端关闭 splash + 显示主窗口，下载继续（主界面 HUD 提示）
    if (skipBtn) {
      skipBtn.addEventListener('click', function () {
        skipBtn.disabled = true
        if (skipWrap) skipWrap.hidden = true
        setText('正在打开应用…')
        if (tauri.core) {
          tauri.core.invoke('splash_skip_download').catch(function () {
            skipBtn.disabled = false
            if (skipWrap) skipWrap.hidden = false
          })
        }
      })
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start)
  } else {
    start()
  }
})()
