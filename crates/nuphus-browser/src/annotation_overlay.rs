//! Annotation overlay script, shared by every browser surface.
//!
//! **Source of truth**: `frontend/tools/overlay-script.src.js`. The file you
//! are reading is generated — run `node frontend/tools/sync-overlay-script.mjs`
//! to refresh it, and `--check` (CI) to verify it has not drifted.
//!
//! ## Why it lives in this crate
//!
//! The script belongs to the lowest layer so the Tauri shell can reach it:
//! `preview_protocol.rs` refers to it through this crate, not the other way
//! around (a copy living in the shell would force this crate to depend on the
//! shell — backwards, and impossible).
//!
//! ## Host detection
//!
//! The script probes its host at runtime:
//!
//! - **iframe host** (`preview://`): the app window is `window.parent`, so
//!   snapshots go up via `postMessage` and commands arrive the same way. This is
//!   the only wired-up consumer.
//! - **top-level host**: the page has no parent window, so `postMessage` would
//!   only echo back to itself; snapshots are parked on
//!   `window.__NUPHUS_ANNOTATIONS__` instead. The branch is kept in the script
//!   but has **no consumer** — CDP-side annotation was removed (2026-10-08).
//!
//! Either way the overlay is **inert until told otherwise** — every document
//! listener returns on its first line while the annotating flag is false, so a
//! page nobody is marking up behaves exactly as if this script were absent.

