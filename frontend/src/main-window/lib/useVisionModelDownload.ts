// useVisionModelDownload — 本地视觉模型（OCR / YOLO）自动下载的共享状态
// （事件驱动，无轮询）。契约与 src-tauri/src/models/bootstrap.rs 一致：
//   命令 vision_models_status —— 只读快照 { ocrReady, yoloReady, missing, dir, downloading }
//   命令 preload_ocr         —— 触发 / 重试下载（后台线程，立即返回）
//   事件 models:download —— { kind:'progress', file, downloaded, total, index, count }
//                           { kind:'done', ocr_ready, yolo_ready }
//                           { kind:'error', message }  终态恰好一次
// 与 STT 的 useSttModelDownload 同构；首启下载由应用启动时 bootstrap 自动发起，
// 本 hook 仅跟随进度流 + 提供手动 refresh / retry。

import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '../../core/bridge'
import {
  visionModelsStatus,
  retryVisionDownload,
  type ModelsDownloadPayload,
  type VisionModelsStatus,
} from './api'

export interface ModelsDownloadProgress {
  file: string
  downloaded: number
  /** 0 = 服务端未给 Content-Length，UI 应退化为只显示已下载体积 */
  total: number
  index: number
  count: number
}

export function useVisionModelDownload(onDone?: () => void) {
  const [status, setStatus] = useState<VisionModelsStatus | null>(null)
  const [downloading, setDownloading] = useState(false)
  const [progress, setProgress] = useState<ModelsDownloadProgress | null>(null)
  const [error, setError] = useState('')
  const onDoneRef = useRef(onDone)
  onDoneRef.current = onDone

  /** 重新查询后端状态快照（进入 tab / done / error 后调用） */
  const refresh = useCallback(async () => {
    try {
      const s = await visionModelsStatus()
      if (!s) return null
      setStatus(s)
      setDownloading(s.downloading)
      return s
    } catch {
      return null
    }
  }, [])

  useEffect(() => {
    let unl: (() => void) | undefined
    let mounted = true
    ;(async () => {
      const u = await listen<ModelsDownloadPayload>('models:download', p => {
        if (p.kind === 'progress') {
          setDownloading(true)
          setError('')
          setProgress({
            file: p.file,
            downloaded: p.downloaded,
            total: p.total,
            index: p.index,
            count: p.count,
          })
        } else if (p.kind === 'done') {
          setDownloading(false)
          setProgress(null)
          setError('')
          refresh()
          onDoneRef.current?.()
        } else if (p.kind === 'error') {
          setDownloading(false)
          setError(p.message || 'download failed')
          refresh()
        }
      })
      if (!mounted) {
        u()
        return
      }
      unl = u
    })()
    return () => {
      mounted = false
      unl?.()
    }
  }, [refresh])

  /** 手动触发 / 重试下载（幂等：后端单飞，已存在则直接完成） */
  const retry = useCallback(async () => {
    setError('')
    setProgress(null)
    try {
      await retryVisionDownload()
      setDownloading(true)
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      setError(msg)
    }
  }, [])

  return { status, downloading, progress, error, refresh, retry }
}

/** 进度展示文本：已知总大小 → 百分比；未知 → 已下载 MB */
export function modelsDownloadProgressText(p: ModelsDownloadProgress): string {
  if (p.total > 0) {
    return `${Math.min(100, Math.round((p.downloaded / p.total) * 100))}%（${p.index}/${p.count}）`
  }
  return `已下载 ${(p.downloaded / 1024 / 1024).toFixed(0)} MB（${p.index}/${p.count}）`
}

/** 进度条宽度百分比（0-100）；未知总大小时返回 null（UI 隐藏填充条） */
export function modelsDownloadProgressPct(p: ModelsDownloadProgress): number | null {
  if (p.total <= 0) return null
  return Math.min(100, (p.downloaded / p.total) * 100)
}
