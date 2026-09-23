/**
 * 本机图片缩略图（手机端）：Agent 回复里的电脑本地图片路径 → 可见缩略图 + 点击放大。
 *
 * ⚠️ 为什么不用 `<img src="/file?path=…">`：中继通道的归属路由依赖
 *    `X-Tunnel-Device` 头，`<img>` 无法携带自定义头 → 中继下必然失败；token 也走
 *    Header。因此统一 fetch（见 api.fetchFileBlob）取 Blob → objectURL 渲染。
 *
 * 行为约定：
 * - 懒加载：进入视口才请求（老 WebView 无 IntersectionObserver 时退化为挂载即请求）；
 * - 缓存：同一路径并发/重复渲染共享同一次请求与同一个 objectURL（引用计数），
 *   最后一个使用者卸载时 revokeObjectURL，避免内存泄漏；
 * - 降级：请求失败 / 解码失败 → 显示原始路径文本（绝不空白、绝不破图图标）。
 */

import { useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
import { fetchFileBlob } from '../api'

interface CachedImage {
  /** 已完成或进行中的 objectURL 请求（同一路径共享，避免重复下载） */
  url: Promise<string>
  /** 当前挂载中的使用者数量：归零即回收 objectURL */
  refs: number
}

const imageCache = new Map<string, CachedImage>()

/** 取用某路径的 objectURL（引用计数 +1）；同路径复用同一请求 */
function acquireImageUrl(path: string): Promise<string> {
  const hit = imageCache.get(path)
  if (hit) {
    hit.refs += 1
    return hit.url
  }
  const entry: CachedImage = {
    refs: 1,
    url: fetchFileBlob(path).then(blob => URL.createObjectURL(blob)),
  }
  imageCache.set(path, entry)
  // 失败不驻留缓存：否则一次网络抖动会让该路径永久降级（重挂载也无法重试）
  entry.url.catch(() => {
    if (imageCache.get(path) === entry) imageCache.delete(path)
  })
  return entry.url
}

/** 归还引用（-1）；归零时 revokeObjectURL 并移出缓存 */
function releaseImageUrl(path: string): void {
  const entry = imageCache.get(path)
  if (!entry) return
  entry.refs -= 1
  if (entry.refs > 0) return
  imageCache.delete(path)
  void entry.url.then(
    url => URL.revokeObjectURL(url),
    () => {
      /* 请求本身失败：无 objectURL 可回收 */
    },
  )
}

interface Props {
  /** 电脑本地图片绝对路径（原样显示于降级态，不加工） */
  path: string
}

export default function LocalImage({ path }: Props) {
  const [url, setUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  const [preview, setPreview] = useState(false)
  const holderRef = useRef<HTMLSpanElement | null>(null)

  useEffect(() => {
    let cancelled = false
    let acquired = false
    setUrl(null)
    setFailed(false)

    const load = () => {
      if (acquired) return
      acquired = true
      acquireImageUrl(path).then(
        objectUrl => {
          if (!cancelled) setUrl(objectUrl)
        },
        () => {
          if (!cancelled) setFailed(true)
        },
      )
    }

    const holder = holderRef.current
    const canObserve = typeof IntersectionObserver !== 'undefined' && holder !== null
    let observer: IntersectionObserver | null = null
    if (canObserve) {
      observer = new IntersectionObserver(
        entries => {
          if (entries.some(e => e.isIntersecting)) {
            load()
            observer?.disconnect()
          }
        },
        // 提前 200px 触发：滚动到附近即开始拉取，减少停顿感
        { rootMargin: '200px' },
      )
      observer.observe(holder)
    } else {
      load()
    }

    return () => {
      cancelled = true
      observer?.disconnect()
      // 只有真正取用过引用才归还，避免未进入视口的实例误减他人计数
      if (acquired) releaseImageUrl(path)
    }
  }, [path])

  // ▸ 降级：原始路径文本（可换行，不截断）
  if (failed) {
    return (
      <span className="m-local-image-error" data-image-path={path}>
        {path}
      </span>
    )
  }

  return (
    <span className="m-local-image" ref={holderRef} data-image-path={path}>
      {url ? (
        <img
          className="mobile-msg-image"
          src={url}
          alt={path}
          loading="lazy"
          onClick={() => setPreview(true)}
        />
      ) : (
        // 占位（保持版面稳定，避免图片到达时页面跳动）
        <span className="m-local-image-pending" aria-busy="true" aria-label={path} />
      )}
      {preview && url && (
        <span
          className="mobile-lightbox"
          role="dialog"
          aria-label="图片预览"
          onClick={() => setPreview(false)}
        >
          <img src={url} alt="图片预览" />
          <button
            type="button"
            className="mobile-lightbox-close"
            onClick={() => setPreview(false)}
            aria-label="关闭预览"
          >
            <X size={20} aria-hidden="true" />
          </button>
        </span>
      )}
    </span>
  )
}
