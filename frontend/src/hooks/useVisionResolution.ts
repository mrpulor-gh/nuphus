import { useEffect, useState } from 'react'
import {
  getVisionResolution,
  VISION_RESOLUTION_CHANGED_EVENT,
  type VisionResolution,
} from '../main-window/lib/api'

/** 图像理解策略的**前端唯一事实源**。
 *
 * 挂载查询一次，之后监听 VISION_RESOLUTION_CHANGED_EVENT 失效重查——
 * 所有会改变解析结果的配置写入（主模型切换 / 视觉绑定保存 / 模型行视觉开关）
 * 都在 api.ts 的保存包装器内 dispatch 本事件，此处是唯一消费点。
 *
 * 为什么不能让消费方各自查询：同一事实三份副本（挂载/拖图/进栏各查一次），
 * 配置变更后必然漂移——气泡渲染与引用栏提示会互相矛盾。消费方只读返回值，
 * "怎么显示"自己定，"是什么"只有这里知道。
 *
 * 返回 null = 未查到 / 查询失败（后端 to_api_messages 仍按真实能力分流，
 * 消费方按「未知 → 宽松放行」处理，不误伤）。 */
export function useVisionResolution() {
  const [resolution, setResolution] = useState<VisionResolution | null>(null)

  useEffect(() => {
    let cancelled = false
    const refresh = () => {
      getVisionResolution()
        .then(r => {
          if (!cancelled) setResolution(r)
        })
        .catch(() => {
          if (!cancelled) setResolution(null)
        })
    }
    refresh()
    window.addEventListener(VISION_RESOLUTION_CHANGED_EVENT, refresh)
    return () => {
      cancelled = true
      window.removeEventListener(VISION_RESOLUTION_CHANGED_EVENT, refresh)
    }
  }, [])

  return resolution
}
