// ScreenCaptureTool.tsx — 交互式截图工具
// 用户框选屏幕区域 → 保存为模板图片 → 返回路径给工作流/找图

import { useState } from 'react'
import { invoke } from '../../core/bridge'
import { RegionPicker } from './RegionPicker'

interface Region {
  x: number
  y: number
  width: number
  height: number
}

interface ScreenCaptureToolProps {
  onClose: () => void
  onCaptured: (result: { path: string; region: Region }) => void
  /** 预载截图路径（由外部在窗口隐藏时拍好） */
  bgImagePath?: string
}

export function ScreenCaptureTool({ onClose, onCaptured, bgImagePath }: ScreenCaptureToolProps) {
  const [saving, setSaving] = useState(false)

  const handleConfirm = async (region: Region) => {
    setSaving(true)
    try {
      // 不传 path：Rust 端自动使用 captures_dir() 生成正确绝对路径
      const result = await invoke<{ width: number; height: number; path: string }>('execute_tool', {
        tool_name: 'desktop_screenshot',
        params: {
          region: { x: region.x, y: region.y, width: region.width, height: region.height },
        },
      })

      if (result?.path) {
        onCaptured({ path: result.path, region })
      } else {
        console.error('Screenshot save returned no path')
      }
    } catch (e) {
      console.error('Screenshot capture failed:', e)
    } finally {
      setSaving(false)
    }
  }

  return (
    <RegionPicker
      mode="capture"
      bgImagePath={bgImagePath}
      onClose={onClose}
      onConfirm={handleConfirm}
    />
  )
}
