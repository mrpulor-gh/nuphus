/**
 * DataEdge.tsx — 虚线数据边（1.4b）
 * 变量名 label + 管道 tooltip；悬空引用为黄色（warning）；
 * 跨层/外部注入锚点边为更弱样式。边不可点选编辑（1.5#4/#5）。
 */

import { memo } from 'react'
import { BaseEdge, getBezierPath, type EdgeProps, type Edge } from '@xyflow/react'

export type DataFlowEdge = Edge<
  { label?: string; pipes?: string[]; dangling?: boolean; external?: boolean; producerStepId?: string },
  'data'
>

export const DataEdge = memo(function DataEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
}: EdgeProps<DataFlowEdge>) {
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition })
  const tip = [
    data?.producerStepId ? `生产者: ${data.producerStepId}` : null,
    data?.pipes?.length ? `管道: ${data.pipes.join(' → ')}` : null,
    data?.dangling ? '未捕获引用（运行时由 inputs/params 注入）' : null,
  ].filter(Boolean).join('\n')
  const cls = [
    'wfc-edge-data',
    data?.dangling ? 'wfc-edge-data--dangling' : '',
    data?.external ? 'wfc-edge-data--external' : '',
  ].filter(Boolean).join(' ')
  return (
    <>
      <BaseEdge id={id} path={path} className={cls} interactionWidth={0}>
        {tip && <title>{tip}</title>}
      </BaseEdge>
      {data?.label && (
        <text x={labelX} y={labelY - 4} className={`wfc-edge-label${data?.dangling ? ' wfc-edge-label--dangling' : ''}`} textAnchor="middle">
          {data.label}
        </text>
      )}
    </>
  )
})
