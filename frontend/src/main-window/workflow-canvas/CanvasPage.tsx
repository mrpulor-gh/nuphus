/**
 * CanvasPage.tsx — 工作流画布页面壳（设计文档 2.2）
 *
 * 职责：面包屑 + 工具栏 + ReactFlowProvider + 执行状态叠加 + 编辑编排。
 * 数据契约：
 * - IR 唯一真源：加载 wfGetRaw → 编辑缓冲 steps → IrEditOp → 重投影 → wf_save
 * - 运行中只读锁（RunStarted→RunCompleted）；Action::Custom 整树只读（V13/R1）
 * - 位置 sidecar 不污染 IR（wf_layout_get/save）
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  BackgroundVariant,
  Controls,
  applyNodeChanges,
  useReactFlow,
  MarkerType,
  type Node as FlowNode,
  type Edge as FlowEdge,
  type NodeChange,
  type Viewport,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import {
  X,
  Save,
  Play,
  Undo2,
  Redo2,
  Plus,
  CircleCheckBig,
  CornerUpLeft,
  ListChecks,
} from 'lucide-react'

import type { WorkflowStep, ToolSchema } from '../../core/types'
import {
  wfGetRaw,
  wfSave,
  wfValidate,
  wfRun,
  wfLayoutGet,
  wfLayoutSave,
  type ValidationReport,
} from '../lib/api'
import { useWorkflowGate } from '../lib/useWorkflowGate'
import type {
  WorkflowIR,
  CanvasLayoutSidecar,
  CanvasLayer,
  CanvasNode,
  LaneId,
  StepVisualStatus,
} from './types'
import { projectWorkflow, containerLanes, laneSteps, stepKind } from './projection'
import { playUiSound } from '../../ui/sound'
import {
  layoutLayer,
  layerDir,
  layerPosFromSidecar,
  mergePosIntoSidecar,
  pruneSidecar,
  type NodePos,
} from './layout'
import {
  applyOp,
  checkOp,
  IrEditHistory,
  newStep,
  cloneStepWithNewIds,
  collectIds,
  locateStep,
} from './irEdit'
import { validateIR, type Problem } from './validate'
import { subscribeRunStatus, aggregateContainerBadges, type RunStatusSnapshot } from './runStatus'
import { StepNode, NodeActionsContext } from './nodes/StepNode'
import { ContainerNode } from './nodes/ContainerNode'
import { LaneFrame } from './nodes/LaneFrame'
import { SequenceEdge, EDGE_INSERT_EVENT } from './edges/SequenceEdge'
import { DataEdge } from './edges/DataEdge'
import { Inspector, loadToolsOnce } from './Inspector'
import { ToolPalette, TOOL_DRAG_MIME } from './ToolPalette'
import { skeletonFromSchema } from './toolSkeleton'
import { ProblemsPanel } from './ProblemsPanel'
import { OutlinePanel } from './OutlinePanel'
import { IntentFormPanel } from './IntentFormPanel'
import type { IntentForm } from './intentTypes'
import { buildIntentTextTemplate } from './intentText'
import './workflow-canvas.css'

const nodeTypes = { step: StepNode, container: ContainerNode, lane: LaneFrame }
const edgeTypes = { sequence: SequenceEdge, data: DataEdge }

const ADDABLE_KINDS: { kind: string; desc: string }[] = [
  { kind: 'tool', desc: '调用工具（桌面/浏览器/文件等）' },
  { kind: 'seq', desc: '顺序容器，子步骤依次执行' },
  { kind: 'loop', desc: '循环容器，遍历或按次数重复' },
  { kind: 'if', desc: '条件分支（then/else 双泳道）' },
  { kind: 'call', desc: '调用另一个工作流' },
  { kind: 'wait', desc: '等待人工确认后继续' },
  { kind: 'chat', desc: 'Chat Agent 对话步骤' },
  { kind: 'script', desc: '执行脚本（Python 等）' },
  { kind: 'assert', desc: '断言校验，失败即中断' },
  { kind: 'mcp', desc: '调用 MCP server 工具' },
  { kind: 'sleep', desc: '延时等待指定秒数' },
  { kind: 'break', desc: '立即跳出当前循环' },
  { kind: 'continue', desc: '跳过本次循环进入下一轮' },
]

interface CanvasPageProps {
  workflowId: string
  onClose: () => void
}

/**
 * 点击式添加的插入位置：选中步骤之后（同层同泳道）；
 * 未选中 / 选中步骤不在当前层 → 首泳道末尾（与原「追加末尾」行为一致）
 */
function clickInsertion(
  steps: WorkflowStep[],
  layer: CanvasLayer,
  selectedId: string | null,
): { lane: LaneId; index: number } {
  const sel = selectedId ? locateStep(steps, selectedId) : null
  const afterSel = sel && sel.layerId === layer.layerId ? sel : null
  const lane = afterSel?.lane ?? layer.swimlanes[0]?.id ?? 'main'
  const index = afterSel
    ? afterSel.index + 1
    : layer.nodes.filter(n => n.lane === lane && !n.synthetic).length
  return { lane, index }
}

export function CanvasPage(props: CanvasPageProps) {
  return (
    <ReactFlowProvider>
      <CanvasInner {...props} />
    </ReactFlowProvider>
  )
}

/** 计算拖拽插入指示线的锚点（flow 坐标；按层主轴排序，index 为「移除自身后」的插入位） */
function insertHintFlowPos(
  layer: CanvasLayer,
  lane: LaneId,
  index: number,
  rf: ReturnType<typeof useReactFlow>,
): { x: number; y: number; horizontal: boolean } | null {
  const horizontal = layerDir(layer.layerId) === 'LR'
  const siblings = layer.nodes
    .filter(n => n.lane === lane && !n.synthetic)
    .map(n => {
      const fn = rf.getNodes().find(fn => fn.id === n.id)
      const pos = fn?.position ?? { x: 0, y: 0 }
      const w = fn?.measured?.width ?? (n.category === 'container' ? 220 : 200)
      const h = fn?.measured?.height ?? (n.category === 'container' ? 64 : 56)
      return { pos, w, h }
    })
    .sort((a, b) => (horizontal ? a.pos.x - b.pos.x : a.pos.y - b.pos.y))
  if (index <= 0) {
    const first = siblings[0]
    if (!first) return null
    return {
      x: horizontal ? first.pos.x - 14 : first.pos.x,
      y: horizontal ? first.pos.y : first.pos.y - 14,
      horizontal,
    }
  }
  const prev = siblings[index - 1]
  if (!prev) return null
  return {
    x: horizontal ? prev.pos.x + prev.w + 10 : prev.pos.x,
    y: horizontal ? prev.pos.y : prev.pos.y + prev.h + 10,
    horizontal,
  }
}

interface ConfirmState {
  message: string
  resolve: (ok: boolean) => void
}

