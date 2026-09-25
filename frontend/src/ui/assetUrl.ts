/**
 * toAssetUrl — 文件系统路径 → 浏览器可访问 URL（Tauri asset protocol）
 *
 * 单一实现点：ChatPanel / UserInputPrompt / ThemesPage 原先各写了一份逐字
 * 相同的私有副本，抽到这里共用（三份必然漂移，改一处漏两处）。
 *
 * 语义：
 * - 空值 → null
 * - 已经是 URL 的（http/https/data/asset/tauri）**原样返回**。
 *   这一条同时承担存量兼容：早期皮肤背景/头像存的是 dataURL，
 *   升级到「只存本地路径」后老数据仍能正常渲染，不会因为格式变化而丢图。
 * - 其余视为本地文件系统绝对路径 → convertFileSrc() 转 asset://
 */

import { convertFileSrc, invoke } from '@tauri-apps/api/core'

/**
 * 皮肤背景专用：把本机图片路径解析成**既不进 CSS 值、也不经 asset://** 的 URL。
 *
 * # 为什么不能复用 `resolveLocalImageUrl`（2026-09-26 定稿）
 *
 * 它把整张图读成 base64 dataURL。皮肤背景早先就吃这个亏：图形数据一旦成为
 * CSS 值的一部分，就要撞 Chromium/WebView2 对单个 CSS 值 **2 MiB** 的硬上限
 * （实测 2,097,152 字符通过 / 2,100,000 起静默丢弃），而 base64 相对原文件膨胀
 * 4/3 —— 图片只要 > ~1.5 MB 就必然不显示，且零报错、零提示。
 *
 * 本函数改为按序尝试**两条都不会把字节写进 CSS 的通道**：
 *
 * 1. `convertFileSrc()`（asset://）—— 走 Tauri 自定义协议，活字节由 WebView 自己
 *    流式读取，与图片大小无关。这是首选，也是 user_assets.rs 声明的架构意图。
 *    但它只在 Tauri 的 asset 上下文可用：前端跑在 Vite dev server 时它会**原样
 *    返回传入的裸路径**（2026-09-25 实测教训，见 resolveLocalImageUrl 注释），
 *    所以解出来的 URL 必须**回读校验**才算数，不能只看函数有没有抛异常。
 * 2. 回读失败（dev server 的常态）→ 经 `read_image_base64` 把字节读进来，
 *    用 Blob URL 交给浏览器。blob: 是浏览器侧引用，同样不进 CSS 值。
 *
 * 之所以敢在回退里让字节进内存：这条路径只在 dev server 下走到，且 blob: 的驻留
 * 期完全可控（见 `releaseSkinImageUrl`），不像 dataURL 那样被 CSSOM 永久持有、
 * 每帧参与样式重算。发布版（WebView2）走 asset://，一个字节都不进 JS 堆。
 *
 * # 生命周期（调用方契约）
 * data: / http(s): / blob: 入参原样返回，**不产生**需要释放的资源；
 * 一旦返回值以 `blob:` 开头，调用方必须在该 URL 被替换或组件卸载时把它交给
 * `releaseSkinImageUrl()` —— 否则这张图的字节会随 Blob 活到页面关闭（泄漏）。
 */
export async function resolveSkinImageUrl(path: string | null | undefined): Promise<string | null> {
  if (!path) return null
  if (/^(https?:\/\/|data:|blob:|asset:\/\/|tauri:\/\/)/i.test(path)) return path
  const assetUrl = toAssetUrl(path)
  if (assetUrl && assetUrl !== path && (await isLoadableImage(assetUrl))) return assetUrl
  return readImageAsBlobUrl(path)
}

/**
 * 释放 `resolveSkinImageUrl` 产生的 blob URL。
 *
 * 实现细节：**不做 `blob:` 前缀预判**，直接交给 `URL.revokeObjectURL`。
 * 规范规定它对非 blob URL 是无操作（no-op），而预判会导致真实的泄漏 ——
 * 旧值 `blob:`、新值 `data:`（存量图片）时，预判会按"这次不用释放"跳过，
 * 旧 blob 的字节就活到页面关闭。交给浏览器判定既正确又少一条自己的规则。
 */
export function releaseSkinImageUrl(url: string | null | undefined): void {
  if (!url) return
  URL.revokeObjectURL(url)
}

/**
 * 回读校验：只有图片真的解码成功，URL 才算可渲染。
 *
 * 这是本方案的关键一步 —— `convertFileSrc` 在 dev server 下**静默**返回裸路径
 * （不抛错，见 resolveLocalImageUrl 注释），只看"解出来没有"会把一条
 * `http://localhost:5174/C:\...` 当成有效 URL 交出去，重演 2026-09-25 那次
 * 「CSS 变量看着有值、背景其实从不加载」。所以一律用真实解码结果判定：
 * onload 且 naturalWidth>0 才算过，onerror 一律否。
 *
 * jsdom 不加载图片资源（onload 永不触发），测试可通过 `__setSkinImageProbe`
 * 注入判定函数；生产路径不会走到那个分支。
 */
