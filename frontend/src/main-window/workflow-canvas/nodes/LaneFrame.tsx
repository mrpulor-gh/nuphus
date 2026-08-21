/**
 * LaneFrame.tsx — 泳道背景框（if 双泳道，设计文档 1.2）
 * 非交互装饰节点：不可选、不可拖、置于节点层之下。
 */

import { memo } from 'react'
import type { NodeProps, Node } from '@xyflow/react'

export type LaneFrameFlow = Node<{ title: string; width: number; height: number; empty?: boolean }, 'lane'>

export const LaneFrame = memo(function LaneFrame({ data }: NodeProps<LaneFrameFlow>) {
  return (
    <div
      className={`wfc-lane${data.empty ? ' wfc-lane--empty' : ''}`}
      style={{ width: data.width, height: data.height }}
    >
      <div className="wfc-lane-title">{data.title}</div>
      {data.empty && <div className="wfc-lane-empty-hint">空分支 · 可添加步骤</div>}
    </div>
  )
})
