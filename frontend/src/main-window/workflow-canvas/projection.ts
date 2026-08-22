/**
 * projection.ts — IR → CanvasLayer 投影算法（设计文档第 1 章）
 *
 * 纯函数，可单测。核心规则：
 * - 节点 id === Step.id；容器子层 layerId === 容器 step.id；根层 "root"
 * - 顺序边 = 同泳道数组顺序；数据边 = capture/{{var}} 配对（遍历序最近前序生产者）
 * - 跨层边在父层聚合到「直接子节点」端点（穿透容器），真实生产者记入 producerStepId
 * - 合成节点（entry/cond/external 锚点）带 :: 后缀，绝不回写 IR
 */

import type { WorkflowStep, Condition, VarRef } from '../../core/types'
import type {
  CanvasEdge,
  CanvasLayer,
  CanvasNode,
  LaneId,
  ProjectionIndex,
  StepKind,
  Swimlane,
} from './types'
import { SYNTH } from './types'
import { profileStep, walkSteps, type StepVarProfile } from './dataEdges'

// ── kind 判定（对齐 types.rs kind_str；无法识别的 Action → custom）──

const KNOWN_ACTION_KEYS = [
  'tool',
  'seq',
  'loop',
  'if',
  'call',
  'wait',
  'chat',
  'script',
  'assert',
  'mcp',
  'sleep',
  'break',
  'continue',
] as const

export function stepKind(step: WorkflowStep): StepKind {
  const d = step.do as Record<string, unknown>
  if (!d || typeof d !== 'object') return 'custom'
  for (const k of KNOWN_ACTION_KEYS) {
    if (k in d) return k
  }
  return 'custom'
}

/** 容器判定：seq/loop/if 恒为容器；wait 带非空 auto 为容器（1.2 表） */
export function containerLanes(
  step: WorkflowStep,
): { kind: 'seq' | 'loop' | 'if' | 'wait'; lanes: Swimlane[] } | null {
  const d = step.do as Record<string, unknown> | undefined
  if (!d || typeof d !== 'object') return null
  if (Array.isArray(d.seq)) return { kind: 'seq', lanes: [{ id: 'main', title: '顺序' }] }
  if (d.loop && typeof d.loop === 'object')
    return { kind: 'loop', lanes: [{ id: 'main', title: '循环体' }] }
  if (d.if && typeof d.if === 'object')
    return {
      kind: 'if',
      lanes: [
        { id: 'then', title: 'THEN' },
        { id: 'else', title: 'ELSE' },
      ],
    }
  if (Array.isArray(d.auto) && (d.auto as unknown[]).length > 0)
    return { kind: 'wait', lanes: [{ id: 'main', title: '自动操作' }] }
  return null
}

/** 取容器在指定泳道的子步骤数组 */
export function laneSteps(step: WorkflowStep, lane: LaneId): WorkflowStep[] {
  const d = step.do as Record<string, unknown> | undefined
  if (!d || typeof d !== 'object') return []
  if (Array.isArray(d.seq) && lane === 'main') return d.seq as WorkflowStep[]
  const loop = d.loop as { do?: WorkflowStep[] } | undefined
  if (loop && Array.isArray(loop.do) && lane === 'main') return loop.do
  const ifDef = d.if as { then?: WorkflowStep[]; else?: WorkflowStep[] } | undefined
  if (ifDef) {
    if (lane === 'then' && Array.isArray(ifDef.then)) return ifDef.then
    if (lane === 'else' && Array.isArray(ifDef.else)) return ifDef.else
  }
  if (Array.isArray(d.auto) && lane === 'main') return d.auto as WorkflowStep[]
  return []
}

/** 容器直接子步骤总数（所有泳道合计） */
function childCount(step: WorkflowStep): number {
  const c = containerLanes(step)
  if (!c) return 0
  return c.lanes.reduce((n, l) => n + laneSteps(step, l.id).length, 0)
}

// ── 摘要文本 ──

function varRefText(r: VarRef | undefined): string {
  if (r === undefined || r === null) return ''
  if (typeof r === 'string') return r
  if (typeof r === 'object' && 'var' in r) return `{${r.var}}`
  return ''
}

