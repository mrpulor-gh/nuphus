// VideoProgressBadge — 视频字幕获取进度的轻量状态行
//
// 渲染位置：ChatPanel 的 chat-input-dock 内、ChatInputBar 正上方，
// 与输入框左边缘对齐（dock 与 .chat-input-area 共享同一定位几何）。
// 仅在管线运行时显示，done/error 后自动消失（useVideoProgress 契约）。
//
// 视觉克制：无 emoji、无边框/背景/圆角，仅一个旋转 spinner（SVG，
// currentColor）+ 一行动态提示文字；颜色/字号/间距全部走 W1 语义 token。

import { LoaderCircle } from 'lucide-react'
import { useVideoProgress } from '../lib/useVideoProgress'

export function VideoProgressBadge() {
  const p = useVideoProgress()
  if (!p) return null

  const pct = p.percent != null ? ` ${Math.min(100, Math.round(p.percent))}%` : ''
  return (
    <div className="video-progress-badge" role="status">
      <LoaderCircle size={13} className="video-progress-badge-icon" aria-hidden />
      <span className="video-progress-badge-text">
        {p.message}
        {pct}
      </span>
    </div>
  )
}
