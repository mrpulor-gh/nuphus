/**
 * 示教录制采集脚本：记录人（不是 Agent）在页面上的真实操作，形成结构化步骤。
 *
 * 与 overlay 的关系：
 * - overlay 负责「意图」（点哪里、改成什么样）；本脚本负责「动作」（做过什么）。
 *   两者互补：Agent 拆解工作流时，需要动作序列来还原流程，需要意图标注来说明
 *   每一步的目标与验收标准。
 * - 共用同一套宿主传输约定（见下），Rust 侧用同一对方法读/下发。
 *
 * 观察而不干预（red line）：
 * - 所有 listener 走 capture 阶段记录，**不调用 preventDefault / stopPropagation**，
 *   不改写任何页面行为——录制期间页面必须与平常完全一致。
 * - 非录制态第一行即返回，对页面零开销、零干扰。
 *
 * 记录字段（每步）：selector 用 nth-child 索引链，与 overlay 的选择器策略一致，
 * 这样标注与录制指向同一元素时能对上。
 */
;(function () {
  'use strict'
  var LOG = '[nuphus-recorder]'
  try {
    if (window.__nuphusRecorderLoaded) return
    window.__nuphusRecorderLoaded = true

    var CDP_HOST = window.parent === window
    var BUFFER = '__NUPHUS_RECORDING__'
    var recording = false

    /** 上报一条步骤。iframe 宿主走 postMessage；CDP 宿主挂全局由 Rust 取回。 */
    function push(step) {
      try {
        if (!CDP_HOST) {
          window.parent.postMessage({ type: 'nuphus:recording-step', step: step }, '*')
          return
        }
        var buf = window[BUFFER] || (window[BUFFER] = [])
        buf.push(step)
        // 只保留最近 500 步，避免长时间录制把页面内存吃满；调用方应按需取走。
        if (buf.length > 500) window[BUFFER] = buf.slice(-500)
      } catch (err) {
        console.error(LOG + ' 步骤上报失败', err)
      }
    }

    /**
     * 生成元素的 CSS 选择器：标签 + #id（若有可用id）+ nth-child 链。
     * 与 overlay 的策略一致，保证标注与录制指向同一元素时可对齐。
     */
    function selectorFor(el) {
      if (!el || el.nodeType !== 1) return ''
      if (el.id && /^[A-Za-z][\w-]*$/.test(el.id)) return '#' + el.id
      var parts = []
      var node = el
      var depth = 0
      while (node && node.nodeType === 1 && depth < 6) {
        var tag = node.tagName.toLowerCase()
        var parent = node.parentElement
        if (parent) {
          var sameTag = []
          for (var i = 0; i < parent.children.length; i++) {
            if (parent.children[i].tagName === node.tagName) sameTag.push(parent.children[i])
          }
          if (sameTag.length > 1) {
            tag += ':nth-of-type(' + (sameTag.indexOf(node) + 1) + ')'
          }
        }
        parts.unshift(tag)
        if (tag.indexOf('#') === 0) break
        node = parent
        depth++
      }
      return parts.join(' > ')
    }

    function describe(el) {
      if (!el || el.nodeType !== 1) return ''
      var text = (el.innerText || el.value || el.getAttribute('aria-label') || '').trim()
      if (text.length > 80) text = text.slice(0, 80) + '…'
      return text
    }

    // ── click：记录元素，不阻止任何默认行为 ──
    document.addEventListener(
      'click',
      function (ev) {
        if (!recording) return
        push({
          action: 'click',
          selector: selectorFor(ev.target),
          text: describe(ev.target),
          url: location.href,
          at: Date.now()
        })
      },
      true
    )

    // ── input：change 而非 input——input 每敲一个键触发一次，噪声太大；
    //    change 在失焦/提交时触发一次，正好是「这一项填完了」的语义。
    document.addEventListener(
      'change',
      function (ev) {
        if (!recording) return
        var el = ev.target
        var type = (el.type || '').toLowerCase()
        if (type === 'checkbox' || type === 'radio') {
          push({
            action: 'check',
            selector: selectorFor(el),
            checked: !!el.checked,
            url: location.href,
            at: Date.now()
          })
          return
        }
        push({
          action: 'fill',
          selector: selectorFor(el),
          value: typeof el.value === 'string' ? el.value : '',
          url: location.href,
          at: Date.now()
        })
      },
      true
    )

    // ── 键盘：只记有意义的组合（Enter/Escape/Tab/功能键），普通字符已由 fill 覆盖 ──
    document.addEventListener(
      'keydown',
      function (ev) {
        if (!recording) return
        var k = ev.key
        if (k === 'Enter' || k === 'Escape' || k === 'Tab' || k === 'Backspace') {
          push({
            action: 'key',
            key: k,
            selector: selectorFor(ev.target),
            url: location.href,
            at: Date.now()
          })
        }
      },
      true
    )

    // ── 提交：表单提交是工作流里重要的语义节点 ──
    document.addEventListener(
      'submit',
      function (ev) {
        if (!recording) return
        push({
          action: 'submit',
          selector: selectorFor(ev.target),
          url: location.href,
          at: Date.now()
        })
      },
      true
    )

    // ── 下行接口（Rust 经 evaluate 调用）──
    window.__nuphusRecording = {
      start: function () {
        window[BUFFER] = []
        recording = true
        return true
      },
      stop: function () {
        recording = false
        return window[BUFFER] || []
      },
      take: function () {
        var buf = window[BUFFER] || []
        window[BUFFER] = []
        return buf
      },
      peek: function () {
        return window[BUFFER] || []
      },
      isRecording: function () {
        return recording
      }
    }
  } catch (err) {
    console.error('[nuphus-recorder] 初始化失败', err)
  }
})()