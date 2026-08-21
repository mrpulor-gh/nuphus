/**
 * irEdit.ts — IrEditOp 应用器 + 合法性拦截矩阵（设计文档 1.5）+ 撤销/重做栈
 *
 * 原则：操作一律表达为 IrEditOp → 应用生成新 IR → 重投影 → 权威校验 → wf_save。
 * 禁止画布维护独立"图模型"再整体导出（防图/树语义漂移）。
 */

import type { WorkflowStep, Action, RunRecord } from '../../core/types'
import type { EditCheck, IrEditOp, LaneId, LayerRef } from './types'
import { containerLanes, laneSteps, stepKind } from './projection'
import { walkSteps, profileStep } from './dataEdges'

// ── 定位 ──

interface StepLocation {
  step: WorkflowStep
  /** 所在的直接兄弟列表（可变引用，指向克隆树内数组） */
  siblings: WorkflowStep[]
  index: number
  lane: LaneId
  /** 所在层 layerId（root 直接子步骤为 'root'） */
  layerId: string
  parentId: string | null
}

/** 在树中定位步骤（携带所在兄弟列表与层信息） */
export function locateStep(steps: WorkflowStep[], stepId: string): StepLocation | null {
  let found: StepLocation | null = null
  const visit = (
    list: WorkflowStep[],
    lane: LaneId,
    layerId: string,
    parentId: string | null,
  ): boolean => {
    for (let i = 0; i < list.length; i++) {
      const s = list[i]
      if (s.id === stepId) {
        found = { step: s, siblings: list, index: i, lane, layerId, parentId }
        return true
      }
      const c = containerLanes(s)
      if (c) {
        for (const l of c.lanes) {
          if (visit(laneSteps(s, l.id), l.id, s.id, s.id)) return true
        }
      }
    }
    return false
  }
  visit(steps, 'main', 'root', null)
  return found
}

/** 取 LayerRef + lane 对应的兄弟列表；宿主不存在/非容器返回 null */
function listOf(steps: WorkflowStep[], parent: LayerRef, lane: LaneId): WorkflowStep[] | null {
  if (parent.layerId === 'root') return lane === 'main' ? steps : null
  const loc = locateStep(steps, parent.layerId)
  if (!loc || !containerLanes(loc.step)) return null
  return laneSteps(loc.step, lane)
}

/** 全树 id 集合 */
export function collectIds(steps: WorkflowStep[]): Set<string> {
  const ids = new Set<string>()
  walkSteps(steps, s => ids.add(s.id))
  return ids
}

/** 判断 ancestor 是否为 descendant 的祖先（防容器拖入自身子树） */
function isAncestor(steps: WorkflowStep[], ancestorId: string, descendantId: string): boolean {
  const loc = locateStep(steps, ancestorId)
  if (!loc) return false
  let found = false
  const c = containerLanes(loc.step)
  if (!c) return false
  for (const l of c.lanes) {
    walkSteps(laneSteps(loc.step, l.id), s => {
      if (s.id === descendantId) found = true
    })
  }
  return found
}

/** 步骤是否在任一 loop 子树内（含自身即 loop 的情况；break/continue 合法性） */
function insideLoop(steps: WorkflowStep[], stepId: string): boolean {
  const loc = locateStep(steps, stepId)
  if (!loc) return false
  if (stepKind(loc.step) === 'loop') return true
  let cur: string | null = loc.parentId
  while (cur) {
    const p = locateStep(steps, cur)
    if (!p) return false
    if (stepKind(p.step) === 'loop') return true
    cur = p.parentId
  }
  return false
}

/** 列出步骤及其子树引用的全部变量名 */
function referencedVars(step: WorkflowStep): Set<string> {
  const vars = new Set<string>()
  walkSteps([step], s => {
    for (const c of profileStep(s).consumes) vars.add(c.varName)
  })
  return vars
}

/** 外层 loop 的 item_var 列表（从叶向根） */
function enclosingLoopItemVars(steps: WorkflowStep[], stepId: string): string[] {
  const out: string[] = []
  const loc = locateStep(steps, stepId)
  if (!loc) return out
  let cur: string | null = loc.parentId
  while (cur) {
    const p = locateStep(steps, cur)
    if (!p) break
    if (stepKind(p.step) === 'loop') {
      const def = (p.step.do as Record<string, unknown>).loop as { for_each?: { as?: string } }
      out.push(def.for_each?.as || 'item')
    }
    cur = p.parentId
  }
  return out
}

