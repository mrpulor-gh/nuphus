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
 * | 取消 / 点遮罩关闭 | 关闭不发送，保留本机草稿 |
 *
 * 纯文本意图：单条子步骤字符不设硬上限（不 maxLength 截断），自然输入。
 */
import { useEffect, useState } from 'react'
import { useLanguage } from '../../locales'
import { listDataDirs } from '../lib/api'
import { intentDraftKey, readIntentDraft, writeIntentDraft } from './intentDraft'
import { Plus, Trash2, X } from 'lucide-react'
import type { IntentForm, IntentStage, IntentStep } from './intentTypes'
import { INTENT_FORM_LIMITS } from './intentTypes'
import './intent-form.css'

interface IntentFormPanelProps {
  /** 目标工作流名（画布入口预填 ir.name） */
  initialName: string
  workflowId: string
  /** 提交（不含空行子步骤）；父层负责关闭弹层 + 保存画布 + dispatch append-to-chat */
  onSubmit: (form: IntentForm) => Promise<boolean> | boolean | void
  /** 关闭（不发送，保留草稿） */
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

export function IntentFormPanel({
  initialName,
  workflowId,
  onSubmit,
  onClose,
}: IntentFormPanelProps) {
  const { t } = useLanguage()
  const [stages, setStages] = useState<IntentStage[]>(() => [newStage()])
  const [draftKey, setDraftKey] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [storageError, setStorageError] = useState('')
  const [submitError, setSubmitError] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [confirmClear, setConfirmClear] = useState(false)
  useEffect(() => {
    let alive = true
    void listDataDirs()
      .then(dirs => {
        const workspace = dirs?.find(dir => dir.key === 'plugin')?.path
        if (!workspace) throw new Error('Workspace unavailable')
        const key = intentDraftKey(workspace, workflowId)
        const saved = readIntentDraft(localStorage, key)
        if (!alive) return
        if (saved) setStages(saved)
        setDraftKey(key)
      })
      .catch(error => {
        if (alive) setStorageError(String(error))
      })
      .finally(() => {
        if (alive) setLoading(false)
      })
    return () => {
      alive = false
    }
  }, [workflowId])
  useEffect(() => {
    if (!draftKey || loading) return
    try {
      writeIntentDraft(localStorage, draftKey, stages)
      setStorageError('')
    } catch (error) {
      setStorageError(String(error))
    }
  }, [draftKey, loading, stages])
  const close = () => {
    if (!submitting) onClose()
  }

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
  const canSubmit = !loading && !submitting && hasValidStage && !hasStepsWithoutName

  const submitDisabledTitle = hasStepsWithoutName
    ? t('workflowEditor.intent.missingName')
    : hasValidStage
      ? t('workflowEditor.intent.submitHint')
      : t('workflowEditor.intent.missingSteps')

  const handleSubmit = async () => {
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
    setSubmitting(true)
    setSubmitError('')
    try {
      if ((await onSubmit({ workflowName: initialName.trim(), stages: cleaned })) === false)
        setSubmitError(t('workflowEditor.intent.submitError', ''))
    } catch (error) {
      setSubmitError(t('workflowEditor.intent.submitError', String(error)))
    } finally {
      setSubmitting(false)
    }
  }

  const stagesFull = stages.length >= INTENT_FORM_LIMITS.maxStages

  return (
    <div className="wfc-intent-mask" onClick={close}>
      <div
        className="wfc-intent"
        role="dialog"
        aria-modal="true"
        aria-label={t('workflowEditor.intent.title')}
        onClick={e => e.stopPropagation()}
      >
        <div className="wfc-intent-head">
          <div>
            <h3 className="wfc-intent-title">{t('workflowEditor.intent.title')}</h3>
            <div className="wfc-intent-sub">
              {t('workflowEditor.intent.target')}：
              <span className="wfc-intent-wf" title={initialName}>
                {initialName || t('workflowEditor.intent.unnamed')}
              </span>
            </div>
          </div>
          <button
            type="button"
            className="wfc-icon-btn"
            onClick={close}
            title={t('workflowEditor.intent.close')}
            disabled={submitting}
          >
            <X size={15} />
          </button>
        </div>

        <div className="wfc-intent-body">
          {loading && <p role="status">{t('workflowEditor.intent.loading')}</p>}
          {storageError && (
            <p role="alert">{t('workflowEditor.intent.storageError', storageError)}</p>
          )}
          {submitError && <p role="alert">{submitError}</p>}
          <fieldset
            disabled={loading || submitting}
            style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}
          >
            {stages.length === 0 ? (
              <div className="wfc-intent-empty">{t('workflowEditor.intent.empty')}</div>
            ) : (
              stages.map((stage, si) => {
                const stepsFull = stage.steps.length >= INTENT_FORM_LIMITS.maxStepsPerStage
                return (
                  <div className="wfc-intent-stage" key={stage.id}>
                    <div className="wfc-intent-stage-head">
                      <span className="wfc-intent-stage-no">
                        {t('workflowEditor.intent.stage', String(si + 1))}
                      </span>
                      <input
                        className="wfc-intent-stage-name"
                        value={stage.name}
                        maxLength={INTENT_FORM_LIMITS.maxStageNameLen}
                        placeholder={t('workflowEditor.intent.stageName')}
                        onChange={e => updateStageName(stage.id, e.target.value)}
                      />
                      <button
                        type="button"
                        className="wfc-icon-btn"
                        title={t('workflowEditor.intent.removeStage')}
                        onClick={() => removeStage(stage.id)}
                      >
                        <Trash2 size={13} />
                      </button>
                    </div>

                    {stage.steps.length === 0 ? (
                      <div className="wfc-intent-step-empty">
                        {t('workflowEditor.intent.noSteps')}
                      </div>
                    ) : (
                      stage.steps.map((step, stepIndex) => (
                        <div className="wfc-intent-step-row" key={step.id}>
                          <input
                            className="wfc-intent-step-input"
                            value={step.intent}
                            placeholder={t(
                              'workflowEditor.intent.step',
                              `${si + 1}.${stepIndex + 1}`,
                            )}
                            onChange={e => updateStepIntent(stage.id, step.id, e.target.value)}
                          />
                          <button
                            type="button"
                            className="wfc-icon-btn"
                            title={t('workflowEditor.intent.removeStep')}
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
                          ? t(
                              'workflowEditor.intent.maxSteps',
                              String(INTENT_FORM_LIMITS.maxStepsPerStage),
                            )
                          : t('workflowEditor.intent.addStep')
                      }
                      onClick={() => addStepRow(stage.id)}
                    >
                      <Plus size={13} /> {t('workflowEditor.intent.addStep')}
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
                stagesFull
                  ? t('workflowEditor.intent.maxStages', String(INTENT_FORM_LIMITS.maxStages))
                  : t('workflowEditor.intent.addStage')
              }
              onClick={addStage}
            >
              <Plus size={13} /> {t('workflowEditor.intent.addStage')}
            </button>
          </fieldset>
        </div>

        <div className="wfc-intent-foot">
          <span className="wfc-intent-foot-hint">{t('workflowEditor.intent.hint')}</span>
          <button
            type="button"
            className="wfc-btn"
            disabled={loading || submitting}
            onClick={() => {
              if (!confirmClear) {
                setConfirmClear(true)
                return
              }
              setStages([newStage()])
              setConfirmClear(false)
            }}
          >
            {t(confirmClear ? 'workflowEditor.intent.clearConfirm' : 'workflowEditor.intent.clear')}
          </button>
          <button type="button" className="wfc-btn" onClick={close} disabled={submitting}>
            {t('workflowEditor.intent.close')}
          </button>
          <button
            type="button"
            className="wfc-btn wfc-btn--primary"
            disabled={!canSubmit}
            title={submitDisabledTitle}
            onClick={handleSubmit}
          >
            {t(submitting ? 'workflowEditor.intent.submitting' : 'workflowEditor.intent.submit')}
          </button>
        </div>
      </div>
    </div>
  )
}
