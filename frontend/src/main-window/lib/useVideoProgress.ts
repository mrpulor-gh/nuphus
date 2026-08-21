// useVideoProgress — 视频字幕管线进度（事件驱动，无轮询）
//
// 契约（与 src-tauri/src/video/pipeline.rs 一致）：
//   事件 video:progress — { stage, percent?, message }
//   stage: probe | download_deps | fetch_subs | download_audio | convert | asr
//   终态 done / error 后清除显示。
// 消费者：ChatInputBar 上方的 VideoProgressBadge（唯一，事件全局广播）。

import { useEffect, useState } from 'react'
import { listen } from '../../core/bridge'

export interface VideoProgressPayload {
  stage: string
  percent?: number | null
  message: string
}

/** 当前进行中的视频字幕任务进度；无任务 / 已结束为 null */
export function useVideoProgress(): VideoProgressPayload | null {
  const [progress, setProgress] = useState<VideoProgressPayload | null>(null)

  useEffect(() => {
    let unl: (() => void) | undefined
    let mounted = true
    ;(async () => {
      const u = await listen<VideoProgressPayload>('video:progress', p => {
        if (p.stage === 'done' || p.stage === 'error') {
          setProgress(null)
        } else {
          setProgress(p)
        }
      })
      if (mounted) unl = u
      else u()
    })()
    return () => {
      mounted = false
      unl?.()
    }
  }, [])

  return progress
}