function CanvasInner({ workflowId, onClose }: CanvasPageProps) {
  const rf = useReactFlow()
  // ── 全局执行闸门（大王铁律：任意执行态禁止启动工作流 / 录制）──
  // 画布已打开也不豁免：Agent 跑任务期间运行/录制入口必须锁住（本 wf 自身运行由
  // snapshot.running 只读锁覆盖，闸门轮询感知其它 agent/workflow 的执行态）。
  const gate = useWorkflowGate()
  const gateLocked = gate.locked
  const gateRefresh = gate.refresh
  const gateLockNotice =
    gate.reason === 'workflow' ? '工作流正在执行中，暂不可用！' : '当前有任务执行中，暂不可用！'
  const [ir, setIr] = useState<WorkflowIR | null>(null)
  const [steps, setSteps] = useState<WorkflowStep[] | null>(null)
  const [dirty, setDirty] = useState(false)
  const [layerId, setLayerId] = useState('root')
  const [sidecar, setSidecar] = useState<CanvasLayoutSidecar | null>(null)
  const [snapshot, setSnapshot] = useState<RunStatusSnapshot>({
    steps: new Map(),
    outputs: new Map(),
    running: false,
  })
  // ── 顶部运行态派生（4.2 + Error→fresh 从头 / Paused→续跑 三态拆分）──
  // 供 runWorkflow（按钮/R 快捷键）与顶部按钮/横幅共用；runWorkflow 依赖数组据此更新。
  const lastRun = ir?.run_history?.[0]
  const lastRunError =
    !!lastRun &&
    !snapshot.running &&
    typeof lastRun.status === 'object' &&
    lastRun.status !== null &&
    'Error' in lastRun.status
  const lastRunPaused = !!lastRun && !snapshot.running && lastRun.status === 'Paused'
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [inspectorOpen, setInspectorOpen] = useState(false)
  /** 新建 tool 节点后待聚焦的工具输入框（Inspector 消费一次） */
  const [toolFocusId, setToolFocusId] = useState<string | null>(null)
  const [problems, setProblems] = useState<Problem[]>([])
  const [backendReport, setBackendReport] = useState<ValidationReport | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<ConfirmState | null>(null)
  /** 意图表单弹层（画布顶部「意图表单」入口；不启动任何录制会话） */
  const [intentFormOpen, setIntentFormOpen] = useState(false)
  // ── 工作流重命名（wfc-title 双击/铅笔进入；提交走 save(name)，连同当前步骤一并保存）──
  const [nameEditing, setNameEditing] = useState(false)
  const [nameInput, setNameInput] = useState('')
  // ── 拖拽插入指示（阶段 3：顺序重排改为明确插入，拖动不再自动重排）──
  const [dragInsert, setDragInsert] = useState<{ lane: LaneId; index: number } | null>(null)
  const dragInsertRef = useRef<{ lane: LaneId; index: number } | null>(null)
  const [dragInsertScreen, setDragInsertScreen] = useState<{
    x: number
    y: number
    horizontal: boolean
  } | null>(null)
  const [addMenuOpen, setAddMenuOpen] = useState(false)
  // 添加菜单：点击外部收起（Escape 见键盘表；项内点击见 addStep）
  const addWrapRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!addMenuOpen) return
    const onDown = (e: PointerEvent) => {
      if (addWrapRef.current && !addWrapRef.current.contains(e.target as Node)) {
        setAddMenuOpen(false)
      }
    }
    window.addEventListener('pointerdown', onDown, true)
    return () => window.removeEventListener('pointerdown', onDown, true)
  }, [addMenuOpen])
  const [flowNodes, setFlowNodes] = useState<FlowNode[]>([])
  const [flashId, setFlashId] = useState<string | null>(null)
  /** 连线中点「在此插入」菜单（edgeId = layer.edges 的 CanvasEdge id；x/y = 屏幕坐标） */
  const [edgeInsert, setEdgeInsert] = useState<{ edgeId: string; x: number; y: number } | null>(
    null,
  )
  const edgeMenuWrapRef = useRef<HTMLDivElement>(null)
  // 连线插入菜单：点击外部收起（Escape 见键盘表；项内点击见 insertAtEdge）
  useEffect(() => {
    if (!edgeInsert) return
    const onDown = (e: PointerEvent) => {
      if (edgeMenuWrapRef.current && !edgeMenuWrapRef.current.contains(e.target as Node)) {
        setEdgeInsert(null)
      }
    }
    window.addEventListener('pointerdown', onDown, true)
    return () => window.removeEventListener('pointerdown', onDown, true)
  }, [edgeInsert])

  const flashTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** 新节点闪光高亮（连线插入 / 录制入画布）；复用既有 wfc-flash CSS（0.6s × 2） */
  const flashStep = useCallback((id: string) => {
    if (flashTimerRef.current) clearTimeout(flashTimerRef.current)
    setFlashId(id)
    flashTimerRef.current = setTimeout(() => setFlashId(null), 1500)
  }, [])

  /**
   * 新节点入视口（大王反馈#7 根因修复）：applyEdit → setSteps → 投影 → flowNodes → ReactFlow store
   * 是多级 effect 链，旧代码只等一帧 rAF 就 fitView({nodes:[{id}]})——此时节点多半还没进 store，
   * 或已进但未 measure（宽度 0）会被 fitView 过滤 → 视口不跟随新步骤（表现「预览位置不正确」）。
   * 这里轮询等待节点进入 store 后按节点中心 setCenter（不依赖 measure 完成，默认尺寸兜底），
   * 保证新增节点真实出现在可视区中央附近。
   */
  const focusStepFlow = useCallback(
    (stepId: string) => {
      const attempt = (tries: number) => {
        const node = rf.getNodes().find(n => n.id === stepId)
        if (node && Number.isFinite(node.position.x)) {
          const w = node.measured?.width ?? 200
          const h = node.measured?.height ?? 56
          void rf.setCenter(node.position.x + w / 2, node.position.y + h / 2, {
            zoom: Math.max(rf.getZoom(), 0.8),
            duration: 200,
          })
          return
        }
        if (tries < 12) requestAnimationFrame(() => attempt(tries + 1))
      }
      requestAnimationFrame(() => attempt(0))
    },
    [rf],
  )

  const historyRef = useRef(new IrEditHistory())
  const viewportMem = useRef(new Map<string, Viewport>())
  const breadcrumbRef = useRef<HTMLDivElement | null>(null)
  const sidecarSaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const stepsRef = useRef<WorkflowStep[] | null>(null)
  stepsRef.current = steps

  // ── 加载 ──
  useEffect(() => {
    let alive = true
    ;(async () => {
      const raw = (await wfGetRaw(workflowId)) as unknown as WorkflowIR | null
      if (!alive) return
      if (!raw) {
        setNotice('工作流不存在或已被删除')
        return
      }
      setIr(raw)
      setSteps(structuredClone(raw.steps ?? []))
      const layout = (await wfLayoutGet(workflowId).catch(
        () => null,
      )) as unknown as CanvasLayoutSidecar | null
      if (alive) setSidecar(layout)
    })()
    return () => {
      alive = false
    }
  }, [workflowId])

  // ── 执行状态订阅（2.3：独立 listener，会话边界由 tracker 保证）──
  useEffect(() => subscribeRunStatus(workflowId, setSnapshot), [workflowId])

  // ── 终态对账：RunCompleted 后重拉 run_history 作权威回放源 ──
  const wasRunning = useRef(false)
  useEffect(() => {
    if (wasRunning.current && !snapshot.running) {
      wfGetRaw(workflowId).then(raw => {
        if (raw) setIr(raw as unknown as WorkflowIR)
      })
    }
    wasRunning.current = snapshot.running
  }, [snapshot.running, workflowId])

  // ── 投影（IR → 图层）──
  const projection = useMemo(() => (steps ? projectWorkflow({ steps }) : null), [steps])
  const layer = projection?.layers.get(layerId) ?? null

  // ── 只读判定：运行中锁（1.6）+ 旧格式整树只读（V13/R1）──
  const readOnly = snapshot.running || !!projection?.index.hasCustomNodes
  // 只读翻转（运行开始等）时自动收起残留的连线插入菜单
  useEffect(() => {
    if (readOnly) setEdgeInsert(null)
  }, [readOnly])

  // ── L2 实时校验（防抖 300ms，3.4）──
  useEffect(() => {
    if (!steps) return
    const timer = setTimeout(() => {
      setProblems(validateIR(steps, { runHistory: ir?.run_history }))
    }, 300)
    return () => clearTimeout(timer)
  }, [steps, ir?.run_history])

  // ── 节点问题级别映射（error 优先）──
  const problemByStep = useMemo(() => {
    const m = new Map<string, 'error' | 'warning'>()
    for (const p of problems) {
      if (!p.stepId) continue
      if (p.level === 'error' || !m.has(p.stepId)) m.set(p.stepId, p.level)
    }
    return m
  }, [problems])

  // ── run_history 回放基线（非运行时的持久着色，4.2）──
  const historyStatus = useMemo(() => {
    const m = new Map<string, StepVisualStatus>()
    const last = ir?.run_history?.[0]
    if (!last || snapshot.running) return m
    const recs = Array.isArray(last.steps)
      ? (last.steps as { step_id: string; status: unknown }[])
      : []
    for (const r of recs) {
      if (r.status === 'Success') m.set(r.step_id, { state: 'success' })
      else if (r.status === 'Skipped') m.set(r.step_id, { state: 'skipped' })
      else if (
        typeof r.status === 'object' &&
        r.status !== null &&
        'Error' in (r.status as object)
      ) {
        m.set(r.step_id, {
          state: 'error',
          message: String((r.status as { Error: unknown }).Error),
        })
      }
    }
    return m
  }, [ir?.run_history, snapshot.running])

  const badges = useMemo(
    () => (projection ? aggregateContainerBadges(projection.index, snapshot.steps) : new Map()),
    [projection, snapshot.steps],
  )

  // ── 大纲状态合成：run_history 基线 ← 实时事件覆盖；容器子树 running/error 上浮（红点优先，
  //    语义对齐 aggregateContainerBadges，success/skipped/paused 不上浮）──
  const outlineStatus = useMemo(() => {
    const m = new Map<string, StepVisualStatus['state']>()
    if (!steps) return m
    const merged = new Map<string, StepVisualStatus>()
    for (const [id, st] of historyStatus) merged.set(id, st)
    for (const [id, st] of snapshot.steps) merged.set(id, st)
    const visit = (list: WorkflowStep[]): { running: boolean; error: boolean } => {
      let running = false
      let error = false
      for (const step of list) {
        const c = containerLanes(step)
        const child = c
          ? visit(c.lanes.flatMap(l => laneSteps(step, l.id)))
          : { running: false, error: false }
        const own = merged.get(step.id)?.state
        const state: StepVisualStatus['state'] | undefined = child.error
          ? 'error'
          : child.running
            ? own === 'error'
              ? 'error'
              : 'running'
            : own
        if (state) m.set(step.id, state)
        if (state === 'error') error = true
        else if (state === 'running' || state === 'retrying') running = true
      }
      return { running, error }
    }
    visit(steps)
    return m
  }, [steps, snapshot.steps, historyStatus])

  // ── 层 → React Flow 图 ──
  const laneFrames = useMemo(() => {
    if (!layer || layer.swimlanes.length < 2) return []
    const pos = layoutLayer(layer, layerPosFromSidecar(sidecar, layer.layerId))
    const frames: {
      id: string
      title: string
      x: number
      y: number
      width: number
      height: number
      empty: boolean
      lane: LaneId
    }[] = []
    for (const lane of layer.swimlanes) {
      const laneNodes = layer.nodes.filter(n => n.lane === lane.id && !n.synthetic)
      const all = layer.nodes.filter(n => n.lane === lane.id)
      if (all.length === 0) continue
      let minX = Infinity,
        minY = Infinity,
        maxX = -Infinity,
        maxY = -Infinity
      for (const n of all) {
        const p = pos.get(n.id)
        if (!p) continue
        minX = Math.min(minX, p.x)
        minY = Math.min(minY, p.y)
        maxX = Math.max(maxX, p.x + 220)
        maxY = Math.max(maxY, p.y + 64)
      }
      if (!Number.isFinite(minX)) continue
      frames.push({
        id: `lane:${layer.layerId}:${lane.id}`,
        title: lane.title,
        x: minX - 28,
        y: minY - 44,
        width: maxX - minX + 56,
        height: maxY - minY + 72,
        empty: laneNodes.length === 0,
        lane: lane.id,
      })
    }
    // 空泳道占位（1.2：else 为空时泳道显示空态，可拖入）——无节点可包围盒，借邻位放置
    for (const lane of layer.swimlanes) {
      if (frames.some(f => f.lane === lane.id)) continue
      const anchor = frames[0]
      const baseX = anchor ? anchor.x + anchor.width + 40 : 0
      frames.push({
        id: `lane:${layer.layerId}:${lane.id}`,
        title: lane.title,
        x: baseX,
        y: anchor ? anchor.y : 0,
        width: 240,
        height: 160,
        empty: true,
        lane: lane.id,
      })
    }
    return frames
  }, [layer, sidecar])

  useEffect(() => {
    if (!layer) {
      setFlowNodes([])
      return
    }
    const pos = layoutLayer(layer, layerPosFromSidecar(sidecar, layer.layerId))
    const dir = layerDir(layer.layerId)
    const nodes: FlowNode[] = []
    for (const f of laneFrames) {
      nodes.push({
        id: f.id,
        type: 'lane',
        position: { x: f.x, y: f.y },
        data: { title: f.title, width: f.width, height: f.height, empty: f.empty },
        selectable: false,
        draggable: false,
        zIndex: -1,
      })
    }
    for (const n of layer.nodes) {
      const p = pos.get(n.id) ?? { x: 0, y: 0 }
      const status = snapshot.steps.get(n.id) ?? historyStatus.get(n.id)
      // 容器子步骤轻量预览：取子层非合成节点的 {name, kind}（最多 8 条 + 总数），纯 CSS hover 展示
      let childrenPreview: { total: number; items: { name: string; kind: string }[] } | undefined
      if (n.category === 'container' && !n.synthetic) {
        const childNodes = (projection?.layers.get(n.id)?.nodes ?? []).filter(c => !c.synthetic)
        if (childNodes.length > 0) {
          childrenPreview = {
            total: childNodes.length,
            items: childNodes.slice(0, 8).map(c => ({ name: c.name, kind: c.kind })),
          }
        }
      }
      nodes.push({
        id: n.id,
        type: n.category === 'container' ? 'container' : 'step',
        position: p,
        data: {
          canvas: n,
          status,
          problem: problemByStep.get(n.id),
          badge: badges.get(n.id),
          dir,
          childrenPreview,
        },
        selected: selectedId === n.id,
        draggable: !readOnly && !n.synthetic,
        selectable: true,
        className: flashId === n.id ? 'wfc-flash' : undefined,
      })
    }
    setFlowNodes(nodes)
  }, [
    layer,
    sidecar,
    snapshot.steps,
    historyStatus,
    problemByStep,
    badges,
    selectedId,
    readOnly,
    laneFrames,
    flashId,
    projection,
  ])

  // ── 加载完成视口适配 ──
  // ReactFlow 的 fitView 仅在初始挂载（此时 flowNodes 为空，适配的是空图）与 prop 值变化时触发；
  // 数据加载完成后 flowNodes 更新为真实节点时不会自动重新适配。若节点被手动拖到视口外
  // （边缘 / 负坐标均合法），重开画布时节点落在视口外，画布看似空白。
  // 这里在节点首次就绪后主动 fitView 一次，自适应任意坐标；之后不再触发，避免干扰用户操作。
  const didInitialFit = useRef(false)
  useEffect(() => {
    if (!flowNodes.length || didInitialFit.current) return
    didInitialFit.current = true
    requestAnimationFrame(() => {
      void rf.fitView({ padding: 0.2 })
    })
  }, [flowNodes, rf])

  const flowEdges: FlowEdge[] = useMemo(() => {
    if (!layer) return []
    return layer.edges.map(e => {
      if (e.kind === 'data' || e.kind === 'external') {
        return {
          id: e.id,
          source: e.source,
          target: e.target,
          type: 'data',
          data: {
            label: e.label,
            pipes: e.pipes,
            dangling: e.dangling,
            external: e.kind === 'external',
            producerStepId: e.producerStepId,
          },
          selectable: false,
        }
      }
      const decorative = e.kind === 'loopback' || e.kind === 'cond'
      return {
        id: e.id,
        source: e.source,
        target: e.target,
        type: 'sequence',
        // readOnly 透传给 SequenceEdge：只读画布不渲染中点插入手柄（运行中/旧格式只读）
        data: { decorative, label: e.label, readOnly },
        selectable: false,
        markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14 },
      }
    })
  }, [layer, readOnly])

  // ── 视口记忆（2.4）──
  const switchLayer = useCallback(
    (next: string) => {
      if (next === layerId) return
      viewportMem.current.set(layerId, rf.getViewport())
      setLayerId(next)
      setSelectedId(null)
      setInspectorOpen(false)
      setEdgeInsert(null) // 连线插入菜单绑定旧层边 id，切层即失效
      requestAnimationFrame(() => {
        const mem = viewportMem.current.get(next)
        if (mem) void rf.setViewport(mem)
        else void rf.fitView({ padding: 0.2 })
      })
    },
    [layerId, rf],
  )

  // ── 编辑编排 ──
  const askConfirm = useCallback(
    (message: string) =>
      new Promise<boolean>(resolve => {
        setConfirm({ message, resolve })
      }),
    [],
  )

  // D4：关闭 Inspector 时若有空名/必填缺失字段给确认提醒（不推翻即时提交模型）
  const closeInspector = useCallback(() => {
    // 已有确认弹窗在等待用户决定时不重复弹（Esc 连按/点击竞争场景）
    if (confirm) return
    const cur = stepsRef.current
    const step = selectedId && cur ? locateStep(cur, selectedId)?.step : null
    if (step && !readOnly && !step.name.trim()) {
      void askConfirm('该步骤尚未命名，空名称将无法通过保存/运行校验。仍要关闭编辑？').then(ok => {
        if (ok) setInspectorOpen(false)
      })
      return
    }
    setInspectorOpen(false)
  }, [selectedId, readOnly, askConfirm, confirm])

  // ── 关闭守卫：有未保存修改时二次确认（复用 askConfirm 弹窗）──
  const handleClose = useCallback(() => {
    if (!dirty) {
      onClose()
      return
    }
    void askConfirm('有未保存的修改，关闭将丢弃（如需保留请先点「保存」）。确认关闭？').then(ok => {
      if (ok) onClose()
    })
  }, [dirty, askConfirm, onClose])

  const applyEdit = useCallback(
    async (op: Parameters<typeof applyOp>[1]) => {
      const cur = stepsRef.current
      if (!cur || readOnly) return false
      const check = checkOp(cur, op, { runHistory: ir?.run_history })
      if (!check.ok) {
        setNotice(check.reason ?? '操作被拒绝')
        return false
      }
      if (check.confirm) {
        const ok = await askConfirm(check.confirm)
        if (!ok) return false
      }
      historyRef.current.push(cur)
      setSteps(applyOp(cur, op))
      setDirty(true)
      setNotice(null)
      return true
    },
    [readOnly, ir?.run_history, askConfirm],
  )

  const undo = useCallback(() => {
    const cur = stepsRef.current
    if (!cur || readOnly) return
    const prev = historyRef.current.undo(cur)
    if (prev) {
      setSteps(prev)
      setDirty(true)
    }
  }, [readOnly])

  const redo = useCallback(() => {
    const cur = stepsRef.current
    if (!cur || readOnly) return
    const next = historyRef.current.redo(cur)
    if (next) {
      setSteps(next)
      setDirty(true)
    }
  }, [readOnly])

  // ── 保存（1.6：wf_save 后端强制校验，errors 阻断回 ProblemsPanel）
  // nameOverride：重命名时传入新名（与当前步骤编辑一并保存；空/省略则沿用原名）──
  const save = useCallback(
    async (nameOverride?: string) => {
      const cur = stepsRef.current
      if (!cur || !ir) return
      const name = nameOverride?.trim()
      const payload = { ...ir, steps: cur, ...(name ? { name } : {}) }
      try {
        const resp = await wfSave(payload)
        if (!resp) {
          setNotice('保存失败：后端无响应')
          return
        }
        setBackendReport(resp.report)
        if (resp.saved) {
          setDirty(false)
          setNotice(name ? `已保存，工作流已重命名为「${name}」` : '已保存')
          setIr(payload)
        } else {
          setNotice('保存被阻断：存在校验错误，详见问题面板「后端校验」')
        }
      } catch (e) {
        setNotice(`保存失败：${String(e)}`)
      }
    },
    [ir],
  )

  const runCheck = useCallback(async () => {
    const cur = stepsRef.current
    if (!cur || !ir) return
    try {
      const report = await wfValidate({ ...ir, steps: cur })
      if (!report) {
        setNotice('校验失败：后端无响应')
        return
      }
      setBackendReport(report)
      setNotice(report.passed ? '后端校验通过' : '后端校验发现错误，详见问题面板')
    } catch (e) {
      setNotice(`校验失败：${String(e)}`)
    }
  }, [ir])

  const runWorkflow = useCallback(async () => {
    if (snapshot.running) return
    if (dirty) {
      setNotice('有未保存的编辑，请先保存（Ctrl+S）再运行')
      return
    }
    // 闸门点击级复核（轮询窗口内竞态收口；后端 execute_workflow 另有兜底）
    const cur = await gateRefresh()
    if (cur.locked) {
      setNotice(
        cur.reason === 'workflow' ? '工作流正在执行中，暂不可用！' : '当前有任务执行中，暂不可用！',
      )
      return
    }
    try {
      // Error → fresh 从头完整执行（不再续连，避免失败死循环）；
      // Paused / 无历史 → fresh=false 断点续连（completed_ids 为空时从头等价）。
      await wfRun(workflowId, lastRunError)
    } catch (e) {
      setNotice(`启动失败：${String(e)}`)
    }
  }, [workflowId, dirty, snapshot.running, gateRefresh, lastRunError])

  // ── sidecar 持久化（防抖）──
  const persistSidecar = useCallback(
    (next: CanvasLayoutSidecar) => {
      setSidecar(next)
      if (sidecarSaveTimer.current) clearTimeout(sidecarSaveTimer.current)
      sidecarSaveTimer.current = setTimeout(() => {
        const layers = projection?.layers
        const cleaned = layers ? pruneSidecar(next, layers) : next
        void wfLayoutSave(workflowId, cleaned).catch(() => {})
      }, 600)
    },
    [workflowId, projection],
  )

  // ── 拖拽：重排 / 跨泳道 / 入容器 / 到面包屑（2.5）──
  const onNodesChange = useCallback((changes: NodeChange[]) => {
    setFlowNodes(ns => applyNodeChanges(changes, ns))
    if (changes.some(c => c.type === 'select')) {
      const sel = changes.find(c => c.type === 'select' && c.selected)
      if (sel && sel.type === 'select') setSelectedId(sel.id)
    }
  }, [])

  // ── 拖拽中：检测同层插入目标（阶段 3：顺序重排改为「明确越过节点中心」才插入）──
  // 拖动节点中心越过某兄弟节点中心 → 标记插入位置；未越过任何中心 → 仅改位置不动顺序。
  const onNodeDrag = useCallback(
    (_: MouseEvent | TouchEvent, node: FlowNode) => {
      const cur = stepsRef.current
      if (!cur || !layer || readOnly) return
      const canvasNode = node.data.canvas as CanvasNode | undefined
      if (!canvasNode || canvasNode.synthetic) return
      const horizontal = layerDir(layer.layerId) === 'LR'
      const selfAxis =
        (horizontal ? node.position.x : node.position.y) +
        (horizontal ? (node.measured?.width ?? 200) / 2 : (node.measured?.height ?? 56) / 2)
      const siblings = layer.nodes
        .filter(n => n.lane === canvasNode.lane && !n.synthetic && n.id !== node.id)
        .map(n => {
          const fn = rf.getNodes().find(fn => fn.id === n.id)
          const pos = fn?.position ?? { x: 0, y: 0 }
          const w = fn?.measured?.width ?? (n.category === 'container' ? 220 : 200)
          const h = fn?.measured?.height ?? (n.category === 'container' ? 64 : 56)
          return { pos, center: (horizontal ? pos.x : pos.y) + (horizontal ? w : h) / 2 }
        })
        .sort((a, b) => (horizontal ? a.pos.x - b.pos.x : a.pos.y - b.pos.y))
      const index = siblings.filter(s => s.center < selfAxis).length
      const currentIndex = layer.nodes
        .filter(n => n.lane === canvasNode.lane && !n.synthetic)
        .findIndex(n => n.id === node.id)
      const next = { lane: canvasNode.lane, index }
      dragInsertRef.current = index !== currentIndex ? next : null
      setDragInsert(index !== currentIndex ? next : null)
      // 指示线屏幕坐标（画布容器内绝对定位）
      if (dragInsertRef.current) {
        const hintFlow = insertHintFlowPos(layer, next.lane, next.index, rf)
        if (hintFlow) {
          const vp = rf.getViewport()
          setDragInsertScreen({
            x: hintFlow.x * vp.zoom + vp.x,
            y: hintFlow.y * vp.zoom + vp.y,
            horizontal: hintFlow.horizontal,
          })
        } else {
          setDragInsertScreen(null)
        }
      } else {
        setDragInsertScreen(null)
      }
    },
    [layer, readOnly, rf],
  )

  const onNodeDragStop = useCallback(
    async (event: MouseEvent | TouchEvent, node: FlowNode) => {
      const cur = stepsRef.current
      if (!cur || !layer || !projection) return
      const canvasNode = node.data.canvas as CanvasNode | undefined
      if (!canvasNode || canvasNode.synthetic) return
      // 触屏/鼠标统一取点（dragStop 时 touches 已空，用 changedTouches）
      const pt =
        'changedTouches' in event ? (event.changedTouches[0] ?? { clientX: 0, clientY: 0 }) : event

      // 拖动结束：立即清理插入指示 UI（ref 缓存保留至结构判断后）
      setDragInsert(null)
      setDragInsertScreen(null)

      // 位置入 sidecar（无论是否发生结构移动）
      const withPos = mergePosIntoSidecar(sidecar, layer.layerId, node.id, {
        x: node.position.x,
        y: node.position.y,
      })
      persistSidecar(withPos)

      if (readOnly) return

      // 1) 面包屑 drop（拖出到上层）
      if (breadcrumbRef.current) {
        const items = breadcrumbRef.current.querySelectorAll<HTMLElement>('[data-layer-id]')
        for (const el of items) {
          const r = el.getBoundingClientRect()
          if (
            pt.clientX >= r.left &&
            pt.clientX <= r.right &&
            pt.clientY >= r.top &&
            pt.clientY <= r.bottom
          ) {
            const targetLayer = el.dataset.layerId!
            if (targetLayer !== layer.layerId) {
              const targetCanvasLayer = projection.layers.get(targetLayer)
              const lane: LaneId = targetCanvasLayer?.swimlanes[0]?.id ?? 'main'
              const siblings =
                targetLayer === 'root'
                  ? cur
                  : (() => {
                      const loc = locateStep(cur, targetLayer)
                      return loc ? laneSteps(loc.step, lane) : []
                    })()
              await applyEdit({
                op: 'move_step',
                stepId: node.id,
                to: { parent: { layerId: targetLayer }, lane, index: siblings.length },
              })
              return
            }
          }
        }
      }

      // 2) 容器节点 drop（拖入容器）
      const dropPoint = rf.screenToFlowPosition({ x: pt.clientX, y: pt.clientY })
      const allNodes = rf.getNodes()
      for (const other of allNodes) {
        if (other.id === node.id || other.type !== 'container') continue
        const w = other.measured?.width ?? 220
        const h = other.measured?.height ?? 64
        if (
          dropPoint.x >= other.position.x &&
          dropPoint.x <= other.position.x + w &&
          dropPoint.y >= other.position.y &&
          dropPoint.y <= other.position.y + h
        ) {
          const loc = locateStep(cur, other.id)
          if (loc) {
            const c = containerLanes(loc.step)
            const lane = c?.lanes[0]?.id ?? 'main'
            await applyEdit({
              op: 'move_step',
              stepId: node.id,
              to: { parent: { layerId: other.id }, lane, index: laneSteps(loc.step, lane).length },
            })
          }
          return
        }
      }

      // 3) 同层 / 跨泳道：
      //    - 跨泳道：落点命中另一泳道框 → move_step（明确目标，按真实中心推断目标泳道内插入位）
      //    - 同层：只接受拖动中检测到的插入目标（onNodeDrag 设置 dragInsertRef）→ reorder；
      //      未越过任何节点中心 → 仅更新位置（顺序保持稳定，连线不乱）
      const horizontal = layerDir(layer.layerId) === 'LR'
      let targetLane: LaneId = canvasNode.lane
      if (laneFrames.length > 0) {
        for (const f of laneFrames) {
          if (dropPoint.x >= f.x && dropPoint.x <= f.x + f.width) {
            const laneId = f.id.split(':').pop() as LaneId
            targetLane = laneId
            break
          }
        }
      }
      // 目标泳道内的插入位（按节点实际中心推断，真实尺寸）
      const laneSiblings = layer.nodes
        .filter(n => n.lane === targetLane && !n.synthetic && n.id !== node.id)
        .map(n => {
          const fn = allNodes.find(fn => fn.id === n.id)
          const pos = fn?.position ?? { x: 0, y: 0 }
          const w = fn?.measured?.width ?? (n.category === 'container' ? 220 : 200)
          const h = fn?.measured?.height ?? (n.category === 'container' ? 64 : 56)
          return { id: n.id, pos, center: horizontal ? pos.x + w / 2 : pos.y + h / 2 }
        })
        .sort((a, b) => (horizontal ? a.pos.x - b.pos.x : a.pos.y - b.pos.y))
      const dropIndex = horizontal
        ? laneSiblings.filter(s => s.center < dropPoint.x).length
        : laneSiblings.filter(s => s.center < dropPoint.y).length

      if (targetLane !== canvasNode.lane) {
        dragInsertRef.current = null
        await applyEdit({
          op: 'move_step',
          stepId: node.id,
          to: { parent: { layerId: layer.layerId }, lane: targetLane, index: dropIndex },
        })
        return
      }

      // 同层：仅当拖动中有明确插入目标时重排，否则顺序保持稳定
      const insert = dragInsertRef.current
      dragInsertRef.current = null
      if (insert && insert.lane === canvasNode.lane) {
        const currentIndex = layer.nodes
          .filter(n => n.lane === canvasNode.lane && !n.synthetic)
          .findIndex(n => n.id === node.id)
        if (insert.index !== currentIndex) {
          await applyEdit({ op: 'reorder', stepId: node.id, newIndex: insert.index })
        }
      }
    },
    [layer, projection, sidecar, readOnly, rf, laneFrames, applyEdit, persistSidecar],
  )

  // ── 节点选择 / 下钻 / 锚点跳转 ──
  const onNodeClick = useCallback(
    (_: React.MouseEvent, node: FlowNode) => {
      if (node.type === 'lane') return
      const canvasNode = node.data.canvas as CanvasNode | undefined
      if (canvasNode?.synthetic === 'external' && canvasNode.externalProducerId && projection) {
        const targetLayer = projection.index.layerOf.get(canvasNode.externalProducerId)
        if (targetLayer) {
          switchLayer(targetLayer)
          setSelectedId(canvasNode.externalProducerId)
          setFlashId(canvasNode.externalProducerId)
          setTimeout(() => setFlashId(null), 1200)
        }
        return
      }
      setSelectedId(node.id)
    },
    [projection, switchLayer],
  )

  const onNodeDoubleClick = useCallback(
    (_: React.MouseEvent, node: FlowNode) => {
      const canvasNode = node.data.canvas as CanvasNode | undefined
      if (!canvasNode || canvasNode.synthetic) return
      if (canvasNode.category === 'container') switchLayer(node.id)
      else {
        setSelectedId(node.id)
        setInspectorOpen(true)
      }
    },
    [switchLayer],
  )

  // ── 删除 / 复制 / 添加 ──
  // id 可选：传入则操作指定节点（hover 操作按钮），缺省操作当前选中节点（快捷键）
  const deleteSelected = useCallback(
    async (id?: string) => {
      const targetId = id ?? selectedId
      const cur = stepsRef.current
      if (!cur || !targetId || readOnly) return
      const loc = locateStep(cur, targetId)
      if (!loc) return
      if (containerLanes(loc.step)) {
        const ok = await askConfirm(
          `「${loc.step.name || loc.step.id}」是容器，删除将级联删除全部子步骤。确认删除？`,
        )
        if (!ok) return
      }
      const ok = await applyEdit({ op: 'remove_step', stepId: targetId })
      setSelectedId(null)
      setInspectorOpen(false)
      // D3：叶子/容器删除成功给结果反馈（不引入二次确认；Ctrl+Z 可撤销）
      if (ok && !containerLanes(loc.step)) {
        setNotice('已删除（可 Ctrl+Z 撤销）')
      }
    },
    [selectedId, readOnly, applyEdit, askConfirm],
  )

  const duplicateSelected = useCallback(
    async (id?: string) => {
      const targetId = id ?? selectedId
      const cur = stepsRef.current
      if (!cur || !targetId || readOnly || !layer) return
      const loc = locateStep(cur, targetId)
      if (!loc) return
      const ids = collectIds(cur)
      const copy = cloneStepWithNewIds(loc.step, ids)
      copy.name = `${copy.name || copy.id} 副本`
      const ok = await applyEdit({
        op: 'add_step',
        parent: { layerId: loc.layerId },
        lane: loc.lane,
        index: loc.index + 1,
        step: copy,
      })
      if (ok) {
        // D1/D7：复制成功后闪光高亮新副本，用户立刻看到复制落在哪里
        setSelectedId(copy.id)
        flashStep(copy.id)
      }
    },
    [selectedId, readOnly, layer, applyEdit, flashStep],
  )

  // ── 节点 hover 操作（阶段 4：编辑/复制/删除快捷入口，注入节点 data；不经过闭包 selectedId）──
  const nodeActions = useMemo(
    () => ({
      onEdit: (id: string) => {
        setSelectedId(id)
        setInspectorOpen(true)
      },
      onDuplicate: (id: string) => {
        setSelectedId(id)
        void duplicateSelected(id)
      },
      onDelete: (id: string) => {
        setSelectedId(id)
        void deleteSelected(id)
      },
    }),
    [duplicateSelected, deleteSelected],
  )

  const addStep = useCallback(
    async (kind: string) => {
      // 先收菜单：即便守卫未命中（未加载完/只读）也不留残影
      setAddMenuOpen(false)
      const cur = stepsRef.current
      if (!cur || !layer || readOnly) return
      const step = newStep(kind, collectIds(cur))
      const { lane, index } = clickInsertion(cur, layer, selectedId)
      const ok = await applyEdit({
        op: 'add_step',
        parent: { layerId: layer.layerId },
        lane,
        index,
        step,
      })
      if (ok) {
        setSelectedId(step.id)
        setInspectorOpen(true)
        setToolFocusId(kind === 'tool' ? step.id : null)
        focusStepFlow(step.id)
        flashStep(step.id) // D1/D7：普通添加后闪光高亮，与连线插入/录制入画布一致
      }
    },
    [layer, readOnly, selectedId, applyEdit, rf, focusStepFlow, flashStep],
  )

  // ── 工具面板：点击/拖拽创建 tool 步骤（with 按 input_schema required 预填骨架）──
  const addToolStep = useCallback(
    async (tool: ToolSchema, dropPoint?: { x: number; y: number }) => {
      const cur = stepsRef.current
      if (!cur || !layer || readOnly) return
      const step = newStep('tool', collectIds(cur))
      step.do = { tool: tool.name, with: skeletonFromSchema(tool.input_schema) }
      let lane: LaneId
      let index: number
      if (dropPoint) {
        // 落点定位：多泳道层按落点 x 命中泳道框；插入序沿层主轴取「最近节点之后」
        // （LR 层同级横向流走 x 轴，TB 子层走 y 轴——与 onNodeDragStop 同规则）
        lane = layer.swimlanes[0]?.id ?? 'main'
        for (const f of laneFrames) {
          if (dropPoint.x >= f.x && dropPoint.x <= f.x + f.width) {
            lane = f.lane
            break
          }
        }
        const horizontal = layerDir(layer.layerId) === 'LR'
        // 按节点「实际中心」推断插入序（与 onNodeDragStop 同规则）：真实尺寸计算中心，替换固定 100/28
        const centers = layer.nodes
          .filter(n => n.lane === lane && !n.synthetic)
          .map(n => {
            const fn = rf.getNodes().find(fn => fn.id === n.id)
            const pos = fn?.position ?? { x: 0, y: 0 }
            const w = fn?.measured?.width ?? (n.category === 'container' ? 220 : 200)
            const h = fn?.measured?.height ?? (n.category === 'container' ? 64 : 56)
            return horizontal ? pos.x + w / 2 : pos.y + h / 2
          })
        index = centers.filter(c => c < (horizontal ? dropPoint.x : dropPoint.y)).length
      } else {
        ;({ lane, index } = clickInsertion(cur, layer, selectedId))
      }
      const ok = await applyEdit({
        op: 'add_step',
        parent: { layerId: layer.layerId },
        lane,
        index,
        step,
      })
      if (ok) {
        setSelectedId(step.id)
        setInspectorOpen(true)
        focusStepFlow(step.id)
        flashStep(step.id) // D1/D7：工具面板添加后闪光高亮
      }
    },
    [layer, readOnly, selectedId, laneFrames, applyEdit, rf, focusStepFlow, flashStep],
  )

  // 工具注册表（与 Inspector/ToolPalette 共享模块级缓存）：拖拽 drop 时按名查 schema 预填骨架
  const [wfToolsList, setWfToolsList] = useState<ToolSchema[] | null>(null)
  useEffect(() => {
    let alive = true
    void loadToolsOnce().then(t => {
      if (alive) setWfToolsList(t)
    })
    return () => {
      alive = false
    }
  }, [])

  /** 意图表单提交（2026-09-05 方案A，取代旧的录制一键交给 WorkflowAgent）：
   *  纯文本模板注入 workflow 输入框（带阶段/子步骤意图 + workflow id + 目录 + 名称），
   *  先保存画布（如有 dirty）→ 关画布 → 不启动任何后端执行
   *  （由用户在聊天发送后进 WorkflowAgent） */
  const submitIntentForm = useCallback(
    (form: IntentForm) => {
      setIntentFormOpen(false)
      void save().finally(() => {
        onClose()
        const text = buildIntentTextTemplate(form, workflowId, ir?.name)
        window.dispatchEvent(
          new CustomEvent('nuphus:append-to-chat', {
            detail: { text, mode: 'workflow' },
          }),
        )
      })
    },
    [save, onClose, workflowId, ir?.name],
  )

  // ── 工具面板拖拽入画布（HTML5 DnD）──
  const onToolDragOver = useCallback(
    (e: React.DragEvent) => {
      // 无条件放行 dragOver（React Flow 官方 DnD 模式）：合法性判断在 onDrop 用 getData 完成。
      // 不能依赖 dataTransfer.types.includes —— WebView2 的 types 是 DOMStringList，无 includes 方法，
      // 会导致 dragOver 不 preventDefault → drop 被浏览器默认行为拦截（拖拽添加失败）。
      e.preventDefault()
      e.dataTransfer.dropEffect = readOnly ? 'none' : 'copy'
    },
    [readOnly],
  )

  const onToolDrop = useCallback(
    (e: React.DragEvent) => {
      const name = e.dataTransfer.getData(TOOL_DRAG_MIME)
      if (!name) return
      e.preventDefault()
      if (readOnly) return
      // 注册表缺失（加载失败/新工具未同步）时降级为空 schema：骨架 {}，工具名仍落位
      const tool = wfToolsList?.find(t => t.name === name) ?? {
        name,
        description: '',
        input_schema: {} as Record<string, unknown>,
      }
      const point = rf.screenToFlowPosition({ x: e.clientX, y: e.clientY })
      void addToolStep(tool, point)
    },
    [readOnly, wfToolsList, rf, addToolStep],
  )

  // ── 键盘表（2.5）──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // 意图表单打开时：画布快捷键全部隔离，仅 Escape 可关闭弹层（防止误触 Delete/复制等）
      if (intentFormOpen) {
        if (e.key === 'Escape') {
          e.preventDefault()
          setIntentFormOpen(false)
        }
        return
      }
      const tag = (e.target as HTMLElement)?.tagName
      const typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
        e.preventDefault()
        void save()
        return
      }
      if (typing) return
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault()
        if (e.shiftKey) redo()
        else undo()
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'y') {
        e.preventDefault()
        redo()
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'd') {
        e.preventDefault()
        void duplicateSelected()
      } else if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault()
        void deleteSelected()
      } else if (e.key === 'n' || e.key === 'N') {
        if (!readOnly) setAddMenuOpen(o => !o)
      } else if (e.key === 'r' || e.key === 'R') {
        void runWorkflow()
      } else if (e.key === 'Enter' && selectedId) {
        setInspectorOpen(true)
      } else if (e.key === 'Escape') {
        if (edgeInsert) setEdgeInsert(null)
        else if (addMenuOpen) setAddMenuOpen(false)
        else if (inspectorOpen) closeInspector()
        else setSelectedId(null)
      } else if (e.altKey && e.key === 'ArrowLeft') {
        e.preventDefault()
        if (layer && layer.parentChain.length > 0) {
          switchLayer(layer.parentChain[layer.parentChain.length - 1])
        }
      }
    }
    // capture 阶段监听：WebView2 会把 Alt+← 当历史后退在 bubble 前吞掉，
    // 必须 capture 先拿到事件并 preventDefault；remove 需保持同一 capture 标志
    window.addEventListener('keydown', onKey, { capture: true })
    return () => window.removeEventListener('keydown', onKey, { capture: true })
  }, [
    save,
    undo,
    redo,
    duplicateSelected,
    deleteSelected,
    runWorkflow,
    readOnly,
    selectedId,
    addMenuOpen,
    edgeInsert,
    inspectorOpen,
    closeInspector,
    intentFormOpen,
    layer,
    switchLayer,
  ])

  // ── ProblemsPanel 定位（3.3：下钻 + 居中闪烁）──
  const locateNode = useCallback(
    (stepId: string) => {
      if (!projection) return
      const targetLayer = projection.index.layerOf.get(stepId)
      if (targetLayer && targetLayer !== layerId) switchLayer(targetLayer)
      setSelectedId(stepId)
      setFlashId(stepId)
      setTimeout(() => setFlashId(null), 1500)
      focusStepFlow(stepId)
    },
    [projection, layerId, switchLayer, rf, focusStepFlow],
  )

  // ── 连线中点插入（大王反馈）：SequenceEdge 派发 wf-insert-at → 记录 target edgeId + 屏幕坐标 ──
  useEffect(() => {
    const onInsertAt = (e: Event) => {
      if (readOnly) return
      const d = (e as CustomEvent<{ edgeId?: string; x?: number; y?: number }>).detail
      if (!d?.edgeId) return
      setEdgeInsert({ edgeId: d.edgeId, x: d.x ?? 0, y: d.y ?? 0 })
    }
    window.addEventListener(EDGE_INSERT_EVENT, onInsertAt)
    return () => window.removeEventListener(EDGE_INSERT_EVENT, onInsertAt)
  }, [readOnly])

  /** 菜单标题预览：该边终点（插入后紧跟新节点的那个步骤）显示名 */
  const edgeTargetName = useMemo(() => {
    if (!edgeInsert || !steps || !layer) return null
    const ce = layer.edges.find(e => e.id === edgeInsert.edgeId && e.kind === 'sequence')
    if (!ce) return null
    const loc = locateStep(steps, ce.target)
    return loc?.step.name || loc?.step.id || null
  }, [edgeInsert, steps, layer])

  /**
   * 从连线中点插入：把新步骤插到 target 节点前（原 sequence 边 source→target 变
   * source→新→target 两条边——projection 由层内步骤顺序推导 seq 边，add_step 后自动重建）。
   * 单次 applyEdit → undo 一步整体回退。
   */
  const insertAtEdge = useCallback(
    async (kind: string) => {
      const target = edgeInsert
      setEdgeInsert(null) // 先收菜单：只读/失效/失败也不留残影
      const cur = stepsRef.current
      if (!cur || !layer || readOnly || !target) return
      const ce = layer.edges.find(e => e.id === target.edgeId && e.kind === 'sequence')
      if (!ce) {
        setNotice('插入位置已失效（边结构已变化），请重新点击连线中点')
        return
      }
      const tLoc = locateStep(cur, ce.target)
      if (!tLoc) {
        setNotice('无法定位该连线终点节点，未插入')
        return
      }
      if (tLoc.layerId !== layer.layerId) {
        setNotice('跨层连线暂不支持插入，请进入对应子层操作')
        return
      }
      const step = newStep(kind, collectIds(cur))
      const ok = await applyEdit({
        op: 'add_step',
        parent: { layerId: tLoc.layerId },
        lane: tLoc.lane,
        index: tLoc.index, // target 节点前 = 原边 source→新、新→target
        step,
      })
      if (!ok) return
      setSelectedId(step.id)
      setInspectorOpen(true)
      flashStep(step.id)
      focusStepFlow(step.id)
    },
    [edgeInsert, layer, readOnly, applyEdit, rf, flashStep, focusStepFlow],
  )

  // ── 面包屑 ──
  const breadcrumb = useMemo(() => {
    if (!layer || !projection) return []
    const chain = [...layer.parentChain, layer.layerId]
    return chain.map(id => ({
      id,
      label: id === 'root' ? 'root' : (projection.index.nodeById.get(id)?.name ?? id),
    }))
  }, [layer, projection])

  // ── 顶部横幅锚点（Error → 失败步骤名；Paused → 暂停前最后已完成步骤名）──
  // lastRunError/lastRunPaused 已在顶部 state 区派生（供 runWorkflow fresh 决策）。
  const anchorStepName = useMemo(() => {
    if (!lastRun) return null
    const recs = Array.isArray(lastRun.steps)
      ? (lastRun.steps as { step_id: string; status: unknown }[])
      : []
    if (lastRunError) {
      // Error：仅指向真正失败步骤；run 记录无失败步骤（校验前失败等）→ 省略「于 X」
      const failed = recs.find(
        r => typeof r.status === 'object' && r.status !== null && 'Error' in (r.status as object),
      )
      if (!failed) return null
      return projection?.index.nodeById.get(failed.step_id)?.name ?? failed.step_id
    }
    if (lastRunPaused) {
      // Paused：暂停发生在步骤边界，取暂停前最后一条已完成记录作进度锚点
      const anchor = recs[recs.length - 1]
      if (!anchor) return null
      return projection?.index.nodeById.get(anchor.step_id)?.name ?? anchor.step_id
    }
    return null
  }, [lastRun, lastRunError, lastRunPaused, projection])

  const selectedStep = useMemo(() => {
    if (!selectedId || !steps) return null
    return locateStep(steps, selectedId)?.step ?? null
  }, [selectedId, steps])

  if (!ir || !steps || !projection) {
    return (
      <div className="wfc-page">
        <div className="wfc-loading">{notice ?? '加载画布中…'}</div>
      </div>
    )
  }

  /** 工作流重命名提交：空名/未变化 → 退出编辑；否则保存当前步骤 + 新名 */
  const commitNameRename = () => {
    const name = nameInput.trim()
    if (!name || !ir || name === ir.name) {
      setNameInput(ir?.name ?? '')
      setNameEditing(false)
      return
    }
    setNameEditing(false)
    void save(name)
  }

  return (
    <div className="wfc-page">
      {/* ── 工具栏 ── */}
      <div className="wfc-toolbar">
        <button type="button" className="wfc-icon-btn" onClick={handleClose} title="关闭画布">
          <X size={15} />
        </button>
        {/* 工作流名称：双击/失焦提交改名（readOnly 禁改；提交走 save(name) 连同当前步骤保存） */}
        {nameEditing && ir && !readOnly ? (
          <input
            className="wfc-title-input"
            autoFocus
            value={nameInput}
            onChange={e => setNameInput(e.target.value)}
            onBlur={commitNameRename}
            onKeyDown={e => {
              if (e.key === 'Enter') commitNameRename()
              else if (e.key === 'Escape') {
                setNameInput(ir?.name ?? '')
                setNameEditing(false)
              }
            }}
          />
        ) : (
          <span
            className="wfc-title"
            title={readOnly ? undefined : '双击重命名工作流'}
            onDoubleClick={() => {
              if (readOnly) return
              setNameInput(ir?.name ?? '')
              setNameEditing(true)
            }}
          >
            {ir.name}
          </span>
        )}
        {dirty && <span className="wfc-badge wfc-badge--dirty">未保存</span>}
        {readOnly && (
          <span className="wfc-badge">{snapshot.running ? '运行中 · 只读' : '只读'}</span>
        )}

        <div className="wfc-toolbar-spacer" />

        <button
          type="button"
          className="wfc-btn"
          onClick={() => {
            playUiSound('switch')
            setIntentFormOpen(true)
          }}
          disabled={readOnly}
          title={
            readOnly
              ? snapshot.running
                ? '运行中 · 画布只读'
                : '只读画布，不可发起意图'
              : '用阶段 + 子步骤描述要做的事，交给 AI 整理为工作流'
          }
        >
          <ListChecks size={13} /> 意图表单
        </button>

        <button
          type="button"
          className="wfc-btn"
          onClick={undo}
          disabled={readOnly}
          title="撤销（Ctrl+Z）"
        >
          <Undo2 size={13} />
        </button>
        <button
          type="button"
          className="wfc-btn"
          onClick={redo}
          disabled={readOnly}
          title="重做（Ctrl+Shift+Z）"
        >
          <Redo2 size={13} />
        </button>
        <div className="wfc-add-wrap" ref={addWrapRef}>
          <button
            type="button"
            className="wfc-btn"
            onClick={() => setAddMenuOpen(o => !o)}
            disabled={readOnly}
            title="添加节点（N）"
          >
            <Plus size={13} /> 添加
          </button>
          {addMenuOpen && (
            <div className="wfc-add-menu">
              {ADDABLE_KINDS.map(({ kind, desc }) => (
                <button
                  type="button"
                  key={kind}
                  className="wfc-add-item"
                  title={desc}
                  onClick={() => void addStep(kind)}
                >
                  <span className="wfc-add-item-kind">{kind}</span>
                  <span className="wfc-add-item-desc">{desc}</span>
                </button>
              ))}
            </div>
          )}
        </div>
        <button
          type="button"
          className="wfc-btn"
          onClick={() => void runCheck()}
          title="后端权威校验"
        >
          <CircleCheckBig size={13} /> 检查
        </button>
        <button
          type="button"
          className="wfc-btn"
          onClick={() => void save()}
          disabled={!dirty || snapshot.running}
          title="保存（Ctrl+S，保存前强制校验）"
        >
          <Save size={13} /> 保存
        </button>
        <button
          type="button"
          className="wfc-btn wfc-btn--primary"
          onClick={() => void runWorkflow()}
          disabled={snapshot.running || gateLocked}
          title={
            gateLocked && !snapshot.running
              ? gateLockNotice
              : lastRunError
                ? '运行（R）：上次运行失败，从头完整执行（不再续连）'
                : lastRunPaused
                  ? '续跑（R）：自动跳过已完成步骤，从暂停处继续'
                  : '运行（R）：执行工作流'
          }
        >
          <Play size={13} /> {lastRunPaused ? '续跑' : '运行'}
        </button>
      </div>

      {/* ── 横幅区 ── */}
      {projection.index.hasCustomNodes && (
        <div className="wfc-banner wfc-banner--error">
          <strong>格式不兼容：</strong>本工作流包含旧格式节点，画布已切换为只读。 可通过 AI
          对话重新生成同目标工作流（V2 格式）后在画布中编辑；旧格式仍可经原聊天通道运行。
        </div>
      )}
      {(lastRunError || lastRunPaused) && !projection.index.hasCustomNodes && (
        <div className="wfc-banner wfc-banner--warning">
          {lastRunError
            ? `上次运行失败${anchorStepName ? `于「${anchorStepName}」` : ''}，点击「运行」将从头完整执行，不再续连上次进度。`
            : `上次运行已暂停${anchorStepName ? `于「${anchorStepName}」` : ''}，点击「续跑」将自动跳过已完成步骤，从暂停处继续。`}
        </div>
      )}
      {notice && (
        <div className="wfc-banner wfc-banner--info">
          {notice}
          <button type="button" className="wfc-icon-btn" onClick={() => setNotice(null)}>
            <X size={12} />
          </button>
        </div>
      )}
      {snapshot.running && (
        <div className="wfc-banner wfc-banner--running">
          运行中 —— 画布已锁定为只读，防止编辑-执行竞态
        </div>
      )}

      {/* ── 面包屑 ── */}
      <div className="wfc-breadcrumb" ref={breadcrumbRef}>
        {layer && layer.parentChain.length > 0 && (
          <button
            type="button"
            className="wfc-icon-btn"
            title="返回父层（Alt+←）"
            onClick={() => switchLayer(layer.parentChain[layer.parentChain.length - 1])}
          >
            <CornerUpLeft size={14} />
          </button>
        )}
        {breadcrumb.map((b, i) => (
          <span key={b.id} className="wfc-crumb-wrap">
            {i > 0 && <span className="wfc-crumb-sep">/</span>}
            <button
              type="button"
              data-layer-id={b.id}
              className={`wfc-crumb${b.id === layerId ? ' is-active' : ''}`}
              onClick={() => switchLayer(b.id)}
              title={readOnly ? b.label : `${b.label}（可拖拽节点到此移入该层）`}
            >
              {b.label}
            </button>
          </span>
        ))}
        <span className="wfc-crumb-hint">双击容器进入子层 · Alt+← 返回父层</span>
      </div>

      {/* ── 画布主体 ── */}
      <div className="wfc-canvas-wrap">
        {/* NodeActionsContext：节点 hover 操作（阶段 4）；readOnly 时注入 null 隐藏操作条 */}
        <NodeActionsContext.Provider value={readOnly ? null : nodeActions}>
          <ReactFlow
            nodes={flowNodes}
            edges={flowEdges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            onNodesChange={onNodesChange}
            onNodeDrag={onNodeDrag}
            onNodeDragStop={onNodeDragStop}
            onNodeClick={onNodeClick}
            onNodeDoubleClick={onNodeDoubleClick}
            onPaneClick={() => {
              setSelectedId(null)
              setInspectorOpen(false)
            }}
            onDragOver={onToolDragOver}
            onDrop={onToolDrop}
            nodesConnectable={false}
            edgesFocusable={false}
            deleteKeyCode={null}
            fitView
            minZoom={0.2}
            maxZoom={2}
            proOptions={{ hideAttribution: false }}
          >
            <Background variant={BackgroundVariant.Lines} gap={24} color="var(--line-1)" />
            <Controls showInteractive={false} />
          </ReactFlow>
        </NodeActionsContext.Provider>

        {/* ── 拖拽插入指示线（阶段 3：重排前视觉反馈；松手后按 dragInsertRef 决定是否重排）── */}
        {dragInsertScreen && (
          <div
            className="wfc-insert-hint"
            style={{
              left: dragInsertScreen.x,
              top: dragInsertScreen.y,
              width: dragInsertScreen.horizontal ? 2 : 96,
              height: dragInsertScreen.horizontal ? 96 : 2,
              transform: dragInsertScreen.horizontal
                ? 'translate(-1px, -48px)'
                : 'translate(-48px, -1px)',
            }}
          />
        )}

        {/* ── 工具面板（左侧分类竖条；只读时禁用，定位/增步均走 applyEdit 管线）── */}
        <ToolPalette disabled={readOnly} onAdd={tool => void addToolStep(tool)} />

        {/* ── 结构大纲（右下角，替代 MiniMap；定位是纯视图操作，readOnly/运行中均可用）── */}
        <OutlinePanel
          steps={steps}
          selectedId={selectedId}
          statuses={outlineStatus}
          onLocate={locateNode}
        />

        {inspectorOpen && selectedStep && (
          <Inspector
            step={selectedStep}
            readOnly={readOnly}
            idReferenced={(ir.run_history ?? []).some(r =>
              (Array.isArray(r.steps) ? (r.steps as { step_id?: string }[]) : []).some(
                s => s.step_id === selectedStep.id,
              ),
            )}
            lastOutput={snapshot.outputs.get(selectedStep.id)}
            focusToolId={toolFocusId}
            onPatch={patch =>
              void applyEdit({ op: 'update_fields', stepId: selectedStep.id, patch })
            }
            onPatchAction={action =>
              void applyEdit({
                op: 'update_fields',
                stepId: selectedStep.id,
                patch: { do: action },
              })
            }
            onClose={closeInspector}
          />
        )}
      </div>

      {/* ── 连线中点「在此插入」菜单（fixed 定位在圆点屏幕坐标旁；复用 wfc-add-item 项样式）── */}
      {edgeInsert && (
        <div
          ref={edgeMenuWrapRef}
          className="wfc-add-menu wfc-edge-add-menu"
          style={{
            left: Math.max(8, Math.min(edgeInsert.x + 12, window.innerWidth - 296)),
            top: Math.max(8, Math.min(edgeInsert.y + 12, window.innerHeight - 400)),
          }}
        >
          <div className="wfc-edge-add-title">
            {edgeTargetName ? `在此插入（插到「${edgeTargetName}」之前）` : '在此插入'}
          </div>
          {ADDABLE_KINDS.map(({ kind, desc }) => (
            <button
              type="button"
              key={kind}
              className="wfc-add-item"
              title={desc}
              onClick={() => void insertAtEdge(kind)}
            >
              <span className="wfc-add-item-kind">{kind}</span>
              <span className="wfc-add-item-desc">{desc}</span>
            </button>
          ))}
        </div>
      )}

      {/* ── 问题面板 ── */}
      <ProblemsPanel
        problems={problems}
        backendReport={backendReport}
        onLocate={locateNode}
        nameOf={id => projection.index.nodeById.get(id)?.name ?? id}
      />

      {/* ── 意图表单弹层（画布顶部「意图表单」入口；不启动录制、不改画布 dirty） ── */}
      {intentFormOpen && (
        <IntentFormPanel
          initialName={ir.name}
          onSubmit={submitIntentForm}
          onClose={() => setIntentFormOpen(false)}
        />
      )}

      {/* ── 确认弹窗 ── */}
      {confirm && (
        <div className="wfc-confirm-mask">
          <div className="wfc-confirm">
            <div className="wfc-confirm-msg">{confirm.message}</div>
            <div className="wfc-confirm-actions">
              <button
                type="button"
                className="wfc-btn"
                onClick={() => {
                  confirm.resolve(false)
                  setConfirm(null)
                }}
              >
                取消
              </button>
              <button
                type="button"
                className="wfc-btn wfc-btn--primary"
                onClick={() => {
                  confirm.resolve(true)
                  setConfirm(null)
                }}
              >
                确认
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

// 文件结束
