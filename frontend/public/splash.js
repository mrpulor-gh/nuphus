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
    var pctNum = document.getElementById('dlPct')
    if (pctNum) pctNum.textContent = p + '%'
    // 完成态收尾：pct 到 100 说明该阶段已就绪——立即撤下「后台下载」出口。
    // （曾出现形态：真实下载完成后按钮仍一直挂着；10s 定时器只在触发瞬间
    // 检查 lastPct，错过完成时机就没有任何东西再隐藏它。）
    if (p >= 100) {
      if (skipWrap) skipWrap.hidden = true
      if (skipTimer) {
        clearTimeout(skipTimer)
        skipTimer = null
      }
    }
  }

  function setText(text) {
    if (!hint || hint.textContent === text) return
    hint.style.opacity = '0'
    setTimeout(function () {
      hint.textContent = text
      hint.style.opacity = '1'
    }, 120)
  }

  // 下载面板文件名行：从进度文案里提取文件名（如「正在下载视觉模型… x.onnx 45%」）
  function setDlFile(text) {
    var el = document.getElementById('dlFile')
    if (!el) return
    var m = /[…\s]([A-Za-z0-9_.\-]+\.(?:onnx|txt|bin|json))/.exec(text || '')
    el.textContent = m ? m[1] : (text || '').replace(/^正在下载[^…]*…?\s*/, '')
  }

  // 「后台下载」出口仅在真正下载时出现：下载开始 10s 未完成才显示
  // （短下载不闪烁）；非下载阶段隐藏加载条与按钮，splash 回归纯文字状态。
  // 缓存路径下 splash 秒关，定时器无副作用；只有真正卡在下载/启动才触发。
  var SKIP_DELAY_MS = 10000
  var skipTimer = null
  // 最近一次 pct（0..=100）：pct=100（就绪/非下载）时绝不亮出「后台下载」——
  // 双保险（后端已保证已存在文件不发 pct，此处兜底防止任何 pct 事件误亮按钮）。
  var lastPct = -1

  // ── 逻辑起点：先主动查一次「是否真的需要下载」──
  // 不靠「有没有收到进度事件」推断（事件可能因 webview 时序丢失，查询不会）。
  // 全就绪 → 本次会话绝不亮下载面板/按钮；缺模型 → 才允许亮出下载 UI。
  // 查询返回前的保守默认 true（允许显示），失败退化为旧的事件驱动行为。
  var needsDownload = true
  ;(function checkBootstrapStatus() {
    var t = window.__TAURI__
    if (!t || !t.core) return
    t.core
      .invoke('splash_bootstrap_status')
      .then(function (s) {
        needsDownload = !!(s && s.needs_download)
      })
      .catch(function () {})
  })()

  function showBar() {
    if (bar) bar.hidden = false
    document.body.classList.add('downloading')
  }

  function hideBar() {
    if (bar) {
      bar.hidden = true
      setIndeterminate(true) // 复位不定宽动画，供下次下载从头开始
    }
    document.body.classList.remove('downloading')
    var pctNum = document.getElementById('dlPct')
    if (pctNum) pctNum.textContent = ''
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
      // 仅在「确认需要下载」+ 仍在下载（加载条可见）+ 未完成（pct<100）时
      // 才亮出按钮——10s 后状态查询早已返回，needsDownload 是权威判定。
      if (needsDownload && (!bar || !bar.hidden) && lastPct >= 0 && lastPct < 100) {
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
          // 真实下载进行中（下载文案）→ 拉高确认态：覆盖启动后新增的下载
          // （如嵌入模型毒化自愈重下），自愈下载同样要有「后台下载」出口。
          if (/正在下载/.test(d.text || '')) needsDownload = true
          // 只在「已确认需要下载」的会话里亮出下载面板与（延时后）后台下载按钮。
          // 全就绪但收到数值 pct（历史回归形态）→ 不亮面板、不启动倒计时。
          if (needsDownload) {
            showBar()
            ensureSkipTimer()
          }
          setPct(d.pct)
        } else {
          // 纯文字阶段：隐藏下载面板/按钮
          hideBar()
        }
        if (d.text) {
          setText(d.text)
          setDlFile(d.text)
        }
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
