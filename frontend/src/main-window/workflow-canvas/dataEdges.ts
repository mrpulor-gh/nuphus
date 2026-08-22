/**
 * dataEdges.ts — {{var}} 扫描：生产者/消费者配对（设计文档 1.4b）
 *
 * 判定规则（与 variables.rs / compiler.rs 对齐）：
 * - 生产者：step.capture == "x" → 产出变量 x
 * - 消费者·模板引用：with/params/script code 等任意字符串（含嵌套）中 {{x}} 或 {{x | pipe}}
 * - 消费者·VarRef 引用：Condition / for_each.items 中 {"var":"x"}（点号路径取根名）
 * - 排除：{{ENV:*}}（环境变量）、{{params.*}}（params.json 外部注入）
 * - 管道：get/json/len/default 管道名记入边 tooltip
 */

import type { WorkflowStep, Condition, VarRef } from '../../core/types'

/** 一次变量消费 */
export interface VarConsumption {
  /** 根变量名（点号路径已取根） */
  varName: string
  /** 管道名列表（如 ["get"] / ["json","len"]） */
  pipes: string[]
}

/** 步骤级扫描结果 */
export interface StepVarProfile {
  stepId: string
  /** capture 产出（最多一个） */
  produces?: string
  /** 消费列表（按出现顺序，可能重复） */
  consumes: VarConsumption[]
}

// compiler.rs:116 同款：\{\{\s*([A-Za-z_]\w*) —— 只取标识符根名
const VAR_REF_RE = /\{\{\s*([A-Za-z_]\w*)((?:[^}]*)?)\}\}/g

/** 被排除的外部注入根名：params.* 由 params.json 注入；ENV:* 正则只捕获到根名 ENV */
function isExternalVar(name: string): boolean {
  return name === 'params' || name === 'ENV'
}

/** 解析管道段：'| get "h" | len' → ['get', 'len'] */
function parsePipes(rest: string): string[] {
  if (!rest) return []
  return rest
    .split('|')
    .map(p => p.trim())
    .filter(Boolean)
    .map(p => {
      const m = p.match(/^([A-Za-z_]\w*)/)
      return m ? m[1] : ''
    })
    .filter(Boolean)
}

/** 扫描单个字符串中的 {{var}} 引用（外部环境变量 ENV:* 不会被正则捕获） */
export function scanTemplateRefs(text: string, out: VarConsumption[]): void {
  if (!text.includes('{{')) return
  VAR_REF_RE.lastIndex = 0
  let m: RegExpExecArray | null
  while ((m = VAR_REF_RE.exec(text)) !== null) {
    const varName = m[1]
    if (isExternalVar(varName)) continue
    out.push({ varName, pipes: parsePipes(m[2] || '') })
  }
}

/** 递归扫描 JSON 值中的字符串模板引用（对齐 compiler.rs scan_refs 遍历） */
function scanJsonRefs(v: unknown, out: VarConsumption[]): void {
  if (typeof v === 'string') {
    scanTemplateRefs(v, out)
  } else if (Array.isArray(v)) {
    for (const item of v) scanJsonRefs(item, out)
  } else if (v && typeof v === 'object') {
    for (const val of Object.values(v as Record<string, unknown>)) scanJsonRefs(val, out)
  }
}

/** VarRef：{"var":"x.y"} 取根名 x；纯字符串字面量做模板扫描 */
function scanVarRef(r: VarRef | undefined, out: VarConsumption[]): void {
  if (r === undefined || r === null) return
  if (typeof r === 'string') {
    scanTemplateRefs(r, out)
    return
  }
  if (typeof r === 'object' && 'var' in r && typeof r.var === 'string') {
    const root = r.var.split('.')[0]
    if (root && !isExternalVar(root) && !root.startsWith('ENV:')) {
      out.push({ varName: root, pipes: [] })
    }
  }
}

/** Condition 12 变体的操作数扫描 */
function scanCondition(cond: Condition | undefined, out: VarConsumption[]): void {
  if (!cond || typeof cond !== 'object') return
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
    const arr = c[key]
    if (Array.isArray(arr)) for (const r of arr) scanVarRef(r as VarRef, out)
  }
  for (const key of ['not_empty', 'empty']) {
    if (key in c) scanVarRef(c[key] as VarRef, out)
  }
}

/** 子步骤数组键（容器体，不作为参数扫描） */
const CONTAINER_KEYS = new Set(['seq', 'do', 'then', 'else', 'auto'])

function scanActionPayload(value: unknown, out: VarConsumption[]): void {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    scanJsonRefs(value, out)
    return
  }
  const obj = value as Record<string, unknown>
  // if 条件 / loop until / loop for_each.items 走 VarRef 语义扫描
  const ifDef = obj.if as Record<string, unknown> | undefined
  if (ifDef) scanCondition(ifDef.condition as Condition, out)
  const loopDef = obj.loop as Record<string, unknown> | undefined
  if (loopDef) {
    if (loopDef.until) scanCondition(loopDef.until as Condition, out)
    const forEach = loopDef.for_each as { items?: VarRef } | undefined
    if (forEach?.items) scanVarRef(forEach.items, out)
  }
  const assertDef = obj.assert as Record<string, unknown> | undefined
  if (assertDef) scanCondition(assertDef.condition as Condition, out)

  // 其余字段做通用模板扫描（跳过容器子步骤数组与已专项处理的条件体）
  for (const [k, v] of Object.entries(obj)) {
    if (CONTAINER_KEYS.has(k)) continue
    if (k === 'if' || k === 'assert') {
      // 已扫描 condition；无其他字符串字段
      continue
    }
    if (k === 'loop' && v && typeof v === 'object') {
      for (const [lk, lv] of Object.entries(v as Record<string, unknown>)) {
        if (lk === 'do' || lk === 'until' || lk === 'for_each') continue
        scanJsonRefs(lv, out)
      }
      continue
    }
    scanJsonRefs(v, out)
  }
}

/** 扫描单个步骤的变量生产/消费画像 */
export function profileStep(step: WorkflowStep): StepVarProfile {
  const consumes: VarConsumption[] = []
  scanActionPayload(step.do, consumes)
  return {
    stepId: step.id,
    produces: typeof step.capture === 'string' && step.capture ? step.capture : undefined,
    consumes,
  }
}

/**
 * 全树遍历序（对齐 compiler.rs 深度优先：step → seq/loop.do/if.then/if.else/wait.auto）。
 * 数据边配对与遮蔽判定都以此顺序为准（"遍历序最近的前序生产者"）。
 */
export function walkSteps(
  steps: WorkflowStep[],
  visit: (step: WorkflowStep, parentId: string | null) => void,
  parentId: string | null = null,
): void {
  for (const step of steps) {
    visit(step, parentId)
    const d = step.do as Record<string, unknown> | undefined
    if (!d || typeof d !== 'object') continue
    if (Array.isArray(d.seq)) walkSteps(d.seq as WorkflowStep[], visit, step.id)
    const loop = d.loop as { do?: WorkflowStep[] } | undefined
    if (loop && Array.isArray(loop.do)) walkSteps(loop.do, visit, step.id)
    const ifDef = d.if as { then?: WorkflowStep[]; else?: WorkflowStep[] } | undefined
    if (ifDef) {
      if (Array.isArray(ifDef.then)) walkSteps(ifDef.then, visit, step.id)
      if (Array.isArray(ifDef.else)) walkSteps(ifDef.else, visit, step.id)
    }
    if (Array.isArray(d.auto)) walkSteps(d.auto as WorkflowStep[], visit, step.id)
  }
}
