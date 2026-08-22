/**
 * layout.ts — dagre 自动布局 + sidecar 坐标合并（设计文档 1.7 / 第 2 章）
 *
 * - 每层独立布局（下钻模型天然分页，每层 <50 节点，dagre 同步耗时可忽略）
 * - if 双泳道：then/else 分别布局后横向并排
 * - sidecar 只存用户手动拖过的节点；未记录节点由 dagre 自动布局
 * - 位置是可选元数据，绝不写回 IR
 */

import dagre from '@dagrejs/dagre'
import type { CanvasLayer, CanvasLayoutSidecar, CanvasNode, LaneId } from './types'

export interface NodePos {
  x: number
  y: number
}

/** 节点尺寸（与 workflow-canvas.css 卡片宽度对齐） */
const SIZE = {
  leaf: { width: 200, height: 56 },
  container: { width: 220, height: 64 },
  unknown: { width: 200, height: 56 },
  anchor: { width: 150, height: 40 },
}

function nodeSize(n: CanvasNode): { width: number; height: number } {
  // 必须返回新对象：dagre.layout 会把 x/y/rank 原地写入 setNode 的值对象，
  // 共享 SIZE 常量引用会导致同类节点坐标互相覆盖（实测两容器节点重叠同坐标）
  if (n.synthetic) return { ...SIZE.anchor }
  return { ...SIZE[n.category] }
}

const LANE_GAP = 80

/** 布局方向：root 层同级步骤横向流（LR），容器子层树状纵向（TB） */
export type LayoutDir = 'LR' | 'TB'

/** 层布局方向唯一判定（CanvasPage 注入 Handle 方向与 layoutLayer 共用，防漂移） */
export function layerDir(layerId: string): LayoutDir {
  return layerId === 'root' ? 'LR' : 'TB'
}

/** 单泳道 dagre 布局（返回左上角坐标） */
function layoutLane(
  nodes: CanvasNode[],
  edges: CanvasLayer['edges'],
  dir: LayoutDir,
): Map<string, NodePos> {
  const g = new dagre.graphlib.Graph()
  g.setGraph({ rankdir: dir, nodesep: 48, ranksep: 72, marginx: 24, marginy: 24 })
  g.setDefaultEdgeLabel(() => ({}))
  const ids = new Set(nodes.map(n => n.id))
  for (const n of nodes) g.setNode(n.id, nodeSize(n))
  for (const e of edges) {
    if (ids.has(e.source) && ids.has(e.target)) g.setEdge(e.source, e.target)
  }
  dagre.layout(g)
  const out = new Map<string, NodePos>()
  for (const n of nodes) {
    const p = g.node(n.id)
    const s = nodeSize(n)
    out.set(n.id, { x: p.x - s.width / 2, y: p.y - s.height / 2 })
  }
  return out
}

/**
 * 整层布局：泳道内 dagre，多泳道横向并排；sidecar 手动坐标最后覆盖。
 * @param sidecarPos 本层已保存的手动坐标（layerId → pos 表已取出）
 * @param dir 布局方向，默认按 layerDir(layerId)：root=LR / 子层=TB
 */
export function layoutLayer(
  layer: CanvasLayer,
  sidecarPos?: Record<string, NodePos>,
  dir: LayoutDir = layerDir(layer.layerId),
): Map<string, NodePos> {
  const result = new Map<string, NodePos>()
  let laneOffsetX = 0

  for (const lane of layer.swimlanes) {
    const laneNodes = layer.nodes.filter(n => n.lane === lane.id)
    if (laneNodes.length === 0) continue
    // 泳道内边 = 两端都在本泳道的边
    const laneIds = new Set(laneNodes.map(n => n.id))
    const laneEdges = layer.edges.filter(e => laneIds.has(e.source) && laneIds.has(e.target))
    const pos = layoutLane(laneNodes, laneEdges, dir)
    let maxX = 0
    for (const [id, p] of pos) {
      result.set(id, { x: p.x + laneOffsetX, y: p.y })
      const w = laneNodes.find(n => n.id === id)
      maxX = Math.max(maxX, p.x + (w ? nodeSize(w).width : 0))
    }
    laneOffsetX += maxX + LANE_GAP
  }

  // sidecar 手动坐标覆盖（只存用户拖过的节点，1.7）
  if (sidecarPos) {
    for (const [id, p] of Object.entries(sidecarPos)) {
      if (result.has(id)) result.set(id, { x: p.x, y: p.y })
    }
  }
  return result
}

/** 读取 sidecar 中某层的坐标表 */
export function layerPosFromSidecar(
  sidecar: CanvasLayoutSidecar | null | undefined,
  layerId: string,
): Record<string, NodePos> | undefined {
  if (!sidecar || sidecar.version !== 1) return undefined
  return sidecar.layers?.[layerId]?.pos
}

/** 把用户拖拽坐标并入 sidecar（不可变更新，返回新对象） */
export function mergePosIntoSidecar(
  sidecar: CanvasLayoutSidecar | null | undefined,
  layerId: string,
  nodeId: string,
  pos: NodePos,
): CanvasLayoutSidecar {
  const base: CanvasLayoutSidecar =
    sidecar && sidecar.version === 1 ? sidecar : { version: 1, layers: {} }
  const layerEntry = base.layers[layerId] ?? { pos: {} }
  return {
    version: 1,
    layers: {
      ...base.layers,
      [layerId]: { pos: { ...layerEntry.pos, [nodeId]: { x: pos.x, y: pos.y } } },
    },
  }
}

/** 保存前清理：删除已不存在节点的残留 pos 条目（1.7） */
export function pruneSidecar(
  sidecar: CanvasLayoutSidecar,
  layers: Map<string, CanvasLayer>,
): CanvasLayoutSidecar {
  const out: CanvasLayoutSidecar = { version: 1, layers: {} }
  for (const [layerId, entry] of Object.entries(sidecar.layers)) {
    const layer = layers.get(layerId)
    if (!layer) continue
    const ids = new Set(layer.nodes.map(n => n.id))
    const pos: Record<string, NodePos> = {}
    for (const [nodeId, p] of Object.entries(entry.pos)) {
      if (ids.has(nodeId)) pos[nodeId] = p
    }
    out.layers[layerId] = { pos }
  }
  return out
}
