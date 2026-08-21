/**
 * 移动端图片压缩 Worker：把 FileReader / createImageBitmap / canvas 全部移出主线程。
 *
 * 背景（iOS 18 standalone PWA 实测）：主线程直接解码 48MP 相册原图会**冻结 UI**——
 * React 状态更新（菜单关闭/处理中指示器）无法渲染、超时定时器被阻塞、无错误提示，
 * 表现为「只弹已选择 toast，其它什么都没有」。
 * Worker 里解码/压缩不占主线程 → UI 始终响应，进度与结果必然可见。
 *
 * 协议：
 *   in : { id, file }                    （File 可结构化克隆，无需 transfer）
 *   out: { id, ok: true, dataUrl } | { id, ok: false, error }
 *
 * 压缩参数与主线程常量一致：最长边 ≤1920，单图 ≤500KB，原图 >20MB 拒绝。
 */

const MAX_IMAGE_EDGE = 1920
const JPEG_QUALITY = 0.8
const MAX_IMAGE_BYTES = 500 * 1024
const MAX_SOURCE_BYTES = 20 * 1024 * 1024

function dataUrlBytes(dataUrl: string): number {
  // data URL 字符数 ≈ 字节数 × 4/3（base64）
  return Math.floor((dataUrl.length * 3) / 4)
}

self.onmessage = async (e: MessageEvent<{ id: number; file: File }>) => {
  const { id, file } = e.data
  try {
    if (file.size > MAX_SOURCE_BYTES) {
      throw new Error('图片过大（>20MB）')
    }
    // createImageBitmap 在 Worker 内解码，绝不阻塞主线程
    const bitmap = await createImageBitmap(file)
    const scale = Math.min(1, MAX_IMAGE_EDGE / Math.max(bitmap.width, bitmap.height))
    const w = Math.max(1, Math.round(bitmap.width * scale))
    const h = Math.max(1, Math.round(bitmap.height * scale))
    const canvas = new OffscreenCanvas(w, h)
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('canvas 2d unavailable')
    ctx.drawImage(bitmap, 0, 0, w, h)
    bitmap.close()
    let quality = JPEG_QUALITY
    let blob = await canvas.convertToBlob({ type: 'image/jpeg', quality })
    while (blob.size > MAX_IMAGE_BYTES && quality > 0.35) {
      quality -= 0.15
      blob = await canvas.convertToBlob({ type: 'image/jpeg', quality })
    }
    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = () => resolve(reader.result as string)
      reader.onerror = () => reject(new Error('blob read failed'))
      reader.readAsDataURL(blob)
    })
    void dataUrlBytes // 保留计算（可选校验）
    self.postMessage({ id, ok: true, dataUrl })
  } catch (err) {
    self.postMessage({ id, ok: false, error: err instanceof Error ? err.message : String(err) })
  }
}

export {}