// ── 合法性拦截矩阵（1.5）──

export interface CheckContext {
  /** V14/1.5#8：run_history 引用检查（改 id 时） */
  runHistory?: RunRecord[]
}

export function checkOp(steps: WorkflowStep[], op: IrEditOp, ctx: CheckContext = {}): EditCheck {
  switch (op.op) {
    case 'add_step': {
      if (!op.step.id) return { ok: false, reason: '新步骤缺少 id' }
      if (collectIds(steps).has(op.step.id)) {
        return { ok: false, reason: `步骤 id「${op.step.id}」已存在（全树必须唯一），请重新生成` }
      }
      if (!listOf(steps, op.parent, op.lane)) {
        return { ok: false, reason: '目标位置不存在（宿主不是容器或泳道无效）' }
      }
      return { ok: true }
    }

    case 'remove_step': {
      const loc = locateStep(steps, op.stepId)
      if (!loc) return { ok: false, reason: '步骤不存在' }
      // 1.5#7：删除容器内最后一个步骤 → 拦截（compiler 空容器 error）
      // if.else 允许为空（IR 语义默认空分支）；root 层为空仅 warning
      if (loc.layerId !== 'root' && loc.siblings.length === 1 && loc.lane !== 'else') {
        return {
          ok: false,
          reason:
            '容器至少需要保留一个步骤（空容器无法通过校验）。请先添加新步骤，再删除本步骤。',
        }
      }
      // 容器级联删除由 UI 弹窗确认（2.5 Delete 行），此处放行
      return { ok: true }
    }

    case 'reorder': {
      const loc = locateStep(steps, op.stepId)
      if (!loc) return { ok: false, reason: '步骤不存在' }
      return { ok: true }
    }

    case 'move_step': {
      const loc = locateStep(steps, op.stepId)
      if (!loc) return { ok: false, reason: '步骤不存在' }
      const targetList = listOf(steps, op.to.parent, op.to.lane)
      if (!targetList) return { ok: false, reason: '目标位置不存在（宿主不是容器或泳道无效）' }

      // 同层同泳道 → 等价 reorder，放行
      const sameList = loc.layerId === op.to.parent.layerId && loc.lane === op.to.lane

      // 防容器拖入自身子树（树成环）
      if (
        op.to.parent.layerId !== 'root' &&
        (op.to.parent.layerId === op.stepId || isAncestor(steps, op.stepId, op.to.parent.layerId))
      ) {
        return { ok: false, reason: '不能把容器移动到自己的子树内（会形成循环嵌套）' }
      }

      if (!sameList) {
        // 1.5#3：break/continue 移出 loop → 拦截（compiler in_loop 检查）
        const kind = stepKind(loc.step)
        if (kind === 'break' || kind === 'continue') {
          const targetInLoop =
            op.to.parent.layerId !== 'root' && insideLoop(steps, op.to.parent.layerId)
          if (!targetInLoop) {
            return {
              ok: false,
              reason: `「${kind}」仅可在循环（loop）内使用。请移动到其他 loop 容器内，或改用条件分支。`,
            }
          }
        }
        // 1.5#3：引用外层 loop item_var 的步骤移出该 loop → 拦截
        const itemVars = enclosingLoopItemVars(steps, op.stepId)
        if (itemVars.length > 0) {
          const refs = referencedVars(loc.step)
          const used = itemVars.filter(v => refs.has(v))
          if (used.length > 0) {
            const targetItemVars =
              op.to.parent.layerId === 'root' ? [] : enclosingLoopItemVars(steps, op.to.parent.layerId)
            const missing = used.filter(v => !targetItemVars.includes(v))
            if (missing.length > 0) {
              return {
                ok: false,
                reason: `该步骤引用了循环变量 ${missing.map(v => `{{${v}}}`).join('、')}，移出循环后变量不存在。请先修改引用，或在目标循环中定义同名 item 变量。`,
              }
            }
          }
        }
      }
      return { ok: true }
    }

    case 'update_fields': {
      const loc = locateStep(steps, op.stepId)
      if (!loc) return { ok: false, reason: '步骤不存在' }
      // 1.5#8：修改 step.id —— 有 run_history 引用时需确认（断点续连记录将失效）
      if (op.patch.id && op.patch.id !== op.stepId) {
        if (collectIds(steps).has(op.patch.id)) {
          return { ok: false, reason: `id「${op.patch.id}」已被其他步骤占用` }
        }
        const referenced = (ctx.runHistory ?? []).some(r => {
          const recs = Array.isArray(r.steps) ? (r.steps as { step_id?: string }[]) : []
          return recs.some(s => s.step_id === op.stepId)
        })
        if (referenced) {
          return {
            ok: true,
            confirm: '该步骤有历史运行记录，修改 id 将使断点续连跳过记录失效（续跑会重新执行本步骤）。确认修改？',
          }
        }
      }
      return { ok: true }
    }

    case 'change_kind': {
      const loc = locateStep(steps, op.stepId)
      if (!loc) return { ok: false, reason: '步骤不存在' }
      // 1.5#9：子步骤迁移需确认；if→其他容器时 else 分支将被丢弃
      const migration = migrateChildren(loc.step, op.action)
      if (migration.dropped > 0) {
        return {
          ok: true,
          confirm: `类型变更将保留 ${migration.kept} 个子步骤、丢弃 ${migration.dropped} 个（else 分支无法迁移）。确认继续？`,
        }
      }
      if (migration.kept > 0) {
        return { ok: true, confirm: `原 ${migration.kept} 个子步骤将迁移到新类型。确认继续？` }
      }
      return { ok: true }
    }
  }
}

