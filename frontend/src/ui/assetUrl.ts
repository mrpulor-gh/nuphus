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

import { convertFileSrc } from '@tauri-apps/api/core'

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
