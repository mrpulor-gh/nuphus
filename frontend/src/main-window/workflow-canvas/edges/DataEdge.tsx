/**
 * DataEdge.tsx — 虚线数据边（1.4b）
 * 变量名 label + 管道 tooltip；悬空引用为黄色（warning）；
 * 跨层/外部注入锚点边为更弱样式。边不可点选编辑（1.5#4/#5）。
 */

import { memo } from 'react'
import { BaseEdge, getBezierPath, type EdgeProps, type Edge } from '@xyflow/react'

export type DataFlowEdge = Edge<
  {
    label?: string
    pipes?: string[]
    dangling?: boolean
    external?: boolean
    producerStepId?: string
    maybeUnset?: boolean
    sourceSummary?: string
  },
  'data'
>

/** CJK glyphs are roughly twice as wide as ASCII; keep labels inside the edge gutter. */
export function compactEdgeLabel(label: string, maxUnits = 18): string {
  let result = ''
  let units = 0
  for (const char of label) {
    units += char.charCodeAt(0) > 255 ? 2 : 1
    if (units > maxUnits) return `${result}…`
    result += char
  }
  return result
}

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
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  })
  const tip = [
    data?.producerStepId ? `生产者: ${data.producerStepId}` : null,
    data?.sourceSummary ? `可能来源: ${data.sourceSummary}` : null,
    data?.maybeUnset ? '可能未赋值：请检查分支、循环或输入默认值' : null,
    data?.pipes?.length ? `管道: ${data.pipes.join(' → ')}` : null,
    data?.dangling ? '未找到前序来源；请检查变量或配置工作流输入' : null,
  ]
    .filter(Boolean)
    .join('\n')
  const cls = [
    'wfc-edge-data',
    data?.dangling || data?.maybeUnset ? 'wfc-edge-data--dangling' : '',
    data?.external ? 'wfc-edge-data--external' : '',
  ]
    .filter(Boolean)
    .join(' ')
  return (
    <>
      <BaseEdge id={id} path={path} className={cls} interactionWidth={0}>
        {tip && <title>{tip}</title>}
      </BaseEdge>
      {data?.label && (
        <text
          x={labelX}
          y={labelY - 4}
          className={`wfc-edge-label${data?.dangling || data?.maybeUnset ? ' wfc-edge-label--dangling' : ''}`}
          textAnchor="middle"
        >
          <title>{[data.label, tip].filter(Boolean).join('\n')}</title>
          {compactEdgeLabel(data.label)}
        </text>
      )}
    </>
  )
})