/** change_kind 子步骤迁移（1.5#9）：then/do/seq/auto → 新容器首位子列表；else 丢弃 */
function migrateChildren(step: WorkflowStep, newAction: Action): { kept: number; dropped: number } {
  const c = containerLanes(step)
  if (!c) return { kept: 0, dropped: 0 }
  const lanes = c.lanes.map(l => ({ id: l.id, list: laneSteps(step, l.id) }))
  const na = newAction as Record<string, unknown>
  const targetIsIf = 'if' in na
  let kept = 0
  let dropped = 0
  for (const { id, list } of lanes) {
    if (targetIsIf && (id === 'then' || id === 'else')) {
      kept += list.length
    } else if (id === 'else') {
      dropped += list.length
    } else {
      kept += list.length
    }
  }
  return { kept, dropped }
}

/** 生成迁移后的子列表（kept 顺序保持遍历序） */
function migratedChildList(step: WorkflowStep, newAction: Action): WorkflowStep[] {
  const c = containerLanes(step)
  if (!c) return []
  const na = newAction as Record<string, unknown>
  const targetIsIf = 'if' in na
  const out: WorkflowStep[] = []
  for (const l of c.lanes) {
    if (!targetIsIf && l.id === 'else') continue // else 丢弃（已经过确认）
    out.push(...laneSteps(step, l.id))
  }
  return out
}

// ── 应用 ──

const clone = <T>(v: T): T => structuredClone(v)

/** 应用 IrEditOp，返回新的 steps 树（不可变语义：输入不被修改）。调用前必须先过 checkOp。 */
export function applyOp(steps: WorkflowStep[], op: IrEditOp): WorkflowStep[] {
  const next = clone(steps)
  switch (op.op) {
    case 'add_step': {
      const list = listOf(next, op.parent, op.lane)
      if (!list) return next
      const idx = Math.max(0, Math.min(op.index, list.length))
      list.splice(idx, 0, clone(op.step))
      break
    }
    case 'remove_step': {
      const loc = locateStep(next, op.stepId)
      if (loc) loc.siblings.splice(loc.index, 1)
      break
    }
    case 'reorder': {
      const loc = locateStep(next, op.stepId)
      if (!loc) break
      const [s] = loc.siblings.splice(loc.index, 1)
      const idx = Math.max(0, Math.min(op.newIndex, loc.siblings.length))
      loc.siblings.splice(idx, 0, s)
      break
    }
    case 'move_step': {
      const loc = locateStep(next, op.stepId)
      const target = listOf(next, op.to.parent, op.to.lane)
      if (!loc || !target) break
      const sameList = loc.layerId === op.to.parent.layerId && loc.lane === op.to.lane
      const [s] = loc.siblings.splice(loc.index, 1)
      let idx = Math.max(0, Math.min(op.to.index, target.length))
      // 同列表内后移：删除后目标下标前移一位
      if (sameList && op.to.index > loc.index) idx -= 1
      target.splice(Math.max(0, idx), 0, s)
      break
    }
    case 'update_fields': {
      const loc = locateStep(next, op.stepId)
      if (!loc) break
      const { id, ...rest } = op.patch
      Object.assign(loc.step, rest)
      if (id && id !== op.stepId) loc.step.id = id
      break
    }
    case 'change_kind': {
      const loc = locateStep(next, op.stepId)
      if (!loc) break
      const children = migratedChildList(loc.step, op.action)
      const action = clone(op.action) as Record<string, unknown>
      // 子步骤注入新容器体
      if ('seq' in action) action.seq = children
      else if ('loop' in action) (action.loop as Record<string, unknown>).do = children
      else if ('if' in action) (action.if as Record<string, unknown>).then = children
      else if ('auto' in action || 'wait' in action) action.auto = children
      loc.step.do = action as Action
      break
    }
  }
  return next
}

