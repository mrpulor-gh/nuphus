// userImage.ts — 用户图片（皮肤背景 / 头像）存储契约
//
// 历史：dataURL 直接存 localStorage。浏览器每源约 5MB 配额，稍大的图在 setItem 时抛
// QuotaExceededError，调用侧无捕获，表现为「选择图片后界面毫无变化」（异常中断了后续
// CSS 变量注入），且同源配额共享，一次大图写入失败会连带影响其它 key 的后续写入。
//
// 现在：图片落盘 {data_dir}/Nuphus/user-images/{kind}/{uuid}.{ext}（save_user_image），
// localStorage 只存 "{kind}/{uuid}.{ext}" 短标识。展示时由 read_user_image 返回磁盘
// 绝对路径，经 convertFileSrc 转 asset:// URL —— 由 WebView 原生读文件渲染。
//
// 关键：**不做 base64 中转**。图片就在本地磁盘，走 asset:// 让浏览器直接读文件，
// 零额外内存拷贝、零大小限制；早先「读回 base64 → dataURL → CSS 变量」的实现对大图
// （数 MB）会生成更大的 base64 串，注入与解码都极慢甚至失败，小图能过、大图出不来。
//
// 兼容：读取时识别历史遗留的 dataURL 值，经 migrateLegacyImage 转存为文件。

import { convertFileSrc, invoke } from '@tauri-apps/api/core'

export type UserImageKind = 'skin' | 'avatar' | 'nuphus-avatar'

interface SavedUserImage {
  name: string
  path: string
}

const LEGACY_PREFIX = 'data:image'

/** 用户图片变更事件名：主题页保存 / 清除后派发，聊天等视图据此重载。 */
export const USER_IMAGES_CHANGED_EVENT = 'nuphus:user-images-changed'

/** localStorage key（只存文件名；历史遗留值可能是 dataURL）。 */
export const SKIN_BG_KEY = 'nuphus_skin_bg'
export const USER_AVATAR_KEY = 'nuphus_user_avatar'
export const NUPHUS_AVATAR_KEY = 'nuphus_nuphus_avatar'

function notifyChanged(): void {
  window.dispatchEvent(new Event(USER_IMAGES_CHANGED_EVENT))
}

/** localStorage 里是历史遗留的 dataURL，还是现在的文件名。 */
export function isLegacyDataUrl(stored: string): boolean {
  return stored.startsWith(LEGACY_PREFIX)
}

/** 磁盘路径 → WebView 可直接加载的 asset:// URL（图片渲染的唯一出口）。
 *
 * Windows 上 Rust 回传的是反斜杠路径，而 convertFileSrc 只做 encodeURIComponent：
 * `\` 会被编码成 %5C，asset 协议 handler 解 URL 时不把 %5C 当路径分隔符，
 * 于是「文件确实存在、localStorage 也写入了，但图片加载不出来」。
 * 先归一化为正斜杠再交给 convertFileSrc（正斜杠在 Windows 上同样有效）。
 */
export function toImageUrl(path: string): string {
  return convertFileSrc(path.replace(/\\/g, '/'))
}

/**
 * 保存：落盘 + localStorage 只留文件名（替换旧文件）。返回可直接用于 CSS/图片的 asset URL。
 * 保存失败（磁盘不可写等）时抛出，由调用方提示用户。
 */
export async function saveStoredImage(
  kind: UserImageKind,
  storageKey: string,
  dataUrl: string,
): Promise<string> {
  const previous = localStorage.getItem(storageKey) || ''
  // 旧 dataURL 不在磁盘上，无从删除，oldName 传空串。
  const oldName = isLegacyDataUrl(previous) ? '' : previous
  const saved = await invoke<SavedUserImage>('save_user_image', {
    kind,
    oldName,
    dataUrl,
  })
  localStorage.setItem(storageKey, saved.name)
  notifyChanged()
  return toImageUrl(saved.path)
}

/** 展示：文件名 → asset URL；历史 dataURL 原样返回；读失败返回空串（无图态）。 */
export async function loadStoredImage(storageKey: string): Promise<string> {
  const stored = localStorage.getItem(storageKey) || ''
  if (!stored) return ''
  if (isLegacyDataUrl(stored)) return stored
  try {
    const path = await invoke<string>('read_user_image', { name: stored })
    return toImageUrl(path)
  } catch {
    return ''
  }
}

/**
 * 迁移：把历史 dataURL 值转存为文件，localStorage 改写为文件名；已是文件名则原样。
 * 返回迁移后的 asset URL（无需迁移时为空串）。
 */
export async function migrateLegacyImage(kind: UserImageKind, storageKey: string): Promise<string> {
  const stored = localStorage.getItem(storageKey) || ''
  if (!stored || !isLegacyDataUrl(stored)) return ''
  try {
    const saved = await invoke<SavedUserImage>('save_user_image', {
      kind,
      oldName: '',
      dataUrl: stored,
    })
    localStorage.setItem(storageKey, saved.name)
    notifyChanged()
    return toImageUrl(saved.path)
  } catch {
    return ''
  }
}

/**
 * 清除：删磁盘文件 + localStorage key（幂等）。
 * 删除失败不阻断清除：key 照删、界面回无图态，孤立文件影响仅限磁盘占用。
 */
export async function deleteStoredImage(storageKey: string): Promise<void> {
  const stored = localStorage.getItem(storageKey) || ''
  if (stored && !isLegacyDataUrl(stored)) {
    try {
      await invoke('delete_user_image', { name: stored })
    } catch {
      // 忽略：见上方说明
    }
  }
  localStorage.removeItem(storageKey)
  notifyChanged()
}
