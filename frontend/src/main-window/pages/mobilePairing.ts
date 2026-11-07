/** 为手机配对入口附加一次性 mobile token，同时保留服务端路由所需的 query。 */
export function withPairToken(base: string, token: string): string {
  const url = new URL(base)
  url.searchParams.set('token', token)
  return url.toString()
}