// ── 撤销/重做（快照栈；树 <200 节点，快照成本可忽略）──

export class IrEditHistory {
  private undoStack: WorkflowStep[][] = []
  private redoStack: WorkflowStep[][] = []
  private limit = 100

  /** 应用 op 前记录当前状态 */
  push(current: WorkflowStep[]): void {
    this.undoStack.push(clone(current))
    if (this.undoStack.length > this.limit) this.undoStack.shift()
    this.redoStack = []
  }

  undo(current: WorkflowStep[]): WorkflowStep[] | null {
    const prev = this.undoStack.pop()
    if (!prev) return null
    this.redoStack.push(clone(current))
    return prev
  }

  redo(current: WorkflowStep[]): WorkflowStep[] | null {
    const next = this.redoStack.pop()
    if (!next) return null
    this.undoStack.push(clone(current))
    return next
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0
  }

  clear(): void {
    this.undoStack = []
    this.redoStack = []
  }
}

// ── 步骤工厂与 id 生成（1.3：{kind_slug}_{6位随机base36}，全局查重兜底）──

const BASE36 = '0123456789abcdefghijklmnopqrstuvwxyz'

export function genStepId(kind: string, existing: Set<string>): string {
  for (let i = 0; i < 64; i++) {
    let suffix = ''
    for (let j = 0; j < 6; j++) suffix += BASE36[Math.floor(Math.random() * 36)]
    const id = `${kind}_${suffix}`
    if (!existing.has(id)) return id
  }
  // 极端碰撞：时间戳兜底，最终由后端 compiler 唯一性检查保证
  return `${kind}_${Date.now().toString(36)}`
}

/** 按 kind 生成新步骤骨架（N 添加菜单 / 复制粘贴） */
export function newStep(kind: string, existing: Set<string>): WorkflowStep {
  const id = genStepId(kind, existing)
  const base = { id, name: '', description: '', on_error: 'abort' as const }
  switch (kind) {
    case 'tool': return { ...base, do: { tool: '', with: {} } }
    case 'seq': return { ...base, do: { seq: [] } }
    case 'loop': return { ...base, do: { loop: { repeat: 1, max: 100, do: [] } } }
    case 'if': return { ...base, do: { if: { condition: { always: true }, then: [], else: [] } } }
    case 'call': return { ...base, do: { call: '', with: {} } }
    case 'wait': return { ...base, do: { wait: '', auto: [] } }
    case 'chat': return { ...base, do: { chat: '', with: {} } }
    case 'script': return { ...base, do: { script: { runtime: 'python', code: '' } } }
    case 'assert': return { ...base, do: { assert: { condition: { always: true } } } }
    case 'mcp': return { ...base, do: { mcp: { server: '', tool: '', with: {} } } }
    case 'sleep': return { ...base, do: { sleep: 1 } }
    case 'break': return { ...base, do: { break: true } }
    case 'continue': return { ...base, do: { continue: true } }
    default: return { ...base, do: { tool: '', with: {} } }
  }
}

/** 复制步骤子树并重新生成全部 id（1.5#10 粘贴防重复） */
export function cloneStepWithNewIds(step: WorkflowStep, existing: Set<string>): WorkflowStep {
  const copy = clone(step)
  const rename = (s: WorkflowStep) => {
    s.id = genStepId(stepKind(s), existing)
    existing.add(s.id)
    const c = containerLanes(s)
    if (c) for (const l of c.lanes) laneSteps(s, l.id).forEach(rename)
  }
  rename(copy)
  return copy
}