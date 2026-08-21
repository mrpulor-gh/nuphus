/**
 * SequenceEdge.tsx — 实线顺序边（1.4a）
 * loopback / cond 装饰边复用本组件（虚线变体）。
 */

import { memo } from 'react'
import { BaseEdge, getBezierPath, type EdgeProps, type Edge } from '@xyflow/react'

export type SequenceFlowEdge = Edge<{ decorative?: boolean; label?: string }, 'sequence'>

export const SequenceEdge = memo(function SequenceEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  markerEnd,
}: EdgeProps<SequenceFlowEdge>) {
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition })
  return (
    <>
      <BaseEdge
        id={id}
        path={path}
        markerEnd={markerEnd}
        className={`wfc-edge-seq${data?.decorative ? ' wfc-edge-seq--decorative' : ''}`}
      />
      {data?.label && (
        <text x={labelX} y={labelY} className="wfc-edge-label" textAnchor="middle">
          {data.label}
        </text>
      )}
    </>
  )
})
