/**
 * 移动端页面加载失败兜底 + 启动自动选路（外部脚本，兼容 CSP script-src 'self'）。
 *
 * 背景（实测 P0）：中继入口大体积主入口 JS 经隧道传输，桌面↔中继写方向半死时数据停滞，
 * 浏览器加载 script/fetch 无超时 → 页面永久白屏。error 事件只覆盖「失败」，挂起需超时检测。
 * 配合 relay-server TUNNEL_IDLE_TIMEOUT_SECS=30：停滞隧道关闭 → 资源加载报错 → 本文件显示提示。
 *
 * 自动选路（大王 2026-08-16 实测反馈：外出用中继留下 PWA/书签，回家连 WiFi 入口仍是中继
 * origin，启动资源走跨境丢包隧道频繁触发本提示，且「重新加载」只是 location.reload 不重新
 * 选路）。平台依据：mixed content 规范对内网 IP 豁免（实测 Chrome：HTTPS 页 fetch
 * http://192.168.x/health 返回 200），故脚本可后台静默探测 LAN 并自动切换：
 *
 *   中继 origin：启动时**不整页跳转**——t=0 location.replace 到 http 局域网会离开
 *     PWA scope（https r.example.com），浏览器退出 standalone → 界面变有头（iOS 主屏幕
 *     图标启动实测）。启动选路交由 App.tsx：先在中继 https origin 无头加载，加载成功后
 *     页面内探测局域网、switchToLan 切直连（不刷新页面，无头保持）。本脚本仅兜底
 *     资源加载失败/挂起场景（此时页面已不可用，跳直连恢复可用性优先）。
 *   局域网 origin：8s 未挂载/加载失败 → 先探局域网本身：仅慢则 reload 保持直连
 *     （局域网本应秒连，挂起多半是桌面 mobile_server 刚起/短暂卡顿）；真不可达
 *     （已离开 WiFi / 桌面离线）且有缓存 relay_url 才跳中继兜底（无 TRIED_KEY 防回环：
 *     中继 origin 已改页面内切换、不再整页跳回，无回环风险；离开 WiFi 后浏览器刷新
 *     可随时切回中继，不再锁死在局域网地址），无中继缓存才显示提示。
 *
 * 「重新加载」按钮带重新选路：先探 LAN 再决定跳直连还是 reload，不再无脑重走旧路线。
 */
