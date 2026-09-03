/**
 * SequenceEdge.tsx — 实线顺序边（1.4a）
 * loopback / cond 装饰边复用本组件（虚线变体）。
 *
 * 连线中点插入手柄（大王反馈）：
 * - 非 decorative 的 sequence 边中点渲染常驻小圆点；hover 变「+」高亮
 * - readOnly（运行中 / 旧格式）与装饰边（loopback/cond）不渲染手柄
 * - 点击圆点：stopPropagation（防冒泡触发画布空白点击）→ 派发 EDGE_INSERT_EVENT
 *   CustomEvent（detail: { edgeId, x, y }，屏幕坐标），CanvasPage 监听后弹出「在此插入」菜单
 * - 手柄尺寸按 zoom 反推 flow 单位半径 → 屏幕上恒定的可点击/可读尺寸（minZoom 0.2 也可用）
 */

import { memo } from 'react'
import { BaseEdge, getBezierPath, useStore, type EdgeProps, type Edge } from '@xyflow/react'

export type SequenceFlowEdge = Edge<
  { decorative?: boolean; label?: string; readOnly?: boolean },
  'sequence'
>

/** CanvasPage 监听此事件打开连线中点插入菜单（SequenceEdge 无 RF props 外回调，用 window 事件最稳） */
export const EDGE_INSERT_EVENT = 'wf-insert-at'

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
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  })
  const zoom = useStore(s => s.transform[2])
  // 目标屏幕半径 → flow 单位半径；minZoom=0.2，封顶防缩小到极限时手柄吞掉大片区域
  const z = Math.max(zoom, 0.2)
  const hitR = Math.min(44, 15 / z)
  const ringR = Math.min(32, 8 / z)
  const dotR = Math.min(18, 4 / z)
  const arm = Math.min(26, 7 / z)

  const insertable = !data?.decorative && !data?.readOnly

  const onDotClick = (e: React.MouseEvent) => {
    // 防冒泡到 RF pane → 不触发画布空白点击（清空选择）
    e.stopPropagation()
    window.dispatchEvent(
      new CustomEvent<{ edgeId: string; x: number; y: number }>(EDGE_INSERT_EVENT, {
        detail: { edgeId: id, x: e.clientX, y: e.clientY },
      }),
    )
  }

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
      {insertable && (
        <g className="wfc-edge-insert" onClick={onDotClick}>
          {/* 不可见命中区（唯一可点击元素，避免手柄干扰拖拽/平移判定） */}
          <circle className="wfc-edge-insert-hit" cx={labelX} cy={labelY} r={hitR} />
          {/* hover 展开环 */}
          <circle className="wfc-edge-insert-ring" cx={labelX} cy={labelY} r={ringR} />
          {/* 常驻圆点 */}
          <circle className="wfc-edge-insert-dot" cx={labelX} cy={labelY} r={dotR} />
          {/* hover 时显示「+」 */}
          <g className="wfc-edge-insert-plus" transform={`translate(${labelX} ${labelY})`}>
            <line x1={-arm} y1={0} x2={arm} y2={0} />
            <line x1={0} y1={-arm} x2={0} y2={arm} />
          </g>
        </g>
      )}
    </>
  )
})