pub const ANNOTATION_OVERLAY_SCRIPT: &str = r####"<script>
(function () {
  'use strict'

  // ── 时序闸门：必须等 DOM 就绪 ──
  //
  // 为什么需要：注入分两条通道。
  // - preview:// iframe：宿主在文档解析后 eval，时序正常。
  // - CDP：`Page.addScriptToEvaluateOnNewDocument` 在 document-start 执行，
  //   此时 document.body / document.head 都还不存在。脚本里
  //   `document.body.appendChild(host)` 与 `document.head.appendChild(style)`
  //   会直接抛错，而整段包在 try 里 → 静默失败 → 标注器根本没起来，
  //   页面看起来毫无变化（现场踩过）。
  //
  // 因此把「登记排队」与「真正启动」分开：document-start 阶段只挂一个
  // once 监听，等 DOMContentLoaded（已就绪则直接跑）再执行真正的初始化。
  // 这样同一条注册在两种宿主下都成立，且跨导航自动重跑。
  function boot() {
  var LOG = '[nuphus-annotator]'
  try {
    if (window.__nuphusAnnotatorLoaded) return
    window.__nuphusAnnotatorLoaded = true

    // ── 宿主传输层（同一份脚本服务两种宿主）──
    // iframe 宿主（preview:// 沙箱）：父窗口在另一个 window，postMessage 可用。
    // CDP 宿主（真实远程页）：页面由 Chrome 加载，window.parent === window，没有
    //   可通信的父窗口，postMessage 只会把消息发回自己。因此改为挂全局变量，
    //   由 Rust 侧 evaluate 读取；下行指令则由 Rust evaluate 直接调用导出的函数。
    // 探测方式：脚本自己运行时 window.parent === window 即 CDP 宿主。
    var CDP_HOST = window.parent === window
    var TRANSPORT_WINDOW = '__NUPHUS_ANNOTATIONS__'

    function emitUp(payload) {
      if (CDP_HOST) {
        // 附带 URL 与时间戳：CDP 下没有父窗口记录上下文，采集端需要自带。
        try {
          window[TRANSPORT_WINDOW] = {
            type: payload.type,
            file: payload.file,
            url: location.href,
            at: Date.now(),
            annotations: payload.annotations
          }
        } catch (err) {
          console.error(LOG + ' CDP 上行失败', err)
        }
        return
      }
      try {
        window.parent.postMessage(payload, '*')
      } catch (err) {
        console.error(LOG + ' 上行失败', err)
      }
    }

    /** CDP 宿主下读取当前标注（供 Rust evaluate 调用）。 */
    window.__nuphusReadAnnotations = function () {
      return window[TRANSPORT_WINDOW] || null
    }

    // ── 状态 ──
    // annotating 由父窗口下发。false 时所有 document 监听器第一行即返回，
    // iframe 对页面而言完全不存在：链接可点、按钮可点、什么都不拦。
    var annotating = false
    var markers = []
    // hover 高亮：宿主元素仅作引用缓存（比对用），不写任何宿主属性
    var highlighted = null
    var dragStart = null
    var suppressClick = false
    var flashTimer = null

    // ── shadow DOM 挂载 ──
    // 宿主页面样式污染不到 overlay，overlay 样式也不泄漏进宿主。
    // 宿主根 pointer-events:none —— overlay 只负责视觉，永不吃事件，
    // 非编辑态下对页面零干扰。
    var host = document.createElement('div')
    host.style.cssText =
      'position:fixed;top:0;left:0;width:0;height:0;z-index:2147483647;pointer-events:none'
    var shadow = host.attachShadow({ mode: 'open' })
    shadow.innerHTML = `
      <style>
              :host { all: initial; }
              /* ── 配色令牌（由父窗口握手时下发覆盖，见 applyTheme）──
                 脚本无法读取父窗口 CSS 变量（preview:// 沙箱与主窗口不同源），
                 故走 postMessage 下发；下列为兜底默认值，保证主题指令到达前也能看见。 */
              :host {
                --pv-annot-accent: #3b82f6;
                --pv-annot-on-accent: #ffffff;
                --pv-annot-region: #f59e0b;
                --pv-annot-shadow: rgba(0, 0, 0, 0.45);
              }
              .markers { position: fixed; inset: 0; pointer-events: none; }
              .marker { position: fixed; display: inline-flex; align-items: center; justify-content: center; width: 20px; height: 20px; border-radius: 50%; background: var(--pv-annot-accent); color: var(--pv-annot-on-accent); font: 700 11px/1 system-ui, sans-serif; box-shadow: 0 1px 5px var(--pv-annot-shadow); pointer-events: none; }
              .regionbox { position: fixed; border: 2px dashed var(--pv-annot-region); background: color-mix(in srgb, var(--pv-annot-region) 8%, transparent); pointer-events: none; }
              .regionbox-idx { position: absolute; top: -10px; left: -10px; display: inline-flex; align-items: center; justify-content: center; width: 20px; height: 20px; border-radius: 50%; background: var(--pv-annot-region); color: var(--pv-annot-on-accent); font: 700 11px/1 system-ui, sans-serif; }
              .highlightrect { position: fixed; display: none; border: 2px solid var(--pv-annot-accent); background: color-mix(in srgb, var(--pv-annot-accent) 12%, transparent); border-radius: 2px; pointer-events: none; }
              .highlightrect.show { display: block; }
              .band { position: fixed; display: none; border: 1px dashed var(--pv-annot-region); background: color-mix(in srgb, var(--pv-annot-region) 12%, transparent); pointer-events: none; }
              .band.show { display: block; }
              @keyframes nuphusFlash { 0%, 49% { opacity: 1; } 50%, 100% { opacity: .2; } }
              .flash { animation: nuphusFlash .18s steps(1, end) 6; }
            </style>
            <div class="markers"></div>
      <div class="band"></div>
      <div class="highlightrect"></div>
    `

    var markerLayer = shadow.querySelector('.markers')
    var bandEl = shadow.querySelector('.band')
    var highlightEl = shadow.querySelector('.highlightrect')

    // 事件是否源自 overlay 自身（shadow 宿主路径上的节点都算）
    function inOverlay(event) {
      if (typeof event.composedPath !== 'function') return false
      var path = event.composedPath()
      for (var i = 0; i < path.length; i += 1) {
        if (path[i] === host) return true
      }
      return false
    }

    /** hover 高亮：矩形覆盖到元素视口位置。对宿主只读 rect，一个属性都不写。 */
    function showHighlight(el) {
      if (!el || el.nodeType !== 1) return
      var r = el.getBoundingClientRect()
      if (r.width <= 0 && r.height <= 0) return
      highlightEl.style.left = r.left + 'px'
      highlightEl.style.top = r.top + 'px'
      highlightEl.style.width = r.width + 'px'
      highlightEl.style.height = r.height + 'px'
      highlightEl.classList.add('show')
    }

    function clearHighlight() {
      highlighted = null
      highlightEl.classList.remove('show')
    }

    function setHighlight(el) {
      if (highlighted === el) return
      clearHighlight()
      highlighted = el
      showHighlight(el)
    }

    /** 取消拖拽中的橡皮筋（Esc / 退出编辑态时调用）。 */
    function cancelDrag() {
      dragStart = null
      bandEl.classList.remove('show')
    }

    /**
     * 编辑态开关（父窗口驱动）。off = 清掉所有 overlay 视觉但**保留 markers**：
     * 标注数据的持久化与发送/放弃裁决都是父窗口职责，iframe 不碰 localStorage，
     * 也不在退出时丢数据（重进编辑态直接重建视觉）。
     */
    function setMode(on) {
          annotating = on === true
          clearHighlight()
          cancelDrag()
          if (annotating) renderOverlays()
          else clearOverlayLayer()
        }

        // ── 滚动/视口变化时重排 overlay（角标必须贴住元素，不能像 fixed 一样悬着）──
        // capture=true：页面内部容器的滚动（不冒泡到 window）也能捕获到，
        // 否则只有 window 自身滚动才重排，滚动长页面里的卡片会脱框。
        window.addEventListener(
          'scroll',
          function () {
            if (annotating) relayoutOverlays()
          },
          true,
        )
        window.addEventListener('resize', function () {
          if (annotating) relayoutOverlays()
        })

    // ── 视觉浮层（唯一保留在 iframe 内的 UI 元素）──
    /** 角标贴到元素左上角（视口坐标；rect 只读，不写宿主）。 */
        function positionMarker(marker) {
          if (!marker || !marker.el) return
          var r = marker.el.getBoundingClientRect()
          if (marker.badge) {
            marker.badge.style.left = Math.max(0, r.left) + 'px'
            marker.badge.style.top = Math.max(0, r.top) + 'px'
          }
          // 区域框：锚在元素上（而非记死的 region 坐标），元素随滚动移动时框跟着走——
          // 否则页面一滚，框与元素分离（2026-10-08 大王报障「感觉是fixed 没固定到内容上」）。
          if (marker.box && marker.region) {
            var size = marker.region
            marker.box.style.left = Math.max(0, r.left + (size.offsetX || 0)) + 'px'
            marker.box.style.top = Math.max(0, r.top + (size.offsetY || 0)) + 'px'
          }
        }

        /**
         * 重排所有 overlay 视觉（滚动 / 视口变化时调用）。
         *
         * 角标与区域框都是 position:fixed 的一次性摆位，页面滚动后会与目标元素分离。
         * 这里在 scroll（捕获阶段，捕获内部容器的滚动）与 resize 时重算所有已挂标记的坐标，
         * 让角标「贴」在元素上。元素已被移除（getBoundingClientRect 尺寸为 0）时跳过。
         */
        function relayoutOverlays() {
          if (!annotating || !markers.length) return
          for (var i = 0; i < markers.length; i += 1) {
            var m = markers[i]
            if (m.el && m.el.isConnected) positionMarker(m)
          }
        }

    function placeMarker(el) {
      var badge = document.createElement('div')
      badge.className = 'marker'
      badge.textContent = String(markers.length + 1)
      markerLayer.appendChild(badge)
      markers.push({ el: el, region: null, comment: '', badge: badge, box: null })
      positionMarker(markers[markers.length - 1])
      afterMarkersChange()
    }

    /** 区域标注：记录 region（视口坐标），中心点命中元素作 selector 锚点。 */
    function placeRegion(el, region) {
          var box = document.createElement('div')
          box.className = 'regionbox'
          box.style.left = region.x + 'px'
          box.style.top = region.y + 'px'
          box.style.width = region.w + 'px'
          box.style.height = region.h + 'px'
          var idx = document.createElement('span')
          idx.className = 'regionbox-idx'
          idx.textContent = String(markers.length + 1)
          box.appendChild(idx)
          markerLayer.appendChild(box)
          // 记下「区域左上角相对锚点元素的偏移」：滚动重排时用锚点实时位置 + 该偏移
          // 重新定位，区域框就跟着元素走（而不是记死的视口坐标）。
          var anchorRect = el.getBoundingClientRect()
          markers.push({
            el: el,
            region: {
              x: region.x, y: region.y, w: region.w, h: region.h,
              offsetX: region.x - anchorRect.left,
              offsetY: region.y - anchorRect.top,
            },
            comment: '', badge: null, box: box,
          })
          afterMarkersChange()
        }

    /** 全量重建 overlay 视觉（进入编辑态 / 恢复历史标注后）：只动 DOM，不动数据。 */
    function renderOverlays() {
      clearOverlayLayer()
      markers.forEach(function (m, i) {
        var label = String(i + 1)
        if (m.region) {
          var box = document.createElement('div')
          box.className = 'regionbox'
          box.style.left = m.region.x + 'px'
          box.style.top = m.region.y + 'px'
          box.style.width = m.region.w + 'px'
          box.style.height = m.region.h + 'px'
          var idx = document.createElement('span')
          idx.className = 'regionbox-idx'
          idx.textContent = label
          box.appendChild(idx)
          m.box = box
          markerLayer.appendChild(box)
        } else {
          var badge = document.createElement('div')
          badge.className = 'marker'
          badge.textContent = label
          m.badge = badge
          markerLayer.appendChild(badge)
          positionMarker(m)
        }
      })
    }

    /** 清空 overlay 视觉层（退出编辑态）。markers 数据保留，重进可重建。 */
    function clearOverlayLayer() {
      markerLayer.innerHTML = ''
      markers.forEach(function (m) {
        m.badge = null
        m.box = null
      })
      cancelDrag()
    }

    // ── markers 数据操作 ──
    /** 删除单条：移除视觉 + 重排序号 + 快照同步父窗口。 */
    function removeMarker(marker) {
      var i = markers.indexOf(marker)
      if (i < 0) return
      if (marker.badge) marker.badge.remove()
      if (marker.box) marker.box.remove()
      if (highlighted === marker.el) clearHighlight()
      markers.splice(i, 1)
      renumber()
      afterMarkersChange()
    }

    /** 按下标删除（父窗口列表项 ×）：越界与无效下标一律忽略。 */
    function removeMarkerAt(index) {
      if (typeof index !== 'number' || index < 0 || index >= markers.length) return
      removeMarker(markers[index])
    }

    /** 批注回写（父窗口列表项输入框）：同值直接返回，避免输入每个字符都广播。 */
    function setComment(index, comment) {
      if (typeof index !== 'number' || index < 0 || index >= markers.length) return
      var text = typeof comment === 'string' ? comment : ''
      if (markers[index].comment === text) return
      markers[index].comment = text
      afterMarkersChange()
    }

    /**
     * 定位闪烁：给该条的 overlay 视觉加 .flash（CSS 关键帧驱动），结束移除——
     * 不写行内 style，更不碰宿主元素。
     */
    function flash(marker) {
      if (!marker) return
      var target = marker.box || marker.badge
      if (!target) return
      if (flashTimer) window.clearTimeout(flashTimer)
      target.classList.remove('flash')
      // 强制重排：保证连续两次定位同一元素时动画能重启
      void target.offsetWidth
      target.classList.add('flash')
      flashTimer = window.setTimeout(function () {
        target.classList.remove('flash')
        flashTimer = null
      }, 1100)
    }

    /** 按 selector 反查已标记元素并闪烁（父窗口列表项 ◎ 定位）。 */
    function focusMarker(selector) {
      if (typeof selector !== 'string' || !selector) return
      for (var i = 0; i < markers.length; i += 1) {
        var found = false
        try {
          found = buildSelector(markers[i].el) === selector
        } catch (err) {
          console.error(LOG + ' 定位标注失败', selector, err)
          return
        }
        if (found) {
          flash(markers[i])
          return
        }
      }
      console.error(LOG + ' 定位标注未命中', selector)
    }

    /** 删中间项后序号重排（badge / regionbox-idx 统一刷新）。 */
    function renumber() {
      markers.forEach(function (m, i) {
        var label = String(i + 1)
        if (m.badge) m.badge.textContent = label
        if (m.box) m.box.firstChild.textContent = label
      })
    }

    /** markers 变更统一收口：重排 + 父窗口快照（持久化唯一数据源）。 */
        function afterMarkersChange() {
          renumber()
          postSnapshot()
        }

        /**
         * 清空全部标注（父窗口「放弃」裁决，2026-10-08 大王报障修复）。
         *
         * 此前 bug：父窗口点「放弃」只清了 localStorage + React state，iframe 内的
         * markers 数据仍在 → 再次进入编辑态时 renderOverlays() 把旧标记全画回来，
         * 用户看到「放弃后标注还在」。数据真正持有方是 iframe，所以清空必须下发到这里。
         *
         * 与「退出编辑态」严格区分：setMode(false) 只清视觉（保留数据，便于误退出恢复），
         * 本函数清数据（不可撤销的裁决）。清完发一次空快照，父窗口据此同步 UI。
         */
        function clearAllMarkers() {
          clearOverlayLayer()
          clearHighlight()
          markers = []
          editing = null
          pendingRegion = null
          afterMarkersChange()
        }

    /** 当前 markers → annotations 载荷数组（P1 字段零改；区域条附加 region）。 */
    function buildAnnotations() {
      return markers.map(function (marker) {
        var r = marker.el.getBoundingClientRect()
        var item = {
          css_selector: buildSelector(marker.el),
          outer_html_snippet: snippet(marker.el),
          rect: { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) },
          dpr: window.devicePixelRatio || 1,
          comment: marker.comment || ''
        }
        if (marker.region) item.region = { x: marker.region.x, y: marker.region.y, w: marker.region.w, h: marker.region.h }
        return item
      })
    }

    /**
     * 快照：markers 每次变更都向父窗口广播当前载荷（nuphus:annotations 同型）。
     * 父窗口（PreviewOverlay）据此实时刷新右侧列表并写 localStorage；「发送」只是
     * 这条快照的显式确认 + 清空，不再单独造发送事件（协议面不扩大）。
     */
    function postSnapshot() {
      emitUp({
        type: 'nuphus:annotations',
        file: currentFile(),
        annotations: buildAnnotations()
      })
    }

    /**
     * 持久化恢复：父窗口重开同文件预览时下发历史标注（selector 回定位 DOM）。
     * 定位失败的条目跳过并 console.error，不影响其余。
     */
    function restoreMarkers(list) {
      if (!Array.isArray(list)) return
      list.forEach(function (a) {
        if (!a || typeof a.css_selector !== 'string') return
        var el = null
        try {
          el = document.querySelector(a.css_selector)
        } catch (err) {
          console.error(LOG + ' 恢复标注选择器无效', a.css_selector, err)
          return
        }
        if (!el) {
          console.error(LOG + ' 恢复标注未命中元素', a.css_selector)
          return
        }
        if (a.region && typeof a.region.x === 'number') {
          placeRegion(el, { x: a.region.x, y: a.region.y, w: a.region.w, h: a.region.h })
        } else {
          placeMarker(el)
        }
        // 批注补写（placeMarker/placeRegion 不触发快照外的二次变更）
        markers[markers.length - 1].comment = typeof a.comment === 'string' ? a.comment : ''
      })
      renderOverlays()
      postSnapshot()
    }

    // ── 编辑态事件（仅 annotating 为真时生效；非编辑态 iframe 完全 inert）──
    document.addEventListener('mousedown', function (event) {
      if (!annotating || inOverlay(event) || event.button !== 0) return
      dragStart = { x: event.clientX, y: event.clientY }
    }, true)

    document.addEventListener('mousemove', function (event) {
      if (!annotating) return
      if (dragStart) {
        // 拖拽中：画橡皮筋，暂停元素高亮（松手再按阈值判定点击/区域）
        var x = Math.min(dragStart.x, event.clientX)
        var y = Math.min(dragStart.y, event.clientY)
        var w = Math.abs(event.clientX - dragStart.x)
        var h = Math.abs(event.clientY - dragStart.y)
        bandEl.style.left = x + 'px'
        bandEl.style.top = y + 'px'
        bandEl.style.width = w + 'px'
        bandEl.style.height = h + 'px'
        bandEl.classList.add('show')
        clearHighlight()
        return
      }
      var el = event.target
      if (!el || el.nodeType !== 1) return
      setHighlight(el)
    }, true)

    document.addEventListener('mouseup', function (event) {
      if (!dragStart) return
      var x0 = dragStart.x
      var y0 = dragStart.y
      cancelDrag()
      if (!annotating || inOverlay(event)) return
      var dx = event.clientX - x0
      var dy = event.clientY - y0
      // 位移不足阈值 → 视作点击（交给 click handler 放置 marker）；成型拖拽才产区域
      if (Math.abs(dx) < 5 && Math.abs(dy) < 5) return
      suppressClick = true
      var region = {
        x: Math.round(Math.min(x0, event.clientX)),
        y: Math.round(Math.min(y0, event.clientY)),
        w: Math.round(Math.abs(dx)),
        h: Math.round(Math.abs(dy)),
      }
      // 中心点命中元素作 selector 锚点（overlay 自身元素不算——band/marker 均
      // pointer-events:none，elementFromPoint 正常穿透，这里只兜底过滤）
      var cx = region.x + region.w / 2
      var cy = region.y + region.h / 2
      var el = document.elementFromPoint(cx, cy)
      if (!el || el.nodeType !== 1 || el === document.body || el === document.documentElement) {
        console.error(LOG + ' 区域中心点未命中有效元素', region)
        return
      }
      placeRegion(el, region)
    }, true)

    // capture 阶段拦截：放置标记 / 对已标记元素再次点击则取消该标记；同时吃掉宿主
    // 页面自身的点击响应（标注模式下不应触发页面按钮/跳转）
    document.addEventListener('click', function (event) {
      if (!annotating || inOverlay(event)) return
      event.preventDefault()
      event.stopPropagation()
      // 拖拽已成型的本次点击序列：吞掉（区域已由 mouseup 处理）
      if (suppressClick) {
        suppressClick = false
        return
      }
      var el = event.target
      if (!el || el.nodeType !== 1) return
      for (var i = 0; i < markers.length; i += 1) {
        if (markers[i].el === el) {
          removeMarker(markers[i])
          return
        }
      }
      placeMarker(el)
    }, true)

    // Esc：退格中途 → 取消橡皮筋；否则请求父窗口退出编辑态。
    // 焦点此刻可能在 iframe 内，父窗口收不到 keydown —— 必须由这一侧上报。
    document.addEventListener('keydown', function (event) {
      if (event.key !== 'Escape') return
      if (dragStart) {
        cancelDrag()
        event.stopPropagation()
        return
      }
      if (!annotating) return
      emitUp({ type: 'nuphus:annotate-exit', file: currentFile(), annotations: [] })
    }, true)

    // ── 下行指令 ──
    // iframe 宿主：父窗口 postMessage。source 铁律：只认 window.parent，
    //   其余窗口（含页面自己伪造的）一律忽略。
    // CDP 宿主：没有父窗口，Rust 侧 evaluate 直接调 window.__nuphusAnnotate(cmd)。
    function handleCommand(data) {
      if (!data || typeof data !== 'object') return
      if (data.type === 'nuphus:annotate-mode') {
        setMode(data.on)
        return
      }
      if (data.type === 'nuphus:annotate-focus') {
        focusMarker(data.selector)
        return
      }
      if (data.type === 'nuphus:annotate-comment') {
        setComment(data.index, data.comment)
        return
      }
      if (data.type === 'nuphus:annotate-remove') {
              removeMarkerAt(data.index)
              return
            }
            if (data.type === 'nuphus:annotate-clear-all') {
              clearAllMarkers()
              return
            }
            if (data.type === 'nuphus:annotations-restore') {
              restoreMarkers(data.annotations)
              return
            }
            if (data.type === 'nuphus:annotate-theme') {
              applyTheme(data.tokens)
            }
    }

    if (CDP_HOST) {
      // CDP 宿主：Rust 侧用 evaluate 调用 window.__nuphusAnnotate({type:..., ...})
      window.__nuphusAnnotate = handleCommand
    } else {
      window.addEventListener('message', function (event) {
        if (event.source !== window.parent) return
        handleCommand(event.data)
      })
    }

          /**
           * 主题令牌下发：父窗口读自己的 CSS 变量（--accent / --on-accent / --warning 等）
           * 传进来。脚本读不到父窗口 CSS（preview:// 与主窗口不同源），这是唯一干净的通道——
           * 比在脚本里硬编码色值好：跟主题走、三主题自动适配、脚本与 Nuphus token 零耦合。
           * 只写 shadow 内的自定义属性，宿主 DOM 依旧零写入。
           */
          function applyTheme(tokens) {
            if (!tokens || typeof tokens !== 'object') return
            var map = {
                    '--pv-annot-accent': tokens.accent,
                    '--pv-annot-on-accent': tokens.onAccent,
                    '--pv-annot-region': tokens.warning,
                    '--pv-annot-shadow': tokens.shadow,
                    '--pv-scroll-thumb': tokens.scrollThumb,
                    '--pv-scroll-thumb-hover': tokens.scrollThumbHover,
                  }
                  // 这些变量有两个消费处：shadow 内的 :host（角标/高亮）与宿主 documentElement
                  //（页面滚动条——它在 iframe 文档里，不在 shadow 内），故两处都写。
                  for (var key in map) {
                    var value = map[key]
                    if (typeof value === 'string' && value) {
                      host.style.setProperty(key, value)
                      document.documentElement.style.setProperty(key, value)
                    }
                  }
          }

    function isValidIdent(s) {
      return typeof s === 'string' && /^[A-Za-z_][-\w]*$/.test(s)
    }

    // css_selector：向上遍历生成 tag#id.class:nth-child(n) 链，优先用 id 早停；
    // querySelector 校验不唯一时回退全索引路径，保证能唯一回定位
    function buildSelector(el) {
      var parts = []
      var node = el
      var depth = 0
      while (node && node.nodeType === 1 && depth < 16) {
        depth += 1
        var part = node.tagName.toLowerCase()
        if (node.id && isValidIdent(node.id)) {
          parts.unshift(part + '#' + node.id)
          break
        }
        var classes = []
        if (node.classList && node.classList.length) {
          for (var i = 0; i < node.classList.length && classes.length < 2; i += 1) {
            if (isValidIdent(node.classList[i])) classes.push(node.classList[i])
          }
        }
        if (classes.length) part += '.' + classes.join('.')
        var parent = node.parentElement
        if (!parent) {
          parts.unshift(part)
          break
        }
        if (parent.children.length > 1) {
          part += ':nth-child(' + (Array.prototype.indexOf.call(parent.children, node) + 1) + ')'
        }
        parts.unshift(part)
        node = parent
      }
      var selector = parts.join(' > ')
      try {
        if (document.querySelector(selector) !== el) selector = fallbackSelector(el)
      } catch (err) {
        console.error(LOG + ' 选择器校验失败，回退索引路径', err)
        selector = fallbackSelector(el)
      }
      return selector
    }

    function fallbackSelector(el) {
      var parts = []
      var node = el
      var depth = 0
      while (node && node.nodeType === 1 && depth < 32) {
        depth += 1
        var parent = node.parentElement
        if (!parent) {
          parts.unshift(node.tagName.toLowerCase())
          break
        }
        parts.unshift(node.tagName.toLowerCase() + ':nth-child(' + (Array.prototype.indexOf.call(parent.children, node) + 1) + ')')
        node = parent
      }
      return parts.join(' > ')
    }

    function snippet(el) {
      var html = ''
      try {
        html = el.outerHTML || ''
      } catch (err) {
        console.error(LOG + ' outerHTML 读取失败', err)
      }
      return html.length > 300 ? html.slice(0, 297) + '...' : html
    }

    // file：从 location.pathname 解析文件名，best-effort（失败退 title）
    function currentFile() {
      try {
        var path = String(document.location.pathname || '')
        var name = decodeURIComponent(path.substring(path.lastIndexOf('/') + 1))
        return name || document.title || 'unknown'
      } catch (err) {
        console.error(LOG + ' 解析文件名失败', err)
        return document.title || 'unknown'
      }
    }

    ;(document.body || document.documentElement).appendChild(host)

        // ── 滚动条细化（2026-10-08 大王报障「滚动条太粗」，两次修复）──
        //
        // 第一次只写 `html::-webkit-scrollbar`，示例页滚动容器其实是 body 内的 .wrap，
        // html 自身不滚动 → 规则永不匹配（实机「粗细没变化」）。教训：滚动条样式必须
        // 覆盖**全域滚动容器**，不能假定页面用 html/body 滚动。
        // 现在：全局 * 选择器兜底任何元素滚动容器 + 保留 html 特化，
        // 视觉规格与主窗口一致（components.css: 6px / border-radius 3px / --scrollbar-thumb 色）。
        // 只往宿主 document 注入一条 <style>（样式，不是 DOM 属性污染），不改元素 style/class。
        try {
              var scrollStyle = document.createElement('style')
              scrollStyle.textContent =
                'html{scrollbar-width:thin}' +
                'html::-webkit-scrollbar,body::-webkit-scrollbar,*::-webkit-scrollbar{width:6px;height:6px}' +
                '*::-webkit-scrollbar-track{background:transparent}' +
                '*::-webkit-scrollbar-thumb{background:var(--pv-scroll-thumb, rgba(128,128,128,.45));border-radius:3px}' +
                '*::-webkit-scrollbar-thumb:hover{background:var(--pv-scroll-thumb-hover, rgba(128,128,128,.65))}'
              document.head.appendChild(scrollStyle)
            } catch (err) {
              console.error(LOG + ' 滚动条样式注入失败', err)
            }

        // 握手：iframe 宿主下告知父窗口（PreviewOverlay）标注器已就绪；
        // 2s 未收到即显示「标注器未就绪」。CDP 宿主没有可通信的父窗口，
        // 改为标记全局标志 —— Rust 侧 read_annotations 读到的对象里
        // ready 为 true 即表示初始化完成，不必再等一次协议往返。
        if (CDP_HOST) {
          window.__NUPHUS_ANNOTATIONS__ = { type: 'nuphus:annotator-ready', ready: true, at: Date.now() }
        } else {
          try {
            window.parent.postMessage({ type: 'nuphus:annotator-ready' }, '*')
          } catch (err) {
            console.error(LOG + ' 握手失败', err)
          }
        }
  } catch (err) {
    // 注入异常绝不影响预览本身：仅 console.error，父窗口经握手超时兜底提示
    console.error(LOG + ' 初始化失败', err)
  }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot, { once: true })
  } else {
    boot()
  }
})();</script>"####;
