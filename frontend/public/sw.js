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
 */
const CACHE = 'nuphus-mobile-v2'
const PRECACHE = [
  './mobile.html',
  './manifest.json',
  './icons/icon-192.png',
  './icons/icon-512.png',
  './icons/icon-maskable-192.png',
  './icons/icon-maskable-512.png',
  './icons/apple-touch-icon-180.png',
]

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
  if (url.pathname.startsWith('/message') ||
      url.pathname.startsWith('/history') ||
      url.pathname.startsWith('/identity') ||
      url.pathname.startsWith('/health') ||
      url.pathname.startsWith('/ws')) {
    return
  }

  // 入口 HTML：network-first（保证发版刷新）
  if (url.pathname.endsWith('/mobile.html') || url.pathname === '/' || url.pathname === '') {
    event.respondWith(
      fetch(req)
        .then(res => {
          const copy = res.clone()
          caches.open(CACHE).then(c => c.put(req, copy))
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