function isLoadableImage(url: string): Promise<boolean> {
  if (typeof skinImageProbe === 'function') return Promise.resolve(skinImageProbe(url))
  if (typeof Image === 'undefined') return Promise.resolve(true)
  return new Promise<boolean>(resolve => {
    const img = new Image()
    img.onload = () => resolve(img.naturalWidth > 0)
    img.onerror = () => resolve(false)
    img.src = url
  })
}

/**
 * 解码探针替身（仅测试用）。
 *
 * 存在的理由：这条分支的成败取决于 WebView 真实的资源加载，jsdom 里无从复现。
 * 测试若靠"等 onload 超时"来走回退路径，就会变成靠时序碰运气；注入探针让
 * 「asset:// 可用 / 不可用」两种真实环境都能被确定性地钉住。生产代码从不调用它。
 */
let skinImageProbe: ((url: string) => boolean) | null = null

export function __setSkinImageProbe(probe: ((url: string) => boolean) | null): void {
  skinImageProbe = probe
}

/**
 * 兜底通道：Rust 读文件 → Blob URL。
 *
 * 与 `resolveLocalImageUrl` 共用 `read_image_base64`，但**不做 dataURL 拼接** ——
 * 只在 JS 侧完成后与 blob 的转换；mime 同样自推（后端对 gif/webp 一律返回
 * image/png，见 resolveLocalImageUrl 注释），否则动图会被当成静态 png。
 */
async function readImageAsBlobUrl(path: string): Promise<string | null> {
  try {
    const res = await invoke<{ base64?: string; mime?: string }>('read_image_base64', {
      imagePath: path,
    })
    if (!res?.base64) return null
    const binary = atob(res.base64)
    const bytes = new Uint8Array(binary.length)
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
    return URL.createObjectURL(new Blob([bytes], { type: guessImageMime(path) }))
  } catch (e) {
    console.error('[assetUrl] 皮肤背景读取本机图片失败:', path, e)
    return null
  }
}

export function toAssetUrl(path: string | null | undefined): string | null {
  if (!path) return null
  if (/^(https?:\/\/|data:|asset:\/\/|tauri:\/\/)/i.test(path)) return path
  try {
    return convertFileSrc(path)
  } catch {
    return null
  }
}

/** 按扩展名推 MIME（含 gif/webp/svg）。 */
export function guessImageMime(path: string): string {
  const ext = (path.split('.').pop() || '').toLowerCase()
  switch (ext) {
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg'
    case 'gif':
      return 'image/gif'
    case 'webp':
      return 'image/webp'
    case 'bmp':
      return 'image/bmp'
    case 'svg':
      return 'image/svg+xml'
    case 'ico':
      return 'image/x-icon'
    default:
      return 'image/png'
  }
}

/**
 * 把本机图片路径解析成**真正可渲染**的 URL（异步）。
 *
 * # 为什么不能只用 `convertFileSrc`（2026-09-25 实测教训）
 *
 * asset:// 通道只在 Tauri 的 asset 上下文里可用。前端跑在 Vite dev server
 * （`location.href === 'http://localhost:5174/'`）时，`convertFileSrc` 原样返回
 * 传入的路径，于是一条裸 Windows 路径被交给浏览器当 URL 请求
 * `http://localhost:5174/C:\Users\...` → `net::ERR_CONNECTION_REFUSED`，
 * 背景**静默不显示**（CSS 变量看着有值，资源其实从未加载）。
 * 而 dev server 正是日常开发与验证的常态，不是边缘场景。
 *
 * 因此这里**不依赖 asset 协议**：一律走「Rust 读文件 → dataURL」，
 * dev 与打包后行为完全一致。代价是一张图的字节会进内存，
 * 相对"根本显示不出来"这是必要成本（且 Rust 侧已有体积上限兜底）。
 *
 * 入参兼容：已是 URL 的（http/https/data/asset/tauri）原样返回，
 * 升级前存 dataURL 的老数据继续可用。
 */
export async function resolveLocalImageUrl(
  path: string | null | undefined,
): Promise<string | null> {
  if (!path) return null
  if (/^(https?:\/\/|data:|asset:\/\/|tauri:\/\/)/i.test(path)) return path
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    const res = await invoke<{ base64?: string; mime?: string }>('read_image_base64', {
      imagePath: path,
    })
    if (!res?.base64) return null
    // 自己推 mime：read_image_base64 对 gif/webp 一律返回 image/png（dict_ocr.rs:494
    // 的 `_ => "image/png"`），直接用会让动态图与 webp 静默变成 png 而渲染异常。
    return `data:${guessImageMime(path)};base64,${res.base64}`
  } catch (e) {
    console.error('[assetUrl] 读取本机图片失败:', path, e)
    return null
  }
}
