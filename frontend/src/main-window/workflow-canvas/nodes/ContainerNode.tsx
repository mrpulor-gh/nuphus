/**
 * ContainerNode.tsx — 容器节点（设计文档 1.3）
 * 卡片 + 子层统计徽标 + 条件/循环摘要 + 双击下钻 + 跨层聚合徽标（红点优先于蓝点）。
 */

import { memo, useContext } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import { ListOrdered, Repeat, GitFork, Hourglass, Pencil, Copy, Trash2 } from 'lucide-react'
import type { CanvasNode, StepVisualStatus } from '../types'
import type { LayoutDir } from '../layout'
import { NodeActionsContext } from './StepNode'

export type ContainerNodeFlow = Node<
  {
    canvas: CanvasNode
    status?: StepVisualStatus
    problem?: 'error' | 'warning'
    badge?: { running: number; error: number }
    dir?: LayoutDir
    /** 子步骤轻量预览（hover 浮出）：非合成子节点 {name, kind}（≤8 条）+ 总数 */
    childrenPreview?: { total: number; items: { name: string; kind: string }[] }
  },
  'container'
>

export const CONTAINER_ICONS = {
  seq: ListOrdered,
  loop: Repeat,
  if: GitFork,
  wait: Hourglass,
} as const

export const ContainerNode = memo(function ContainerNode({
  data,
  selected,
}: NodeProps<ContainerNodeFlow>) {
  const { canvas: node, status, problem, badge, childrenPreview } = data
  const actions = useContext(NodeActionsContext)
  const Icon = CONTAINER_ICONS[node.kind as keyof typeof CONTAINER_ICONS] ?? ListOrdered

  // LR 层（root 横向流）→ 左右锚点；TB 层（子层树状）→ 上下锚点
  const targetPos = data.dir === 'LR' ? Position.Left : Position.Top
  const sourcePos = data.dir === 'LR' ? Position.Right : Position.Bottom

  const classes = [
    'wfc-node',
    'wfc-node--container',
    selected ? 'is-selected' : '',
    status ? ` wfc-node--${status.state}` : '',
    problem ? `wfc-node--check-${problem}` : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <div className={classes} title="双击进入子层">
      <Handle type="target" position={targetPos} className="wfc-handle" />
      {actions && (
        <div className="wfc-node-actions" onClick={e => e.stopPropagation()}>
          <button
            type="button"
            className="wfc-node-act"
            title="编辑"
            onClick={() => actions.onEdit(node.id)}
          >
            <Pencil size={11} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="wfc-node-act"
            title="复制"
            onClick={() => actions.onDuplicate(node.id)}
          >
            <Copy size={11} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="wfc-node-act wfc-node-act--danger"
            title="删除"
            onClick={() => actions.onDelete(node.id)}
          >
            <Trash2 size={11} aria-hidden="true" />
          </button>
        </div>
      )}
      <div className="wfc-node-head">
        <span className="wfc-node-icon" data-kind={node.kind}>
          <Icon size={13} aria-hidden="true" />
        </span>
        <span className="wfc-node-name" title={node.name}>
          {node.name}
        </span>
        <span className="wfc-badge wfc-badge--count">{node.childCount ?? 0} 步</span>
        {badge && badge.error > 0 && (
          <span className="wfc-dot wfc-dot--error" title={`子层 ${badge.error} 个失败`} />
        )}
        {badge && badge.error === 0 && badge.running > 0 && (
          <span className="wfc-dot wfc-dot--running" title={`子层 ${badge.running} 个运行中`} />
        )}
      </div>
      {node.containerSummary && (
        <div className="wfc-node-summary" title={node.containerSummary}>
          {node.containerSummary}
        </div>
      )}
      <div className="wfc-node-foot">
        <span className="wfc-node-kind">{node.kind}</span>
        {node.capture && <span className="wfc-badge wfc-badge--capture">→ {node.capture}</span>}
        {node.onErrorLabel && (
          <span className="wfc-badge wfc-badge--onerror">{node.onErrorLabel}</span>
        )}
      </div>
      <Handle type="source" position={sourcePos} className="wfc-handle" />
      {childrenPreview && (
        <div className="wfc-children-preview" aria-hidden="true">
          <div className="wfc-children-preview-title">子步骤 · {childrenPreview.total}</div>
          <ul className="wfc-children-preview-list">
            {childrenPreview.items.map((c, i) => (
              <li key={`${c.name}-${i}`} className="wfc-children-preview-item">
                <span className="wfc-children-preview-name" title={c.name}>
                  {c.name}
                </span>
                <span className="wfc-children-preview-kind">{c.kind}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
})
