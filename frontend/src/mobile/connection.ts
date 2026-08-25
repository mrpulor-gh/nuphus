/**
 * connection.ts — 连接模式判定（手机端当前访问通道）
 *
 * 模式：
 * - lan：当前页面来自局域网地址（桌面 mobile_server 直连）——免费基础能力
 * - wan：当前页面来自外网地址（中继服务器 / 未来官方服务）——远程访问免费
 *
 * 判定依据：location.hostname 是否为私有网段。私有 = 桌面本机/局域网直连；
 * 非私有 = 经中继服务器访问（手机与桌面不在同一网络）。
 */

export type ConnectionMode = 'lan' | 'wan'

/** 是否为私有/本机网段（局域网直连判定） */
export function isPrivateHost(hostname: string): boolean {
  const h = hostname.toLowerCase()
  if (h === 'localhost' || h === '127.0.0.1' || h === '::1' || h === '[::1]') return true
  // IPv4 私有网段
  if (/^10\./.test(h)) return true
  if (/^192\.168\./.test(h)) return true
  if (/^172\.(1[6-9]|2\d|3[01])\./.test(h)) return true
  // 链路本地
  if (/^169\.254\./.test(h)) return true
  return false
}

/** 局域网直连探测结果：ok=可达 / timeout=超时（真不在同一 WiFi）/ blocked=Mixed Content 拦截 */
export type LanProbeResult = 'ok' | 'timeout' | 'blocked'

/** 是否以 PWA standalone 模式运行（主屏幕图标启动，系统级全屏）。
 *  iOS/Android 均支持 display-mode media query；iOS 另有私有 navigator.standalone 兜底。 */
export function isStandalone(): boolean {
  return (
    window.matchMedia('(display-mode: standalone)').matches ||
    (navigator as unknown as { standalone?: boolean }).standalone === true
  )
}

/**
 * 局域网直连探测（外网/中继页面专用）：
 * fetch 桌面局域网地址 /identity（跨域，依赖 mobile_server 的 CORS 头），3s 超时。
 * - ok（同一 WiFi）→ true：应切回本地直连（免费，不绕中继）
 * - timeout（异地 / 蜂窝）→ 中继通道
 * - blocked：HTTPS 页面（如 https://r.example.com）fetch HTTP 局域网被浏览器 Mixed Content
 *   拦截（iOS Safari 明确拦截；Chrome 允许）——**不代表不可达**，同 WiFi 时应尝试直接导航到
 *   局域网（top-level 导航不受 mixed content 限制）。返回 blocked 由调用方决策。
 */
export async function probeLanDirect(lanUrl: string, token: string): Promise<LanProbeResult> {
  let aborted = false
  try {
    const c = new AbortController()
    const t = setTimeout(() => {
      aborted = true
      c.abort()
    }, 3000)
    // 身份鉴权式探测：带 token 访问 /identity，只有真正的桌面（token 匹配）才返回 200。
    // 避免其他 WiFi 网段冲突时，探测到错误设备误判成「同一局域网」。
    const res = await fetch(`${lanUrl.replace(/\/+$/, '')}/identity`, {
      signal: c.signal,
      cache: 'no-store',
      headers: { 'X-Mobile-Token': token },
    })
    clearTimeout(t)
    return res.ok ? 'ok' : 'timeout'
  } catch {
    // 超时（AbortController 触发）→ 不可达；立即失败（TypeError/Load failed）→ 疑似 Mixed Content
    return aborted ? 'timeout' : 'blocked'
  }
}

/** 中继配置 localStorage 键（局域网配对时缓存，外网页面兜底取桌面局域网地址） */
export const RELAY_STORAGE_KEY = 'nuphus_relay_cfg'

/** 缓存有效期：桌面 IP 可能随 WiFi 重连变化（DHCP），超过 TTL 视为过期，读取时自动丢弃。 */
const RELAY_CACHE_TTL_MS = 24 * 60 * 60 * 1000