function conditionSummary(cond: Condition | undefined): string {
  if (!cond || typeof cond !== 'object') return ''
  const c = cond as Record<string, unknown>
  const ops: [string, string][] = [
    ['equals', '='],
    ['not_equals', '≠'],
    ['contains', '包含'],
    ['starts_with', '前缀'],
    ['regex', '正则'],
    ['not_empty', '非空'],
    ['empty', '为空'],
    ['gt', '>'],
    ['lt', '<'],
    ['gte', '≥'],
    ['lte', '≤'],
    ['always', '恒'],
  ]
  for (const [key, sym] of ops) {
    if (!(key in c)) continue
    const v = c[key]
    if (Array.isArray(v))
      return `${varRefText(v[0] as VarRef)} ${sym} ${varRefText(v[1] as VarRef)}`.trim()
    return `${varRefText(v as VarRef)} ${sym}`.trim()
  }
  return ''
}

function containerSummary(step: WorkflowStep, kind: StepKind): string | undefined {
  const d = step.do as Record<string, unknown> | undefined
  if (!d) return undefined
  if (kind === 'loop') {
    const def = d.loop as {
      for_each?: { items?: VarRef; as?: string }
      repeat?: number
      until?: Condition
      max?: number
    }
    const parts: string[] = []
    if (def.for_each) {
      parts.push(`遍历 ${varRefText(def.for_each.items)} 作为 ${def.for_each.as || 'item'}`)
    } else if (def.repeat != null) {
      parts.push(`重复 ${def.repeat} 次`)
    } else if (def.until) {
      parts.push(`直到 ${conditionSummary(def.until)}`)
    }
    parts.push(`max ${def.max ?? 100}`)
    return parts.join(' · ')
  }
  if (kind === 'if') {
    const def = d.if as { condition?: Condition }
    const s = conditionSummary(def.condition)
    return s ? `条件 ${s}` : undefined
  }
  if (kind === 'wait') {
    return typeof d.wait === 'string' ? `等待 ${d.wait}` : undefined
  }
  return undefined
}

function onErrorLabel(step: WorkflowStep): string | undefined {
  const oe = step.on_error
  if (!oe || oe === 'abort') return undefined
  if (typeof oe === 'string') return oe
  if (typeof oe === 'object') {
    if ('retry' in oe) return 'retry'
    if ('allow_codes' in oe) return 'allow_codes'
  }
  return undefined
}

// ── 全树变量画像（遍历序）──

interface TreeProfile {
  /** 遍历序步骤画像列表 */
  order: StepVarProfile[]
  /** stepId → 遍历序下标 */
  indexOf: Map<string, number>
  /** stepId → 父容器 id（root 直接子步骤为 null） */
  parentOf: Map<string, string | null>
  /** var → 生产者 stepId 列表（遍历序） */
  producers: Map<string, string[]>
}

function buildTreeProfile(rootSteps: WorkflowStep[]): TreeProfile {
  const order: StepVarProfile[] = []
  const indexOf = new Map<string, number>()
  const parentOf = new Map<string, string | null>()
  const producers = new Map<string, string[]>()
  walkSteps(rootSteps, (step, parentId) => {
    const p = profileStep(step)
    indexOf.set(step.id, order.length)
    order.push(p)
    parentOf.set(step.id, parentId)
    if (p.produces) {
      const list = producers.get(p.produces) ?? []
      list.push(step.id)
      producers.set(p.produces, list)
    }
  })
  return { order, indexOf, parentOf, producers }
}

/** 遍历序最近的前序生产者（严格先于消费者） */
function nearestProducer(tree: TreeProfile, varName: string, consumerId: string): string | null {
  const list = tree.producers.get(varName)
  if (!list) return null
  const ci = tree.indexOf.get(consumerId) ?? -1
  let best: string | null = null
  for (const pid of list) {
    const pi = tree.indexOf.get(pid) ?? -1
    if (pi >= 0 && pi < ci) best = pid
  }
  return best
}

