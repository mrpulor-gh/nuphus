/**
 * intentTypes.ts — 意图表单数据模型（画布入口全量 / 对话入口单阶段共享语义）
 *
 * 设计铁律（docs/intent-form-spec.md）：
 * - 仅前端表单内部使用；协议出口按 §3/§4 序列化，不落盘为独立格式。
 * - 纯文本意图、无工具参数；不修改 recorder 代码。
 * - 单条意图字符不设硬上限（不 maxLength 硬截断），阶段/子步骤/阶段名上限走 INTENT_FORM_LIMITS。
 */

/** 单个子步骤：纯文本意图 */
export interface IntentStep {
  /** 本地 key（列表排序/删除定位）；不进入序列化协议 */
  id: string
  /** 必填。用户语言描述这一步做什么（字符不设硬上限，超长仅软提示） */
  intent: string
}

/** 一个阶段 = 大步骤（画布侧对应 seq 容器）
 *  conversation 入口的 step_form 只返回"当前阶段"：
 *  该阶段 name 由 WorkflowAgent 自填，子步骤由用户补录。 */
export interface IntentStage {
  /** 本地 key；不进入序列化协议 */
  id: string
  /** 阶段名（画布容器 name）。上限 INTENT_FORM_LIMITS.maxStageNameLen */
  name: string
  /** 顺序子步骤 */
  steps: IntentStep[]
}

/** 意图表单整体（画布入口全量 / 对话入口单阶段都可归约为此形状） */
export interface IntentForm {
  /** 目标工作流名（画布入口预填 ir.name；对话入口由 Agent 在 prompt/上下文给出） */
  workflowName: string
  stages: IntentStage[]
}

/** 上限常量（大王拍板：意图字符不硬限；阶段 ≤8 / 子步骤 ≤12 / 阶段名 ≤40） */
export const INTENT_FORM_LIMITS = {
  maxStages: 8, // 阶段上限
  maxStepsPerStage: 12, // 每阶段子步骤上限
  maxStageNameLen: 40, // 阶段名上限
} as const
