/**
 * validate.ts — L2 前端镜像校验（设计文档 3.2，V1–V14）
 *
 * 定位：体验前置，不是安全边界。纯结构规则 1:1 镜像 compiler.rs；
 * 环境依赖规则（工具注册表 / script runtime 白名单 / call 目标存在性与循环链）
 * 仅 L3 后端可做（wf_validate / wf_save）。
 */

import type { WorkflowStep, Condition, VarRef, RunRecord } from '../../core/types'
import { stepKind, containerLanes, laneSteps } from './projection'
import { walkSteps, profileStep } from './dataEdges'

export type ProblemLevel = 'error' | 'warning'

export interface Problem {
  level: ProblemLevel
  /** 规则号（V1–V14） */
  rule: string
  message: string
  /** 关联步骤（ProblemsPanel 定位用） */
  stepId?: string
}

const NAME_RE = /^[a-zA-Z_][a-zA-Z0-9_.]*$/

/** 镜像 Ctx.captured：遍历序前向已捕获集合 + loop item_var 作用域 */
interface MirrorCtx {
  ids: Map<string, number>
  captured: Set<string>
  problems: Problem[]
  inLoop: boolean
  loopVars: string[]
  hasCustom: boolean
}

function conditionOperands(cond: Condition): { op: string; operands: VarRef[] } | null {
  const c = cond as Record<string, unknown>
  for (const key of [
    'equals',
    'not_equals',
    'contains',
    'starts_with',
    'regex',
    'gt',
    'lt',
    'gte',
    'lte',
  ]) {
    if (key in c) return { op: key, operands: Array.isArray(c[key]) ? (c[key] as VarRef[]) : [] }
  }
  for (const key of ['not_empty', 'empty']) {
    if (key in c) return { op: key, operands: [c[key] as VarRef] }
  }
  if ('always' in c) return { op: 'always', operands: [] }
  return null
}

function checkCondition(cond: Condition, owner: string, stepId: string, ctx: MirrorCtx): void {
  const parsed = conditionOperands(cond)
  if (!parsed) {
    ctx.problems.push({
      level: 'error',
      rule: 'V7',
      message: `步骤「${owner}」条件为空或无法识别`,
      stepId,
    })
    return
  }
  const { op, operands } = parsed
  // V7：操作数数量
  if (op === 'not_empty' || op === 'empty') {
    if (operands.length !== 1) {
      ctx.problems.push({
        level: 'error',
        rule: 'V7',
        message: `步骤「${owner}」条件 ${op} 需要 1 个操作数`,
        stepId,
      })
    }
  } else if (op !== 'always' && operands.length < 2) {
    ctx.problems.push({
      level: 'error',
      rule: 'V7',
      message: `步骤「${owner}」条件 ${op} 需要至少 2 个操作数`,
      stepId,
    })
  }
  // V6：regex 可编译（取字面量操作数）
  if (op === 'regex') {
    for (const r of operands) {
      if (typeof r === 'string') {
        try {
          new RegExp(r)
        } catch {
          ctx.problems.push({
            level: 'error',
            rule: 'V6',
            message: `步骤「${owner}」正则不可编译: ${r}`,
            stepId,
          })
        }
      }
    }
  }
  // V4：VarRef 前向引用（warning）
  for (const r of operands) {
    if (r && typeof r === 'object' && 'var' in r) {
      const root = r.var.split('.')[0]
      if (
        root &&
        !ctx.captured.has(root) &&
        !ctx.loopVars.includes(root) &&
        root !== '_index' &&
        root !== 'params' &&
        root !== 'ENV'
      ) {
        ctx.problems.push({
          level: 'warning',
          rule: 'V4',
          message: `步骤「${owner}」条件变量「${root}」尚未被先前步骤捕获（运行时可能由 inputs 注入）`,
          stepId,
        })
      }
    }
  }
}