;(function () {
  function readCfg() {
    try {
      return JSON.parse(localStorage.getItem('nuphus_relay_cfg') || 'null') || {}
    } catch (e) {
      return {}
    }
  }
  function readToken() {
    try {
      return localStorage.getItem('nuphus_mobile_token') || ''
    } catch (e) {
      return ''
    }
  }
  function isPrivateHost(h) {
    h = (h || '').toLowerCase()
    return (
      h === 'localhost' ||
      h === '127.0.0.1' ||
      /^10\./.test(h) ||
      /^192\.168\./.test(h) ||
      /^172\.(1[6-9]|2\d|3[01])\./.test(h) ||
      /^169\.254\./.test(h)
    )
  }
  function withToken(base) {
    var t = readToken()
    return base.replace(/\/+$/, '') + '/' + (t ? '?token=' + encodeURIComponent(t) : '')
  }
  /** 探测缓存 LAN 可达性（/health 无鉴权 + CORS 允许，简单 GET 无预检），3s 超时 */
  function probeLan(lanUrl, cb) {
    var ctrl = new AbortController()
    var timer = setTimeout(function () {
      ctrl.abort()
    }, 3000)
    fetch(lanUrl.replace(/\/+$/, '') + '/health', { signal: ctrl.signal, cache: 'no-store' })
      .then(function (res) {
        clearTimeout(timer)
        cb(!!res && res.ok)
      })
      .catch(function () {
        clearTimeout(timer)
        cb(false)
      })
  }

  /** 中继 origin：探 LAN，可达自动跳直连；局域网 origin：先探自身，不可达跳中继。返回是否已接管（导航中） */
  function autoRoute() {
    var cfg = readCfg()
    if (isPrivateHost(location.hostname)) {
      // 局域网 origin：先探局域网本身——只是加载慢则 reload 保持直连；真不可达
      // （已离开 WiFi / 桌面离线）且有缓存中继入口才跳中继兜底。无 TRIED_KEY 防回环：
      // 中继 origin 已不再整页跳回局域网（改由 App.tsx 页面内切换，不刷新），不存在
      // 「局→中→局」整页回环；离开 WiFi 后有头刷新不再锁死在局域网地址，刷新即切中继。
      var lan = cfg.lan_url || location.origin
      probeLan(lan, function (ok) {
        if (ok) {
          location.reload()
        } else {
          var relay = cfg.relay_url
          if (relay) {
            location.replace(withToken(relay))
          } else {
            showFail('局域网不可达且无中继入口，请确认电脑端 Nuphus 已开启「中继转发」')
          }
        }
      })
      return true // 异步探测已接管（由探测回调决定 reload / 跳中继 / 提示）
    }
    if (!cfg.lan_url) return false
    probeLan(cfg.lan_url, function (ok) {
      if (ok) location.replace(withToken(cfg.lan_url))
    })
    return false // 探测异步，不阻塞提示 UI（可达时导航直接发生）
  }

  function showFail(msg) {
    if (document.getElementById('nuphus-load-fail')) return
    var cfg = readCfg()
    var onLan = isPrivateHost(location.hostname)
    var el = document.createElement('div')
    el.id = 'nuphus-load-fail'
    el.style.cssText =
      'position:fixed;inset:0;z-index:99999;display:flex;align-items:center;justify-content:center;background:#0f1115;color:#e6e8ee;font-family:-apple-system,BlinkMacSystemFont,"PingFang SC","Microsoft YaHei",sans-serif;'
    el.innerHTML =
      '<div style="text-align:center;padding:24px;max-width:320px"><div style="font-size:15px;line-height:1.8;margin-bottom:16px">' +
      msg +
      '</div><button id="nuphus-retry-btn" style="background:#f0b429;color:#1a1d24;border:none;border-radius:10px;padding:10px 28px;font-size:14px;font-weight:600">重新加载</button>' +
      (!onLan && cfg.lan_url
        ? '<a href="' +
          withToken(cfg.lan_url) +
          '" style="display:block;margin-top:14px;padding:10px 16px;color:#e6e8ee;border:1px solid #2a2e38;border-radius:10px;font-size:14px;text-decoration:none">同一 WiFi？点这里直连电脑</a>'
        : '') +
      '</div>'
    document.body.appendChild(el)
    document.getElementById('nuphus-retry-btn').onclick = function () {
      // 重新选路而非无脑 reload：中继 origin 先探 LAN（在家→直连秒连），不可达才 reload；
      // 局域网 origin 优先跳中继（在外），无缓存才 reload
      if (onLan) {
        if (!autoRoute()) location.reload()
        return
      }
      var c = readCfg()
      if (c.lan_url) {
        probeLan(c.lan_url, function (ok) {
          if (ok) {
            location.replace(withToken(c.lan_url))
          } else {
            location.reload()
          }
        })
      } else {
        location.reload()
      }
    }
  }

  // ── 启动选路：不再 t=0 整页跳转局域网（会离开 PWA scope → 有头）──
  // 启动后保持当前（中继 https）origin 无头加载，探测与切直连由 App.tsx 页面内完成
  // （switchToLan 只改 apiBase，不刷新页面）。本脚本仅在页面加载失败/挂起（error 事件、
  // 8s 兜底、重新加载按钮）时介入自动选路。

  // 主入口模块加载失败（资源网络错误触发 error 事件，捕获阶段拦截）
  window.addEventListener(
    'error',
    function (e) {
      var src = (e.target && e.target.src) || ''
      if (src.indexOf('/assets/mobile.html-') !== -1) {
        if (!autoRoute()) showFail('网络不稳定，页面资源加载失败')
      }
    },
    true,
  )

  // 挂起兜底：8s 内主入口未挂载（main.tsx 渲染后置 window.__nuphusMounted=true）则先自动
  // 选路、无法自动切换再提示。已显示提示后轮询：模块最终挂载成功则移除提示（慢加载不算失败）。
  setTimeout(function () {
    if (!window.__nuphusMounted && !document.getElementById('nuphus-load-fail')) {
      if (!autoRoute()) showFail('网络不稳定，页面加载较慢')
      var timer = setInterval(function () {
        if (window.__nuphusMounted) {
          var el = document.getElementById('nuphus-load-fail')
          if (el) el.remove()
          clearInterval(timer)
        }
      }, 1000)
    }
  }, 8000)
})()
