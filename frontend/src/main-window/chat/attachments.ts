import type { PendingFile, PendingImage } from '../../core/types'

/** 引用栏条目：由 HTML5 `File` 转换而来（拖拽与附件按钮共用同一条通道）。 */
export type AttachmentEntry =
  { kind: 'image'; image: PendingImage } | { kind: 'file'; file: PendingFile }

/** 图片扩展名白名单——与后端 `read_image_base64` 保持一致。 */
const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'ico'])

/** 单个附件总大小上限（2 GiB），与后端 `MAX_DROPPED_BYTES` 对齐。 */
export const MAX_ATTACHMENT_BYTES = 2 * 1024 * 1024 * 1024

/** 分块大小 8 MiB：内存峰值只与单块有关，大文件不再受单次 IPC 体积约束。 */
export const ATTACHMENT_CHUNK_BYTES = 8 * 1024 * 1024

/** 落盘进度（loaded/total 单位：字节）。 */
export interface AttachProgress {
  name: string
  loaded: number
  total: number
}

export function isImageFile(file: File): boolean {
  if (file.type.startsWith('image/')) return true
  return isImageName(file.name)
}

/** 按文件名判断图片（粘贴的剪贴板路径分支没有 `File.type`，只能看扩展名）。 */
export function isImageName(name: string): boolean {
  const dot = name.lastIndexOf('.')
  if (dot < 0) return false
  return IMAGE_EXTS.has(name.slice(dot + 1).toLowerCase())
}

/** 读 Blob/File 为 data URL（`data:<mime>;base64,<...>`）。 */
export function readFileAsDataUrl(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const url = reader.result
      typeof url === 'string' ? resolve(url) : reject(new Error('读取文件失败'))
    }
    reader.onerror = () => reject(reader.error ?? new Error('读取文件失败'))
    reader.readAsDataURL(file)
  })
}

/** data URL → 纯 base64 载荷（去掉 `data:...;base64,` 前缀）。 */
function stripDataUrlPrefix(dataUrl: string): string {
  const idx = dataUrl.indexOf(',')
  return idx >= 0 ? dataUrl.slice(idx + 1) : dataUrl
}

/**
 * 落盘换真实路径：浏览器屏蔽 `File.path`，非图片附件必须经后端写入缓存目录，
 * 拿到绝对路径后才能像历史原生通道一样被引用栏与后端读文件消费。
 *
 * 大文件走「begin → append(8MB/块) → 完成」的分块协议：内存峰值只与单块有关，
 * 50MB 以上的文件不再被单次 IPC 体积卡住；失败必清半截文件。
 */
async function persistFileForPath(
  file: File,
  onProgress?: (p: AttachProgress) => void,
): Promise<PendingFile> {
  if (file.size > MAX_ATTACHMENT_BYTES) {
    throw new Error(`超过 ${MAX_ATTACHMENT_BYTES / 1024 / 1024 / 1024}GB 上限`)
  }
  const { invoke } = await import('@tauri-apps/api/core')
  const begun = await invoke<{ path: string; name: string }>('begin_dropped_file', {
    fileName: file.name,
  })
  try {
    let offset = 0
    while (offset < file.size) {
      const end = Math.min(offset + ATTACHMENT_CHUNK_BYTES, file.size)
      const chunk = await readFileAsDataUrl(file.slice(offset, end))
      await invoke('append_dropped_file', {
        fileName: begun.name,
        chunkBase64: stripDataUrlPrefix(chunk),
        offset,
      })
      offset = end
      onProgress?.({ name: begun.name, loaded: offset, total: file.size })
    }
  } catch (e) {
    // 宁可删掉残件，也不在缓存目录留半个文件让用户误以为附件可用
    await invoke('abort_dropped_file', { fileName: begun.name }).catch(() => {})
    throw e
  }
  return { path: begun.path, name: begun.name || file.name }
}

/**
 * 单个 File → 引用栏条目。
 *
 * 图片：objectURL 立即给出可预览的 dataUrl 占位语义（这里直接用 dataUrl，
 * 保证发送链路拿到的就是可上传的字符串），调用方可先渲染再补齐。
 * 文档：落盘换绝对路径。
 */
export async function fileToAttachment(
  file: File,
  onProgress?: (p: AttachProgress) => void,
): Promise<AttachmentEntry> {
  if (isImageFile(file)) {
    const dataUrl = await readFileAsDataUrl(file)
    return { kind: 'image', image: { dataUrl, name: file.name } }
  }
  return { kind: 'file', file: await persistFileForPath(file, onProgress) }
}

/** 批量转换：逐个处理，单个失败不影响其余（返回成功项 + 失败描述）。 */
export async function filesToAttachments(
  files: File[],
  onProgress?: (p: AttachProgress) => void,
): Promise<{ entries: AttachmentEntry[]; failures: string[] }> {
  const entries: AttachmentEntry[] = []
  const failures: string[] = []
  for (const file of files) {
    try {
      entries.push(await fileToAttachment(file, onProgress))
    } catch (e) {
      failures.push(`${file.name}: ${e instanceof Error ? e.message : String(e)}`)
    }
  }
  return { entries, failures }
}
