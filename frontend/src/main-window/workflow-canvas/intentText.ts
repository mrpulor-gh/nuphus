/**
 * intentText.ts — 画布意图表单 → WorkflowAgent 纯文本模板（纯函数）
 *
 * 协议：buildIntentTextTemplate 产出的人类可读纯文本，经 CanvasPage dispatch
 * 'nuphus:append-to-chat' 注入聊天 workflow 输入框，由 WorkflowAgent 按文本理解。
 * 序列化样例见 docs/intent-form-spec.md §3（简单 3 步 / 含阶段嵌套 / 混合）。
 *
 * 渲染规则：
 * - 子步骤 trim 去空行；无任何有效子步骤的阶段不进入模板。
 * - 全部阶段均未命名 → 所有有效子步骤整体编号平铺（场景 1「简单 3 步」）。
 * - 存在命名阶段 → 阶段块结构化渲染：`阶段N「name」` + `- 子步骤`，
 *   阶段块之间空行分隔（场景 2/3）；未命名但有步骤的阶段以 `阶段N（未命名）` 兜底。
 */

import type { IntentForm } from './intentTypes'

export function buildIntentTextTemplate(
  form: IntentForm,
  wid: string,
  fallbackName?: string | null,
): string {
  const workflowName = form.workflowName?.trim() || fallbackName?.trim() || '未命名工作流'
  const header = `[意图表单→工作流] 请把以下意图整理为可执行 V2 工作流「${workflowName}」（id=${wid}）：`
  const footer = `要求：意图解析、步骤前置与重置、循环合并、异常分支、缺失参数补偿；\n若某步涉及界面操作细节不确定，请先在实际界面探索跑通再固化；\n完成后覆写 plugin/workflows/${wid}/workflow.json（保留 id=${wid}）并跑通验证。`

  const stages = form.stages ?? []
  const named = stages.some(s => s.name.trim().length > 0)

  // 无命名阶段 → 简单平铺（场景 1）：所有阶段的有效子步骤连续编号
  if (!named) {
    const lines: string[] = []
    for (const st of stages) {
      for (const step of st.steps ?? []) {
        const t = step.intent.trim()
        if (t) lines.push(t)
      }
    }
    if (lines.length === 0) return `${header}\n\n${footer}`
    return `${header}\n${lines.map((t, i) => `${i + 1}. ${t}`).join('\n')}\n\n${footer}`
  }

  // 存在命名阶段 → 阶段块渲染（场景 2/3）
  const blocks: string[] = []
  let stageNo = 0
  for (const st of stages) {
    const valid = (st.steps ?? []).map(s => s.intent.trim()).filter(Boolean)
    if (valid.length === 0) continue // 空阶段（无有效子步骤）不进模板
    stageNo += 1
    const name = st.name.trim()
    const title = name ? `阶段${stageNo}「${name}」` : `阶段${stageNo}（未命名）`
    blocks.push(`${title}\n${valid.map(t => `- ${t}`).join('\n')}`)
  }
  if (blocks.length === 0) return `${header}\n\n${footer}`
  return `${header}\n\n${blocks.join('\n\n')}\n\n${footer}`
}
