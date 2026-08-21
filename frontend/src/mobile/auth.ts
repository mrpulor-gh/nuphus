/**
 * 移动端鉴权：token 生命周期管理
 * 动线：URL ?token=xxx → 读取存 localStorage → 地址栏净化（history.replaceState）
 * 后续 WS 用 query、POST/GET 用 Header。token 仅存 localStorage，不明文另存他处。
 */

const TOKEN_KEY = 'nuphus_mobile_token'

/** 启动时解析 token：优先 URL query（配对链接进入），否则读 localStorage */
export function initToken(): string | null {
  const url = new URL(window.location.href)
  const fromUrl = url.searchParams.get('token')
  if (fromUrl) {
    localStorage.setItem(TOKEN_KEY, fromUrl)
    // 地址栏净化：token 不滞留在 URL（防截图泄露 / 历史记录残留）
    url.searchParams.delete('token')
    window.history.replaceState(null, '', url.pathname + url.search + url.hash)
    return fromUrl
  }
  const stored = localStorage.getItem(TOKEN_KEY)
  return stored && stored.length > 0 ? stored : null
}

/** 配对成功后落盘 token（PairingGuide 密码换 token 回传后由 App 调用） */
export function saveToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token.trim())
}

/** token 失效（401）时清除，回到配对引导 */
export function clearToken(): void {
  localStorage.removeItem(TOKEN_KEY)
}