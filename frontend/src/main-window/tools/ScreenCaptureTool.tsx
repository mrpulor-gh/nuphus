// ScreenCaptureTool.tsx — 交互式截图工具
// 用户框选屏幕区域 → 保存为模板图片 → 返回路径给工作流/找图

import { useState } from 'react'
import { invoke } from '../../core/bridge'
import { backendErrorMessage, isAutomationBusy } from '../lib/api'
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
  /** Preload screenshot path (captured by the caller while the window was hidden) */
  bgImagePath?: string
}

export function ScreenCaptureTool({ onClose, onCaptured, bgImagePath }: ScreenCaptureToolProps) {
  const [saving, setSaving] = useState(false)
  /** 保存失败文案（后端资源门拒绝时是可直接展示的人话） */
  const [errorMessage, setErrorMessage] = useState<string | undefined>()

  const handleConfirm = async (region: Region) => {
    setSaving(true)
    setErrorMessage(undefined)
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
        setErrorMessage('截图保存失败：未返回文件路径')
      }
    } catch (e) {
      console.error('Screenshot capture failed:', e)
      // 资源互斥被拒（automation_busy）等后端错误必须展示——否则用户只会看到
      // 「确认无反应」，误以为工具坏了。文案由后端给出（已剥掉稳定码前缀）。
      setErrorMessage(
        isAutomationBusy(e) ? backendErrorMessage(e) : `截图保存失败：${backendErrorMessage(e)}`,
      )
    } finally {
      setSaving(false)
    }
  }

  return (
    <RegionPicker
      mode="capture"
      bgImagePath={bgImagePath}
      errorMessage={errorMessage}
      onClose={onClose}
      onConfirm={handleConfirm}
    />
  )
}
