import { isPrivateHost } from './connection'

const SERVER_URL_KEY = 'nuphus_mobile_server_url'

export type ServerUrlError = 'empty' | 'invalid'

export interface PreparedServerNavigation {
  url: string
  changed: boolean
}

/** 手机页面只能导航到 HTTP(S)；把桌面的 ws(s) 地址转换成对应页面协议。 */
function withPageScheme(raw: string): string {
  const value = raw.trim()
  if (/^wss:\/\//i.test(value)) return `https://${value.slice(6)}`
  if (/^ws:\/\//i.test(value)) return `http://${value.slice(5)}`
  if (/^[a-z][a-z\d+.-]*:\/\//i.test(value)) return value

  const withoutSlashes = value.replace(/^\/+/, '')
  const authority = withoutSlashes.split('/')[0]
  let hostname = authority
  try {
    hostname = new URL(`http://${authority}`).hostname
  } catch {
    /* 交给 URL 校验统一报 invalid */
  }
  return `${isPrivateHost(hostname) ? 'http' : 'https'}://${withoutSlashes}`
}

/** 规范化服务端页面地址；token 永不写入持久化配置。 */
export function normalizeMobileServerUrl(raw: string): string {
  if (!raw.trim()) throw new Error('empty')
  let url: URL
  try {
    url = new URL(withPageScheme(raw))
  } catch {
    throw new Error('invalid')
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error('invalid')
  if (url.username || url.password) throw new Error('invalid')
  url.searchParams.delete('token')
  url.hash = ''
  return url.toString()
}

export function getCurrentMobileServerUrl(): string {
  try {
    return normalizeMobileServerUrl(window.location.href)
  } catch {
    return window.location.origin
  }
}

/** 优先返回用户上次保存的服务端；无有效记录时使用当前页面 origin。 */
export function getPreferredMobileServerUrl(): string {
  try {
    const stored = localStorage.getItem(SERVER_URL_KEY)
    if (stored) {
      try {
        return normalizeMobileServerUrl(stored)
      } catch {
        localStorage.removeItem(SERVER_URL_KEY)
      }
    }
  } catch {
    /* 隐私模式/损坏记录时回退当前 origin */
  }
  return getCurrentMobileServerUrl()
}

export function saveMobileServerUrl(url: string): void {
  const normalized = normalizeMobileServerUrl(url)
  try {
    localStorage.setItem(SERVER_URL_KEY, normalized)
  } catch {
    /* 存储不可用不阻断页面导航 */
  }
}

/** 返回规范化目标与是否真的需要导航；页面跳转由 UI 层执行。 */
export function prepareMobileServerNavigation(raw: string): PreparedServerNavigation {
  const url = normalizeMobileServerUrl(raw)
  saveMobileServerUrl(url)
  return { url, changed: url !== getCurrentMobileServerUrl() }
}
