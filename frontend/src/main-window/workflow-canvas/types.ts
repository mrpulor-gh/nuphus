/**
 * types.ts — 工作流画布类型契约（设计文档 1.2/1.3/1.4/1.5）
 *
 * 核心约束：
 * - 画布节点 id === Step.id，全局 1:1（compiler 保证全树唯一）
 * - 合成 id 带 `::` 后缀，仅 UI 内部使用，绝不回写 IR、绝不出现在事件流
 * - IR 是唯一真源；画布只持有「当前层视图状态 + 未保存编辑缓冲」
 */

import type { WorkflowStep, Action, RunRecord, ScheduleConfig } from '../../core/types'

// ── 后端 Workflow IR（wf_list 原始形状，不经过 WorkflowItem 归一化）──

export interface WorkflowIR {
  id: string
  name: string
  created_at?: string | null
  updated_at?: string | null
  status: string
  steps: WorkflowStep[]
  doc?: string | null
  schedule?: ScheduleConfig | null
  run_history?: RunRecord[]
  timeout_secs?: number | null
  dry_run?: boolean
}

// ── 步骤 kind（对齐 types.rs kind_str；custom 为画布特化：Action::Custom 旧格式）──

export type StepKind =
  | 'tool'
  | 'seq'
  | 'loop'
  | 'if'
  | 'call'
  | 'wait'
  | 'chat'
  | 'script'
  | 'assert'
  | 'mcp'
  | 'sleep'
  | 'break'
  | 'continue'
  | 'custom'

// ── 分层视图模型（1.2）──

export type LaneId = 'main' | 'then' | 'else'

export interface Swimlane {
  id: LaneId
  title: string
}

/** 节点视觉分类（1.3）：leaf=叶子 / container=容器 / unknown=Action::Custom 只读 */
export type NodeCategory = 'leaf' | 'container' | 'unknown'

export interface CanvasNode {
  /** === Step.id（合成锚点节点除外，见 SYNTH 前缀约定） */
  id: string
  kind: StepKind
  category: NodeCategory
  name: string
  lane: LaneId
  /** capture 变量名（有则显示徽章） */
  capture?: string
  /** on_error !== abort 时显示角标 */
  onErrorLabel?: string
  /** 容器：直接子步骤统计（"3 步"） */
  childCount?: number
  /** 容器摘要：loop=for_each/repeat/until + max；if=条件摘要 */
  containerSummary?: string
  /** 同名变量被后序 capture 遮蔽（1.4b 多生产者规则） */
  shadowedBy?: string
  /** 引用未捕获变量的悬空前缀列表（warning 黄点） */
  danglingVars?: string[]
  /** 合成节点标记（入口锚点/条件块/外部来源锚点）——非 IR 实体 */
  synthetic?: 'entry' | 'cond' | 'external'
  /** external 锚点：变量名 + 真实生产者 step.id（点击跳转用） */
  externalVar?: string
  externalProducerId?: string
}

export type EdgeKind = 'sequence' | 'data' | 'loopback' | 'cond' | 'external'

export interface CanvasEdge {
  /** 顺序边 id 约定 "seq:{from}->{to}"；数据边 "data:{from}->{to}:{var}" */
  id: string
  source: string
  target: string
  kind: EdgeKind
  /** 数据边：变量名标签 */
  label?: string
  /** 数据边：管道名（get/json/len/default），tooltip 展示 */
  pipes?: string[]
  /** 数据边：真实生产者 step.id（穿透容器时 source 是容器节点） */
  producerStepId?: string
  /** 悬空引用：连到「外部注入」锚点的黄色虚边 */
  dangling?: boolean
}

export interface CanvasLayer {
  /** "root" | 容器 step.id */
  layerId: string
  /** 面包屑：root → ... → 父容器 step.id */
  parentChain: string[]
  /** 容器 kind（root 层为 'seq' 语义） */
  containerKind: 'root' | 'seq' | 'loop' | 'if' | 'wait'
  swimlanes: Swimlane[]
  nodes: CanvasNode[]
  edges: CanvasEdge[]
}

// ── 投影产物（全树索引，状态叠加/聚合用）──

export interface ProjectionIndex {
  /** stepId → 父容器 step.id（root 直接子步骤的父为 null） */
  parentOf: Map<string, string | null>
  /** stepId → 所在 layerId */
  layerOf: Map<string, string>
  /** stepId → CanvasNode（跨层快速取节点元数据） */
  nodeById: Map<string, CanvasNode>
  /** 容器 step.id 列表（可下钻） */
  containerIds: Set<string>
  /** 是否存在 Action::Custom 节点（V13：整树只读 + 迁移横幅） */
  hasCustomNodes: boolean
}

// ── IrEditOp（1.5）──

/** LayerRef：定位一个子步骤列表的宿主 */
export interface LayerRef {
  /** "root" 或容器 step.id */
  layerId: string
}

export type IrEditOp =
  | { op: 'add_step'; parent: LayerRef; lane: LaneId; index: number; step: WorkflowStep }
  | { op: 'remove_step'; stepId: string }
  | { op: 'move_step'; stepId: string; to: { parent: LayerRef; lane: LaneId; index: number } }
  | { op: 'reorder'; stepId: string; newIndex: number }
  | { op: 'update_fields'; stepId: string; patch: Partial<WorkflowStep> }
  | { op: 'change_kind'; stepId: string; action: Action }

/** 拦截结果（1.5 矩阵）：ok=false 时 reason 带"为什么"+ 替代操作引导 */
export interface EditCheck {
  ok: boolean
  reason?: string
  /** 受限允许：需用户确认（如 change_kind 丢弃 else 分支） */
  confirm?: string
}

// ── 执行状态叠加（2.3）──

export type StepVisualStatus =
  | { state: 'running' }
  | { state: 'retrying'; attempt: number }
  | { state: 'success' }
  | { state: 'skipped' }
  | { state: 'error'; message?: string }
  | { state: 'paused'; reason?: string }

/** 合成 id 约定（1.3）：仅 UI 内部，绝不回写 IR */
export const SYNTH = {
  entry: (layerId: string) => `${layerId}::entry`,
  cond: (stepId: string) => `${stepId}::cond`,
  external: (layerId: string, varName: string) => `${layerId}::external::${varName}`,
  loopback: (stepId: string) => `${stepId}::loopback`,
} as const

/** 判断 id 是否为合成 id */
export function isSyntheticId(id: string): boolean {
  return id.includes('::')
}

// ── 布局 sidecar（1.7）──

export interface CanvasLayoutSidecar {
  version: 1
  layers: Record<string, { pos: Record<string, { x: number; y: number }> }>
}