interface RelayCache {
  lan_url?: string
  /** 隧道公网入口（非凭据，可缓存）：局域网 origin 页面离开 WiFi 后故障转移到中继用 */
  relay_url?: string
  /** 本机 device_id（hint 下发）：故障转移请求必须携带 ?device= 显式标记——
   *  中继多租户下无标记请求无法确定归属（会被拒绝/引导），实测跨用户串线事故 */
  device_id?: string
  /** 写入时间戳（unix ms）。缺失视为旧格式，读取时按过期处理。 */
  ts?: number
}

function readCache(): RelayCache | null {
  try {
    const raw = localStorage.getItem(RELAY_STORAGE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as RelayCache
    // TTL 过期或旧格式（无 ts）→ 视为无效，立即清除，避免旧 IP 残留
    if (!parsed.ts || Date.now() - parsed.ts > RELAY_CACHE_TTL_MS) {
      localStorage.removeItem(RELAY_STORAGE_KEY)
      return null
    }
    return parsed
  } catch {
    return null
  }
}

/** 写入中继缓存（合并式：保留未提供的字段，刷新时间戳）。绝不缓存 caller_token（审计 P3-5） */
export function saveRelayCache(patch: RelayCache): void {
  try {
    const next = { ...readCache(), ...patch, ts: Date.now() }
    localStorage.setItem(RELAY_STORAGE_KEY, JSON.stringify(next))
  } catch {
    /* ignore */
  }
}

/** 主动清除中继缓存：局域网探测连续失败（桌面 IP 已变）时调用，避免旧地址反复拖慢切换 */
export function clearRelayCache(): void {
  try {
    localStorage.removeItem(RELAY_STORAGE_KEY)
  } catch {
    /* ignore */
  }
}

/** 读缓存的桌面局域网直连地址（无则 null）。外网页面用它探测「是否已回到同一 WiFi」 */
export function getCachedLanUrl(): string | null {
  return readCache()?.lan_url || null
}

/** 读缓存的中继隧道公网入口（无则 null）。局域网 origin 页面故障转移的目标基址 */
export function getCachedRelayUrl(): string | null {
  return readCache()?.relay_url || null
}

/** 读缓存的 device_id（无则 null）：wan 基址拼接 ?device= 用，显式标记本机归属 */
export function getCachedDeviceId(): string | null {
  return readCache()?.device_id || null
}

/** 从当前页面 URL 提取归属设备（?device= / ?device_id=）。
 *  配对期 localStorage 尚无缓存（首次扫码/重置后）——扫码入口本身携带标记，
 *  POST /pair 等请求必须沿用，否则公共中继多设备在线时配对请求被 Ambiguous
 *  拒成引导页，用户怎么重扫都失败（实测死循环）。 */
export function deviceIdFromLocation(): string | null {
  try {
    const q = new URLSearchParams(window.location.search)
    const v = (q.get('device') || q.get('device_id') || '').trim()
    return v || null
  } catch {
    return null
  }
}

/** 从归属前缀路径提取 device：/d/<device_id>/mobile.html（PWA 主屏幕入口 start_url
 *  相对 manifest 解析成 /d/<device_id>/mobile.html，**query 无 device**）→ 从路径段取。
 *  2026-08-26 实测：PWA 打开无 query 归属 → 配对/API 请求无 X-Tunnel-Device 头 →
 *  中继 Ambiguous 或桌面路由错位 → 「网络错误请重试」。 */
export function deviceIdFromPath(): string | null {
  try {
    const m = window.location.pathname.match(/^\/d\/([^/]+)\//)
    if (!m) return null
    const v = decodeURIComponent(m[1]).trim()
    return v || null
  } catch {
    return null
  }
}

/** 归属设备解析顺序：localStorage 缓存（配对成功后 hint 落盘）→ URL query → 归属路径 */
export function resolveTunnelDeviceId(): string | null {
  return getCachedDeviceId() ?? deviceIdFromLocation() ?? deviceIdFromPath()
}

/** 故障转移基址 + 显式设备标记：https://r.example.com → https://r.example.com/?device=<id>。
 *  中继多租户下无标记请求无法确定归属；已有 query/fragment 的 URL 原样返回。 */
export function withDeviceMarker(relayUrl: string, deviceId: string | null): string {
  const base = relayUrl.trim().replace(/\/+$/, '')
  if (!deviceId || base.includes('?') || base.includes('#')) return base
  return `${base}/?device=${deviceId}`
}