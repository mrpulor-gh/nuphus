/**
 * IntentFormPanel.tsx — 画布「意图表单」弹层（阶段卡片 + 子步骤行列表）
 *
 * 设计参照（docs/intent-form-spec.md §5 + RecDraftPanel 行内编辑模式）：
 * - 阶段 = 卡片、子步骤 = 行内列表；每行子步骤一个 input + 删除按钮。
 * - 主按钮永远可见但可禁用：不满足条件时 disabled + title 给原因。
 * - 空态/超限/取消均有明确行为，无静默丢弃（详见下表）。
 *
 * 边界行为：
 * | 场景 | 行为 |
 * |------|------|
 * | 全部阶段为空 / 全空白 | 发送按钮 disabled，title 解释 |
 * | 阶段有 name 但无有效子步骤 | 该阶段视为空容器，模板侧剔除；仅当无任何有效阶段时整体 disabled |
 * | 有子步骤但阶段 name 为空 | 发送按钮 disabled，title 提示阶段名必填（不静默丢弃该阶段内容） |
 * | 超限（阶段>8 / 子步骤>12 / 阶段名>40） | 添加按钮 disabled + title；阶段名 input maxLength 硬限 |
 * | 空行子步骤 | 提交前 trim().filter(Boolean)，不影响其他有效项 |
 * | 取消 / 点遮罩关闭 | 直接关闭不发送、不持久化（本地 state 丢弃） |
 *
 * 纯文本意图：单条子步骤字符不设硬上限（不 maxLength 截断），自然输入。
 */
import { useState } from 'react'
import { Plus, Trash2, X } from 'lucide-react'
import type { IntentForm, IntentStage, IntentStep } from './intentTypes'
import { INTENT_FORM_LIMITS } from './intentTypes'
import './intent-form.css'

interface IntentFormPanelProps {
  /** 目标工作流名（画布入口预填 ir.name） */
  initialName: string
  /** 提交（不含空行子步骤）；父层负责关闭弹层 + 保存画布 + dispatch append-to-chat */
  onSubmit: (form: IntentForm) => void
  /** 取消 / 关闭（不发送、不持久化） */
  onClose: () => void
}

let localSeq = 0
function nextId(): string {
  localSeq += 1
  return `intent-${Date.now().toString(36)}-${localSeq}`
}

function newStep(): IntentStep {
  return { id: nextId(), intent: '' }
}

function newStage(): IntentStage {
  return { id: nextId(), name: '', steps: [newStep()] }
}