/** 遮蔽标记（1.4b/V10）：同一变量被多次 capture 时，除最后生产者外全部标记 */
function shadowedMap(tree: TreeProfile): Map<string, string> {
  const map = new Map<string, string>()
  for (const list of tree.producers.values()) {
    if (list.length < 2) continue
    const last = list[list.length - 1]
    for (let i = 0; i < list.length - 1; i++) map.set(list[i], last)
  }
  return map
}

// ── 循环 item_var 作用域 ──

interface LoopScope {
  loopStepId: string
  layerId: string
  itemVar: string
}

function loopScopeOf(step: WorkflowStep): LoopScope | null {
  const d = step.do as Record<string, unknown> | undefined
  const loop = d?.loop as { for_each?: { as?: string } } | undefined
  if (!loop) return null
  return { loopStepId: step.id, layerId: step.id, itemVar: loop.for_each?.as || 'item' }
}

/** var 是否由 consumer 的某个外层 loop 提供（item_var / _index） */
function enclosingLoopVar(
  consumerId: string,
  varName: string,
  parentOf: Map<string, string | null>,
  stepById: Map<string, WorkflowStep>,
): LoopScope | null {
  let cur = parentOf.get(consumerId) ?? null
  while (cur !== null) {
    const step = stepById.get(cur)
    if (step) {
      const scope = loopScopeOf(step)
      if (scope && (scope.itemVar === varName || varName === '_index')) return scope
    }
    cur = parentOf.get(cur) ?? null
  }
  return null
}

// ── 投影主流程 ──

export interface Projection {
  layers: Map<string, CanvasLayer>
  index: ProjectionIndex
}

/** 沿父链向上，找到 ancestor-or-self 中属于 targetSet 的节点 */
function aggregateTo(
  stepId: string,
  targetSet: Set<string>,
  parentOf: Map<string, string | null>,
): string | null {
  let cur: string | null = stepId
  while (cur !== null) {
    if (targetSet.has(cur)) return cur
    cur = parentOf.get(cur) ?? null
  }
  return null
}

