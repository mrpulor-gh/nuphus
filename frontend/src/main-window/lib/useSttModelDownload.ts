// useSttModelDownload — STT 模型按需下载的共享状态（事件驱动，无轮询）
//
// 契约（与 src-tauri/src/speech/download.rs 一致）：
//   命令 stt_download_model（后台线程下载，立即返回）
//   事件 stt:download — { kind:'progress', file, downloaded, total, index, count }
//                       { kind:'done' } / { kind:'error', message } 终态恰好一次
// 消费者：VoiceButton 下载确认弹窗 + ModelsPage STT 卡片。
// 两处可能同时挂载：事件是全局广播的，双方都跟随同一条进度流；
// 重复调用 start() 时后端回 stt_download_busy，前端转为「跟随进行中的下载」。

import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '../../core/bridge'
import { sttDownloadModel, type SttDownloadPayload } from './api'

export interface SttDownloadProgress {
  file: string
  downloaded: number
  /** 0 = 服务端未给 Content-Length，UI 应退化为只显示已下载体积 */
  total: number
  index: number
  count: number
}

export function useSttModelDownload(onDone?: () => void) {
  const [downloading, setDownloading] = useState(false)
  const [progress, setProgress] = useState<SttDownloadProgress | null>(null)
  const [error, setError] = useState('')
  const onDoneRef = useRef(onDone)
  onDoneRef.current = onDone

  useEffect(() => {
    let unl: (() => void) | undefined
    let mounted = true
    ;(async () => {
      const u = await listen<SttDownloadPayload>('stt:download', p => {
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
          onDoneRef.current?.()
        } else if (p.kind === 'error') {
          setDownloading(false)
          setError(p.message || 'download failed')
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
  }, [])

  const start = useCallback(async () => {
    setError('')
    try {
      await sttDownloadModel()
      setDownloading(true)
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      if (msg.includes('stt_download_busy')) {
        // 另一处已发起下载：跟随其全局进度事件即可，不视为错误
        setDownloading(true)
      } else {
        setError(msg)
      }
    }
  }, [])

  return { downloading, progress, error, start }
}

/** 进度展示文本：已知总大小 → 百分比；未知 → 已下载 MB */
export function sttDownloadProgressText(p: SttDownloadProgress): string {
  if (p.total > 0) {
    return `${Math.min(100, Math.round((p.downloaded / p.total) * 100))}%（${p.index}/${p.count}）`
  }
  return `已下载 ${(p.downloaded / 1024 / 1024).toFixed(0)} MB（${p.index}/${p.count}）`
}

/** 进度条宽度百分比（0-100）；未知总大小时返回 null（UI 隐藏填充条） */
export function sttDownloadProgressPct(p: SttDownloadProgress): number | null {
  if (p.total <= 0) return null
  return Math.min(100, (p.downloaded / p.total) * 100)
}
