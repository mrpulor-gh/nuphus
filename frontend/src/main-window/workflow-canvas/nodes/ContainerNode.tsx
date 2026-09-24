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
import { useLanguage } from '../../../locales'
import { NodeKindBadge } from '../NodeKindBadge'
import { nodeKindLabel } from '../presentation'

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
  const { t } = useLanguage()
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
    <div className={classes} title={t('workflowCanvas.node.enter')}>
      <Handle type="target" position={targetPos} className="wfc-handle" />
      {actions && (
        <div className="wfc-node-actions" onClick={e => e.stopPropagation()}>
          <button
            type="button"
            className="wfc-node-act"
            title={t('common.edit')}
            onClick={() => actions.onEdit(node.id)}
          >
            <Pencil size={11} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="wfc-node-act"
            title={t('common.copy')}
            onClick={() => actions.onDuplicate(node.id)}
          >
            <Copy size={11} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="wfc-node-act wfc-node-act--danger"
            title={t('common.delete')}
            onClick={() => actions.onDelete(node.id)}
          >
            <Trash2 size={11} aria-hidden="true" />
          </button>
        </div>
      )}
      <div className="wfc-node-head">
        <span
          className="wfc-node-icon"
          data-kind={node.kind}
          title={t('workflowCanvas.node.type', nodeKindLabel(node.kind, t))}
        >
          <Icon size={13} aria-hidden="true" />
        </span>
        <span className="wfc-node-name" title={node.name}>
          {node.name}
        </span>
        <span className="wfc-badge wfc-badge--count">
          {t('workflowCanvas.node.count', String(node.childCount ?? 0))}
        </span>
        {badge && badge.error > 0 && (
          <span
            className="wfc-dot wfc-dot--error"
            title={t('workflowCanvas.node.childErrors', String(badge.error))}
          />
        )}
        {badge && badge.error === 0 && badge.running > 0 && (
          <span
            className="wfc-dot wfc-dot--running"
            title={t('workflowCanvas.node.childRunning', String(badge.running))}
          />
        )}
      </div>
      {node.containerSummary && (
        <div className="wfc-node-summary" title={node.containerSummary}>
          {node.containerSummary}
        </div>
      )}
      <div className="wfc-node-foot">
        <NodeKindBadge kind={node.kind} />
        {node.capture && (
          <span
            className="wfc-badge wfc-badge--capture"
            title={t('workflowCanvas.node.output', node.capture)}
          >
            → {node.capture}
          </span>
        )}
        {node.onErrorLabel && (
          <span className="wfc-badge wfc-badge--onerror" title={node.onErrorLabel}>
            {node.onErrorLabel}
          </span>
        )}
      </div>
      <Handle type="source" position={sourcePos} className="wfc-handle" />
      {childrenPreview && (
        <div className="wfc-children-preview" aria-hidden="true">
          <div className="wfc-children-preview-title">
            {t('workflowCanvas.node.children', String(childrenPreview.total))}
          </div>
          <ul className="wfc-children-preview-list">
            {childrenPreview.items.map((c, i) => (
              <li key={`${c.name}-${i}`} className="wfc-children-preview-item">
                <span className="wfc-children-preview-name" title={c.name}>
                  {c.name}
                </span>
                <NodeKindBadge kind={c.kind} />
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
})
