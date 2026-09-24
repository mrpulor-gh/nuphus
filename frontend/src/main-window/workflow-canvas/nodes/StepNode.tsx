/**
 * StepNode.tsx — 叶子节点卡片（设计文档 1.3 视觉规格）
 * kind 图标 + 名称 + capture 徽章 + on_error≠abort 角标 + 执行状态环 + 校验角标。
 * 合成锚点节点（entry/cond/external）复用本组件，虚框样式区分。
 */

import { memo, createContext, useContext } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import {
  Wrench,
  MessageSquare,
  FileCode2,
  ShieldCheck,
  Plug,
  Moon,
  CircleStop,
  SkipForward,
  Hourglass,
  AlertTriangle,
  LogIn,
  LogOut,
  GitBranch,
  Pencil,
  Copy,
  Trash2,
  Settings2,
} from 'lucide-react'
import type { CanvasNode, StepVisualStatus } from '../types'
import type { LayoutDir } from '../layout'
import '../workflow-node-layout.css'

/** 节点 hover 操作（阶段 4：编辑/复制/删除快捷入口） */
export interface NodeActions {
  onEdit: (id: string) => void
  onDuplicate: (id: string) => void
  onDelete: (id: string) => void
}

/** 由 CanvasPage 注入（ReactFlow 外层 Provider），节点组件内消费；null = 只读/无操作 */
export const NodeActionsContext = createContext<NodeActions | null>(null)

export interface InputAnchorActions {
  onConfigure: (name: string) => void
}

export const InputAnchorActionsContext = createContext<InputAnchorActions | null>(null)

export type StepNodeFlow = Node<
  { canvas: CanvasNode; status?: StepVisualStatus; problem?: 'error' | 'warning'; dir?: LayoutDir },
  'step'
>

export const KIND_ICONS: Record<string, typeof Wrench> = {
  tool: Wrench,
  chat: MessageSquare,
  script: FileCode2,
  assert: ShieldCheck,
  mcp: Plug,
  sleep: Moon,
  break: CircleStop,
  continue: SkipForward,
  call: GitBranch,
  wait: Hourglass,
  custom: AlertTriangle,
}

function statusClass(status?: StepVisualStatus): string {
  if (!status) return ''
  return ` wfc-node--${status.state}`
}

export const StepNode = memo(function StepNode({ data, selected }: NodeProps<StepNodeFlow>) {
  const { canvas: node, status, problem } = data
  const actions = useContext(NodeActionsContext)
  const inputActions = useContext(InputAnchorActionsContext)
  const Icon = KIND_ICONS[node.kind] ?? Wrench

  // LR 层（root 横向流）→ 左右锚点；TB 层（子层树状）→ 上下锚点
  const targetPos = data.dir === 'LR' ? Position.Left : Position.Top
  const sourcePos = data.dir === 'LR' ? Position.Right : Position.Bottom

  // 合成锚点节点（1.3：仅 UI 内部）
  if (node.synthetic) {
    const AnchorIcon =
      node.synthetic === 'entry' ? LogIn : node.synthetic === 'cond' ? GitBranch : LogOut
    return (
      <div
        className={`wfc-node wfc-anchor wfc-anchor--${node.synthetic}${node.externalInputDeclared === false ? ' is-undeclared' : ''}`}
        title={node.externalProducerId ? `生产者：${node.externalProducerId}` : node.name}
      >
        <Handle type="target" position={targetPos} className="wfc-handle" />
        <AnchorIcon size={12} aria-hidden="true" />
        <span className="wfc-anchor-name">{node.name}</span>
        {node.externalInputDeclared === false && <span className="wfc-anchor-sub">未找到来源</span>}
        {node.containerSummary && <span className="wfc-anchor-sub">{node.containerSummary}</span>}
        {node.externalInput && node.externalVar && inputActions && (
          <button
            type="button"
            className="wfc-anchor-settings"
            title={`设置外部输入 ${node.externalVar}`}
            aria-label={`设置外部输入 ${node.externalVar}`}
            onClick={event => {
              event.stopPropagation()
              inputActions.onConfigure(node.externalVar!)
            }}
          >
            <Settings2 size={12} aria-hidden="true" />
          </button>
        )}
        <Handle type="source" position={sourcePos} className="wfc-handle" />
      </div>
    )
  }

  const classes = [
    'wfc-node',
    'wfc-node--leaf',
    node.category === 'unknown' ? 'wfc-node--unknown' : '',
    selected ? 'is-selected' : '',
    statusClass(status),
    problem ? `wfc-node--check-${problem}` : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <div className={classes}>
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
        {node.onErrorLabel && (
          <span className="wfc-badge wfc-badge--onerror" title={node.onErrorLabel}>
            {node.onErrorLabel}
          </span>
        )}
      </div>
      <div className="wfc-node-foot">
        <span className="wfc-node-kind">{node.kind}</span>
        {node.capture && (
          <span className="wfc-badge wfc-badge--capture" title={`保存输出到 ${node.capture}`}>
            → {node.capture}
          </span>
        )}
        {node.shadowedBy && (
          <span
            className="wfc-badge wfc-badge--shadowed"
            title={`变量也由步骤 ${node.shadowedBy} 写入，实际值取决于执行路径`}
          >
            同名
          </span>
        )}
        {node.danglingVars && node.danglingVars.length > 0 && (
          <span
            className="wfc-badge wfc-badge--dangling"
            title={`未找到来源：${node.danglingVars.join(', ')}；可检查前序步骤或配置工作流输入`}
          >
            <AlertTriangle size={11} aria-label="未找到来源" />
          </span>
        )}
      </div>
      {status?.state === 'retrying' && (
        <span className="wfc-retry-badge">第 {status.attempt} 次重试</span>
      )}
      {status?.state === 'error' && status.message && (
        <span className="wfc-error-tip" title={status.message}>
          !
        </span>
      )}
      <Handle type="source" position={sourcePos} className="wfc-handle" />
    </div>
  )
})