function validateStep(step: WorkflowStep, ctx: MirrorCtx): void {
  const kind = stepKind(step)
  const owner = step.name || step.id || '(未命名)'

  // V1：id 非空 + 全局唯一
  if (!step.id) {
    ctx.problems.push({
      level: 'error',
      rule: 'V1',
      message: `步骤「${owner}」缺少 id`,
      stepId: step.id,
    })
  } else {
    const n = (ctx.ids.get(step.id) ?? 0) + 1
    ctx.ids.set(step.id, n)
    if (n > 1) {
      ctx.problems.push({
        level: 'error',
        rule: 'V1',
        message: `步骤 id「${step.id}」重复（断点续连依赖全局唯一）`,
        stepId: step.id,
      })
    }
  }

  // V13：Action::Custom（旧格式）
  if (kind === 'custom') {
    ctx.hasCustom = true
    ctx.problems.push({
      level: 'error',
      rule: 'V13',
      message: `步骤「${owner}」是旧格式（不兼容 V2），画布只读。请迁移为 V2 格式。`,
      stepId: step.id,
    })
  }

  // V9：capture 命名规范
  if (step.capture && !NAME_RE.test(step.capture)) {
    ctx.problems.push({
      level: 'error',
      rule: 'V9',
      message: `步骤「${owner}」capture 名「${step.capture}」不合规（^[a-zA-Z_][a-zA-Z0-9_.]*$）`,
      stepId: step.id,
    })
  }

  // V3：break/continue 必须在 loop 内
  if ((kind === 'break' || kind === 'continue') && !ctx.inLoop) {
    ctx.problems.push({
      level: 'error',
      rule: 'V3',
      message: `「${kind}」仅可在循环（loop）内使用`,
      stepId: step.id,
    })
  }

  const d = step.do as Record<string, unknown>

  // kind 专项
  if (kind === 'if') {
    const def = d.if as { condition?: Condition }
    if (def?.condition) checkCondition(def.condition, owner, step.id, ctx)
  }
  if (kind === 'assert') {
    const def = d.assert as { condition?: Condition }
    if (def?.condition) checkCondition(def.condition, owner, step.id, ctx)
  }
  if (kind === 'loop') {
    const def = d.loop as {
      for_each?: { items?: VarRef; as?: string }
      repeat?: number
      until?: Condition
      max?: number
    }
    // V8：max 边界
    const max = def?.max ?? 100
    if (max < 1 || max > 10000) {
      ctx.problems.push({
        level: 'warning',
        rule: 'V8',
        message: `步骤「${owner}」loop.max=${max} 超出建议范围 1..=10000`,
        stepId: step.id,
      })
    }
    // V5：for_each items 已被捕获
    if (def?.for_each) {
      const items = def.for_each.items
      if (!def.for_each.as) {
        // ForEachDef.as 为空（运行时默认 item）——不报错，仅 V5 检查 items
      }
      if (items && typeof items === 'object' && 'var' in items) {
        const root = items.var.split('.')[0]
        if (root && !ctx.captured.has(root) && root !== 'params' && root !== 'ENV') {
          ctx.problems.push({
            level: 'warning',
            rule: 'V5',
            message: `步骤「${owner}」for_each 引用的「${root}」尚未被捕获（运行时缺失将静默空循环）`,
            stepId: step.id,
          })
        }
      }
    }
    if (def?.until) checkCondition(def.until, owner, step.id, ctx)
  }
  if (kind === 'call') {
    // V12：call 目标非空
    if (typeof d.call !== 'string' || !d.call) {
      ctx.problems.push({
        level: 'error',
        rule: 'V12',
        message: `步骤「${owner}」call 目标 workflow_id 为空`,
        stepId: step.id,
      })
    }
  }

  // V4：模板 {{var}} 前向引用（warning；params/ENV/loop item_var 除外）
  for (const cons of profileStep(step).consumes) {
    if (ctx.captured.has(cons.varName)) continue
    if (ctx.loopVars.includes(cons.varName) || cons.varName === '_index') continue
    ctx.problems.push({
      level: 'warning',
      rule: 'V4',
      message: `步骤「${owner}」引用变量「${cons.varName}」尚未被先前步骤捕获（运行时可能由 inputs 注入）`,
      stepId: step.id,
    })
  }

  // V2：容器子步骤非空（else 允许为空）
  const c = containerLanes(step)
  if (c) {
    for (const lane of c.lanes) {
      if (lane.id === 'else') continue
      if (laneSteps(step, lane.id).length === 0) {
        ctx.problems.push({
          level: 'error',
          rule: 'V2',
          message: `容器「${owner}」的${lane.id === 'main' ? '子步骤' : 'THEN 分支'}为空（至少保留一个步骤）`,
          stepId: step.id,
        })
      }
    }
  }

  // capture 登记（遍历序前向）+ V10 遮蔽
  if (step.capture) {
    if (ctx.captured.has(step.capture)) {
      ctx.problems.push({
        level: 'warning',
        rule: 'V10',
        message: `变量「${step.capture}」被重复捕获，先前值将被遮蔽`,
        stepId: step.id,
      })
    }
    ctx.captured.add(step.capture)
  }

  // 递归子树（loop 上下文携带 item_var）
  if (c) {
    const wasInLoop = ctx.inLoop
    const prevLoopVars = ctx.loopVars
    if (kind === 'loop') {
      ctx.inLoop = true
      const def = d.loop as { for_each?: { as?: string } }
      ctx.loopVars = [...ctx.loopVars, def?.for_each?.as || 'item', '_index']
    }
    for (const lane of c.lanes) {
      for (const child of laneSteps(step, lane.id)) validateStep(child, ctx)
    }
    ctx.inLoop = wasInLoop
    ctx.loopVars = prevLoopVars
  }
}

export interface ValidateOptions {
  runHistory?: RunRecord[]
}

/** L2 全量镜像校验（防抖由调用方控制） */
export function validateIR(steps: WorkflowStep[], opts: ValidateOptions = {}): Problem[] {
  const ctx: MirrorCtx = {
    ids: new Map(),
    captured: new Set(),
    problems: [],
    inLoop: false,
    loopVars: [],
    hasCustom: false,
  }
  for (const step of steps) validateStep(step, ctx)

  // V14：编辑后 run_history 断点记录失配（有已完成记录引用的 id 已不存在）
  const last = opts.runHistory?.[0]
  if (last && Array.isArray(last.steps) && last.steps.length > 0) {
    const recs = last.steps as { step_id?: string; status?: unknown }[]
    const missing = recs.filter(r => r.step_id && !ctx.ids.has(r.step_id))
    if (missing.length > 0) {
      ctx.problems.push({
        level: 'warning',
        rule: 'V14',
        message: `结构已变更：${missing.length} 条历史断点记录引用的步骤已不存在，续跑将从变化处重新执行而非精确续点`,
      })
    }
  }
  return ctx.problems
}
