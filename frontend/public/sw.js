/* Nuphus 移动端 Service Worker — PWA 桌面快捷方式/离线缓存
 *
 * ⚠️ 生效前提：仅 secure context（HTTPS 或 localhost）可注册。
 * 局域网 HTTP（http://192.168.x.x:18772）下浏览器拒绝注册——不影响「添加到
 * 主屏幕」：iOS Safari 不依赖 SW 即可 standalone 启动；Android Chrome 走
 * manifest + 快捷方式。SW 仅在 HTTPS 部署（或未来加隧道）时自动生效。
 *
 * 缓存策略（保守）：
 * - mobile.html 入口：network-first（保证发版刷新，永不陈旧）
 * - 构建产物 assets/*（带 hash）：cache-first（immutable，长缓存）
 * - manifest/icons：cache-first（静态资源）
 * - 其余（/message /history /ws 等 API）：一律不拦截，直连网络
 *
 * 2026-09-03 加固（外部直连白屏排查）：
 * - CACHE 升 v3：每次发版清空旧 assets 缓存——旧版 hash 资源（含已知 bug 的旧
 *   bundle）不得在离线兜底路径被旧 HTML 重新命中（此前多轮「删 PWA 重加」的
 *   根因之一：旧 HTML + 旧 JS 被 SW 缓存长期保留，新发版后仍跑旧代码/404）。
 * - 导航入口只缓存「真正的应用 HTML」（含 <div id="root">）。中继在隧道离线/
 *   设备未识别时会以 200 返回自己的静态页（设备离线/正在识别/重试页）——
 *   此前 network-first 会把这种非应用页也写进出入口缓存，之后离线兜底就一直
 *   给用户回非应用页（观感「打不开/白屏」）。现改为只缓存应用页，静态页
 *   照常透传（自带 meta refresh / 自动探测，不落缓存），入口缓存恒为最近一次
 *   真正加载成功的应用页。
 */

const CACHE = 'nuphus-mobile-v3'
const PRECACHE = [
  './mobile.html',
  './manifest.json',
  './icons/icon-192.png',
  './icons/icon-512.png',
  './icons/icon-maskable-192.png',
  './icons/icon-maskable-512.png',
  './icons/apple-touch-icon-180.png',
]

/** 是否为 Nuphus 移动端应用入口 HTML（区别于中继静态页：设备离线/识别中/重试页）。
 * 中继静态页同样是 200 HTML，但无 React 挂载点；只有应用页值得写入入口缓存。 */
function isAppEntryHtml(res) {
  if (!res || !res.ok) return false
  const ct = res.headers.get('content-type') || ''
  if (ct.indexOf('text/html') === -1) return false
  return res.text().then(
    body => body.indexOf('<div id="root">') !== -1,
    () => false,
  )
}

self.addEventListener('install', event => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then(cache => cache.addAll(PRECACHE))
      .then(() => self.skipWaiting()),
  )
})

self.addEventListener('activate', event => {
  event.waitUntil(
    caches
      .keys()
      .then(keys => Promise.all(keys.filter(k => k !== CACHE).map(k => caches.delete(k))))
      .then(() => self.clients.claim()),
  )
})

self.addEventListener('fetch', event => {
  const req = event.request
  if (req.method !== 'GET') return
  const url = new URL(req.url)

  // API 请求绝不缓存：直连网络（含 WS 升级请求、/message /history /identity /health）
  if (
    url.pathname.startsWith('/message') ||
    url.pathname.startsWith('/history') ||
    url.pathname.startsWith('/identity') ||
    url.pathname.startsWith('/health') ||
    url.pathname.startsWith('/ws')
  ) {
    return
  }

  // 入口 HTML：network-first（保证发版刷新）
  if (url.pathname.endsWith('/mobile.html') || url.pathname === '/' || url.pathname === '') {
    event.respondWith(
      fetch(req)
        .then(async res => {
          // 只把真正的应用页写进缓存；中继静态页（离线/识别中/重试）不落缓存，
          // 避免离线兜底永远回非应用页（观感白屏/打不开）。
          if (await isAppEntryHtml(res.clone())) {
            const copy = res.clone()
            caches.open(CACHE).then(c => c.put(req, copy))
          }
          return res
        })
        .catch(() => caches.match(req).then(m => m || caches.match('./mobile.html'))),
    )
    return
  }

  // 静态资源：cache-first（assets/* 带 hash 不可变；icons/manifest 不变）
  event.respondWith(
    caches.match(req).then(hit => {
      if (hit) return hit
      return fetch(req)
        .then(res => {
          if (res.ok) {
            const copy = res.clone()
            caches.open(CACHE).then(c => c.put(req, copy))
          }
          return res
        })
        .catch(() => caches.match(req))
    }),
  )
})