export function IntentFormPanel({ initialName, onSubmit, onClose }: IntentFormPanelProps) {
  const [stages, setStages] = useState<IntentStage[]>(() => [newStage()])

  const updateStageName = (stageId: string, name: string) => {
    setStages(prev => prev.map(s => (s.id === stageId ? { ...s, name } : s)))
  }

  const addStepRow = (stageId: string) => {
    setStages(prev =>
      prev.map(s => {
        if (s.id !== stageId) return s
        if (s.steps.length >= INTENT_FORM_LIMITS.maxStepsPerStage) return s
        return { ...s, steps: [...s.steps, newStep()] }
      }),
    )
  }

  const removeStepRow = (stageId: string, stepId: string) => {
    setStages(prev =>
      prev.map(s => (s.id === stageId ? { ...s, steps: s.steps.filter(x => x.id !== stepId) } : s)),
    )
  }

  const updateStepIntent = (stageId: string, stepId: string, intent: string) => {
    setStages(prev =>
      prev.map(s =>
        s.id === stageId
          ? { ...s, steps: s.steps.map(x => (x.id === stepId ? { ...x, intent } : x)) }
          : s,
      ),
    )
  }

  const addStage = () => {
    setStages(prev => (prev.length >= INTENT_FORM_LIMITS.maxStages ? prev : [...prev, newStage()]))
  }

  const removeStage = (stageId: string) => {
    setStages(prev => prev.filter(s => s.id !== stageId))
  }

  const hasValidStage = stages.some(s => {
    const name = s.name.trim()
    if (!name) return false
    return s.steps.some(x => x.intent.trim().length > 0)
  })
  // 有子步骤但阶段名空的阶段：不能提交（阶段名必填），否则会静默丢弃该阶段内容
  const hasStepsWithoutName = stages.some(s => {
    if (s.name.trim()) return false
    return s.steps.some(x => x.intent.trim().length > 0)
  })
  const canSubmit = hasValidStage && !hasStepsWithoutName

  const submitDisabledTitle = hasStepsWithoutName
    ? '存在填了子步骤但未填名称的阶段：阶段名必填（可写「阶段1」「阶段2」占位）'
    : hasValidStage
      ? '将下方阶段与子步骤整理为意图文本，交给 WorkflowAgent 生成工作流'
      : '至少填写一个阶段的名称与一个子步骤'

  const handleSubmit = () => {
    if (!canSubmit) return
    // 提交协议：仅序列化有效内容（name + 有效子步骤 trim 后保留；空行剔除）
    const cleaned: IntentStage[] = stages
      .map(st => ({
        id: st.id,
        name: st.name.trim(),
        steps: st.steps
          .map(x => ({ id: x.id, intent: x.intent.trim() }))
          .filter(x => x.intent.length > 0),
      }))
      .filter(st => st.name.length > 0 && st.steps.length > 0)
    onSubmit({ workflowName: initialName.trim(), stages: cleaned })
  }

  const stagesFull = stages.length >= INTENT_FORM_LIMITS.maxStages

  return (
    <div className="wfc-intent-mask" onClick={onClose}>
      <div className="wfc-intent" onClick={e => e.stopPropagation()}>
        <div className="wfc-intent-head">
          <div>
            <h3 className="wfc-intent-title">意图表单</h3>
            <div className="wfc-intent-sub">
              目标工作流：
              <span className="wfc-intent-wf" title={initialName}>
                {initialName || '未命名工作流'}
              </span>
            </div>
          </div>
          <button type="button" className="wfc-icon-btn" onClick={onClose} title="取消并关闭">
            <X size={15} />
          </button>
        </div>

        <div className="wfc-intent-body">
          {stages.length === 0 ? (
            <div className="wfc-intent-empty">暂无阶段 —— 点击下方「+ 添加阶段」开始描述</div>
          ) : (
            stages.map((stage, si) => {
              const stepsFull = stage.steps.length >= INTENT_FORM_LIMITS.maxStepsPerStage
              return (
                <div className="wfc-intent-stage" key={stage.id}>
                  <div className="wfc-intent-stage-head">
                    <span className="wfc-intent-stage-no">阶段 {si + 1}</span>
                    <input
                      className="wfc-intent-stage-name"
                      value={stage.name}
                      maxLength={INTENT_FORM_LIMITS.maxStageNameLen}
                      placeholder="阶段名称（必填，如：登录管理后台）"
                      onChange={e => updateStageName(stage.id, e.target.value)}
                    />
                    <button
                      type="button"
                      className="wfc-icon-btn"
                      title="删除该阶段及其全部子步骤"
                      onClick={() => removeStage(stage.id)}
                    >
                      <Trash2 size={13} />
                    </button>
                  </div>

                  {stage.steps.length === 0 ? (
                    <div className="wfc-intent-step-empty">暂无子步骤</div>
                  ) : (
                    stage.steps.map((step, stepIndex) => (
                      <div className="wfc-intent-step-row" key={step.id}>
                        <input
                          className="wfc-intent-step-input"
                          value={step.intent}
                          placeholder={`子步骤 ${si + 1}.${stepIndex + 1}：描述这一步做什么`}
                          onChange={e => updateStepIntent(stage.id, step.id, e.target.value)}
                        />
                        <button
                          type="button"
                          className="wfc-icon-btn"
                          title="删除该子步骤"
                          onClick={() => removeStepRow(stage.id, step.id)}
                        >
                          <X size={13} />
                        </button>
                      </div>
                    ))
                  )}

                  <button
                    type="button"
                    className="wfc-intent-add-row"
                    disabled={stepsFull}
                    title={
                      stepsFull
                        ? `每阶段最多 ${INTENT_FORM_LIMITS.maxStepsPerStage} 个子步骤`
                        : '添加一个子步骤'
                    }
                    onClick={() => addStepRow(stage.id)}
                  >
                    <Plus size={13} /> 添加子步骤
                  </button>
                </div>
              )
            })
          )}

          <button
            type="button"
            className="wfc-intent-add-stage"
            disabled={stagesFull}
            title={
              stagesFull ? `最多 ${INTENT_FORM_LIMITS.maxStages} 个阶段` : '添加一个阶段（大步骤）'
            }
            onClick={addStage}
          >
            <Plus size={13} /> 添加阶段
          </button>
        </div>

        <div className="wfc-intent-foot">
          <span className="wfc-intent-foot-hint">
            纯文本描述子步骤意图，WorkflowAgent 将解析为可执行工作流
          </span>
          <button type="button" className="wfc-btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="wfc-btn wfc-btn--primary"
            disabled={!canSubmit}
            title={submitDisabledTitle}
            onClick={handleSubmit}
          >
            发送给 WorkflowAgent 生成工作流
          </button>
        </div>
      </div>
    </div>
  )
}