export function projectWorkflow(ir: { steps: WorkflowStep[] }): Projection {
  const tree = buildTreeProfile(ir.steps)
  const shadowed = shadowedMap(tree)
  const stepById = new Map<string, WorkflowStep>()
  walkSteps(ir.steps, s => stepById.set(s.id, s))

  const layers = new Map<string, CanvasLayer>()
  const nodeById = new Map<string, CanvasNode>()
  const layerOf = new Map<string, string>()
  const containerIds = new Set<string>()
  let hasCustomNodes = false

  let fallbackSeq = 0
  const makeNode = (step: WorkflowStep, lane: LaneId): CanvasNode => {
    const kind = stepKind(step)
    if (kind === 'custom') hasCustomNodes = true
    const container = containerLanes(step)
    const node: CanvasNode = {
      // 旧格式步骤可能无 id（V13 只读场景）：给稳定占位 id 保证渲染键唯一
      id: step.id || `missing_id_${fallbackSeq++}`,
      kind,
      category: kind === 'custom' ? 'unknown' : container ? 'container' : 'leaf',
      name: step.name || step.id,
      lane,
      capture: typeof step.capture === 'string' && step.capture ? step.capture : undefined,
      onErrorLabel: onErrorLabel(step),
    }
    if (container) {
      node.childCount = childCount(step)
      node.containerSummary = containerSummary(step, kind)
      containerIds.add(step.id)
    }
    const shadow = shadowed.get(step.id)
    if (shadow) node.shadowedBy = shadow
    nodeById.set(step.id, node)
    return node
  }

  const projectLayer = (
    layerId: string,
    parentChain: string[],
    containerKind: CanvasLayer['containerKind'],
    lanes: Swimlane[],
    childrenOf: (lane: LaneId) => WorkflowStep[],
    hostStep: WorkflowStep | null,
  ): void => {
    const nodes: CanvasNode[] = []
    const edges: CanvasEdge[] = []
    const directIds = new Set<string>()

    // 本层直接子节点 + 顺序边（1.4a：同泳道 i → i+1）
    for (const lane of lanes) {
      const children = childrenOf(lane.id)
      for (let i = 0; i < children.length; i++) {
        const child = children[i]
        directIds.add(child.id)
        layerOf.set(child.id, layerId)
        nodes.push(makeNode(child, lane.id))
        if (i > 0) {
          edges.push({
            id: `seq:${children[i - 1].id}->${child.id}`,
            source: children[i - 1].id,
            target: child.id,
            kind: 'sequence',
          })
        }
      }
    }

    // 合成装饰（1.3）：loop 入口锚点 + 回环装饰边；if 条件块
    if (containerKind === 'loop' && hostStep) {
      const entryId = SYNTH.entry(layerId)
      const scope = loopScopeOf(hostStep)
      nodes.unshift({
        id: entryId,
        kind: 'loop',
        category: 'leaf',
        name: scope ? `入口 · ${scope.itemVar}` : '入口',
        lane: 'main',
        synthetic: 'entry',
        containerSummary: containerSummary(hostStep, 'loop'),
      })
      const mainChildren = childrenOf('main')
      if (mainChildren.length > 0) {
        edges.push({
          id: `seq:${entryId}->${mainChildren[0].id}`,
          source: entryId,
          target: mainChildren[0].id,
          kind: 'sequence',
        })
        // 回环边是装饰不是 IR 实体（1.2）
        const loopDef = (hostStep.do as Record<string, unknown>).loop as { max?: number }
        edges.push({
          id: SYNTH.loopback(hostStep.id),
          source: mainChildren[mainChildren.length - 1].id,
          target: entryId,
          kind: 'loopback',
          label: `max ${loopDef?.max ?? 100}`,
        })
      }
    }
    if (containerKind === 'if' && hostStep) {
      const condId = SYNTH.cond(hostStep.id)
      nodes.unshift({
        id: condId,
        kind: 'if',
        category: 'leaf',
        name: containerSummary(hostStep, 'if') || '条件',
        lane: 'then',
        synthetic: 'cond',
      })
      for (const lane of lanes) {
        const first = childrenOf(lane.id)[0]
        if (first) {
          edges.push({
            id: `cond:${condId}->${first.id}`,
            source: condId,
            target: first.id,
            kind: 'cond',
          })
        }
      }
    }

    // 数据边（1.4b）：消费端聚合到本层直接子节点；生产端同理，层外落锚点
    const dataEdgeKeys = new Set<string>()
    const pushDataEdge = (e: CanvasEdge) => {
      const key = `${e.source}→${e.target}#${e.label ?? ''}#${e.dangling ? 'd' : ''}`
      if (dataEdgeKeys.has(key)) return
      dataEdgeKeys.add(key)
      edges.push(e)
    }
    const externalAnchors = new Map<string, CanvasNode>()
    const anchorFor = (varName: string, producerId: string | null): CanvasNode => {
      let anchor = externalAnchors.get(varName)
      if (!anchor) {
        anchor = {
          id: SYNTH.external(layerId, varName),
          kind: 'tool',
          category: 'leaf',
          name: `外部 · ${varName}`,
          lane: lanes[0].id,
          synthetic: 'external',
          externalVar: varName,
          externalProducerId: producerId ?? undefined,
        }
        externalAnchors.set(varName, anchor)
        nodes.push(anchor)
      } else if (producerId && !anchor.externalProducerId) {
        anchor.externalProducerId = producerId
      }
      return anchor
    }

    for (const profile of tree.order) {
      const consumerNode = aggregateTo(profile.stepId, directIds, tree.parentOf)
      if (!consumerNode) continue // 不属于本层视野
      for (const cons of profile.consumes) {
        // 循环 item_var：仅在该 loop 自己的子层画入口锚点边
        const loopScope = enclosingLoopVar(profile.stepId, cons.varName, tree.parentOf, stepById)
        if (loopScope) {
          if (loopScope.layerId === layerId) {
            pushDataEdge({
              id: `data:${SYNTH.entry(layerId)}->${consumerNode}:${cons.varName}`,
              source: SYNTH.entry(layerId),
              target: consumerNode,
              kind: 'data',
              label: cons.varName,
              pipes: cons.pipes,
            })
          }
          continue
        }
        const producerId = nearestProducer(tree, cons.varName, profile.stepId)
        if (!producerId) {
          // 悬空引用 → 外部注入锚点黄虚边（warning 由校验层落 V4）
          const anchor = anchorFor(cons.varName, null)
          if (anchor.id !== consumerNode) {
            pushDataEdge({
              id: `data:${anchor.id}->${consumerNode}:${cons.varName}`,
              source: anchor.id,
              target: consumerNode,
              kind: 'external',
              label: cons.varName,
              pipes: cons.pipes,
              dangling: true,
            })
            const cn = nodeById.get(consumerNode)
            if (cn && cn.id === profile.stepId) {
              cn.danglingVars = [...(cn.danglingVars ?? []), cons.varName]
            }
          }
          continue
        }
        const producerNode = aggregateTo(producerId, directIds, tree.parentOf)
        if (!producerNode) {
          // 生产者在本层视野之外 → 层外来源锚点（2.4，点击跳转到生产者所在层）
          const anchor = anchorFor(cons.varName, producerId)
          if (anchor.id !== consumerNode) {
            pushDataEdge({
              id: `data:${anchor.id}->${consumerNode}:${cons.varName}`,
              source: anchor.id,
              target: consumerNode,
              kind: 'external',
              label: cons.varName,
              pipes: cons.pipes,
              producerStepId: producerId,
            })
          }
          continue
        }
        if (producerNode === consumerNode) continue // 同容器内部，属于其子层视野
        pushDataEdge({
          id: `data:${producerNode}->${consumerNode}:${cons.varName}`,
          source: producerNode,
          target: consumerNode,
          kind: 'data',
          label: cons.varName,
          pipes: cons.pipes,
          producerStepId: producerId,
        })
      }
    }

    // ── 出口数据边（1.8：子层内标注"产出去向层外"）──
    // 本层聚合生产者的变量被层外消费时，画 producer → 去向锚点
    const outAnchors = new Map<string, CanvasNode>()
    const outAnchorFor = (varName: string): CanvasNode => {
      let anchor = outAnchors.get(varName)
      if (!anchor) {
        anchor = {
          id: `${layerId}::out::${varName}`,
          kind: 'tool',
          category: 'leaf',
          name: `去向 · ${varName}`,
          lane: lanes[lanes.length - 1].id,
          synthetic: 'external',
          externalVar: varName,
        }
        outAnchors.set(varName, anchor)
        nodes.push(anchor)
      }
      return anchor
    }
    if (layerId !== 'root') {
      for (const profile of tree.order) {
        if (!profile.produces) continue
        const producerNode = aggregateTo(profile.stepId, directIds, tree.parentOf)
        if (!producerNode) continue // 生产者不在本层视野
        // 是否存在层外消费者（其最近前序生产者正是本步骤）
        const hasOutsideConsumer = tree.order.some(
          p2 =>
            p2.consumes.some(c => c.varName === profile.produces) &&
            nearestProducer(tree, profile.produces!, p2.stepId) === profile.stepId &&
            !aggregateTo(p2.stepId, directIds, tree.parentOf),
        )
        if (hasOutsideConsumer) {
          const anchor = outAnchorFor(profile.produces)
          pushDataEdge({
            id: `data:${producerNode}->${anchor.id}:${profile.produces}`,
            source: producerNode,
            target: anchor.id,
            kind: 'external',
            label: profile.produces,
            producerStepId: profile.stepId,
          })
        }
      }
    }

    layers.set(layerId, { layerId, parentChain, containerKind, swimlanes: lanes, nodes, edges })

    // 递归下钻容器子层
    for (const lane of lanes) {
      for (const child of childrenOf(lane.id)) {
        const c = containerLanes(child)
        if (!c) continue
        projectLayer(
          child.id,
          [...parentChain, layerId],
          c.kind,
          c.lanes,
          l => laneSteps(child, l),
          child,
        )
      }
    }
  }

  projectLayer('root', [], 'root', [{ id: 'main', title: '主流程' }], () => ir.steps, null)

  const index: ProjectionIndex = {
    parentOf: tree.parentOf,
    layerOf,
    nodeById,
    containerIds,
    hasCustomNodes,
  }
  return { layers, index }
}
