/**
 * 工作流执行状态胶囊（WorkflowRunCard）：顶部悬浮单行展示，主题反色背景。
 *
 * - 收起态：一条胶囊横条（状态图标 + 标题 + 步骤进度 + 展开箭头 + 移除 X），
 *   不占消息流空间，悬浮于聊天区顶部（sticky）。
 * - 点击胶囊 → 展开面板（步骤列表 + 遥控按钮 暂停/继续/终止）；再次点击收起。
 * - 首次出现自动展开（用户能立刻看到步骤），用户手动收起后保持选择。
 * - X 任意时刻可移除（执行中也行）：清除预览不影响后端执行，
 *   下一轮 run_started 自动重新出现（store workflowDismissed 逻辑）。
 * - 完成态：胶囊显示「已完成」，仍可展开查看步骤 + X 移除。
 */

import { useEffect, useState } from 'react'
import { Check, ChevronDown, Circle, Loader2, Pause, Play, Square, X } from 'lucide-react'
import type { WorkflowRunStep } from '../../core/types'
import { t } from '../i18n'

export interface WorkflowRunState {
  steps: WorkflowRunStep[]
  lastWorkflowId?: string
  isPaused: boolean
  /** run_completed 置 true：胶囊显示完成态（隐藏控制按钮，步骤列表保留供查看） */
  done: boolean
  message?: string
}

interface Props {
  workflowRun: WorkflowRunState
  /** 控制请求进行中（禁用按钮防重复提交） */
  busy?: boolean
  onPause: () => void
  onResume: () => void
  onTerminate: () => void
  /** 移除胶囊（X）：清除 workflowRun，本轮不再重建；下一轮 run_started 重新出现 */
  onDismiss?: () => void
}

/** 步骤状态图标映射（running 呼吸 / completed 绿 / failed 红） */
function StatusIcon({ status }: { status: WorkflowRunStep['status'] }) {
  switch (status) {
    case 'running':
      return <Loader2 size={14} className="mobile-wf-step-icon is-running" aria-hidden="true" />
    case 'completed':
      return <Check size={14} className="mobile-wf-step-icon is-completed" aria-hidden="true" />
    case 'failed':
      return <X size={14} className="mobile-wf-step-icon is-failed" aria-hidden="true" />
    case 'paused':
      return <Pause size={14} className="mobile-wf-step-icon is-paused" aria-hidden="true" />
    default:
      return <Circle size={12} className="mobile-wf-step-icon is-pending" aria-hidden="true" />
  }
}

export default function WorkflowRunCard({
  workflowRun,
  busy,
  onPause,
  onResume,
  onTerminate,
  onDismiss,
}: Props) {
  const { steps, isPaused, done, message } = workflowRun
  // 首次挂载自动展开（能立刻看到步骤）；用户手动收起后保持选择（状态在组件实例内保留，
  // workflow_event 更新 props 不重置；workflow_clear 卸载、下一轮 run_started 重挂载再自动展开）
  const [expanded, setExpanded] = useState(true)
  const hasFailed = steps.some(s => s.status === 'failed')
  const finished = steps.filter(s => s.status === 'completed' || s.status === 'failed').length
  const hasSteps = steps.length > 0

  // 展开/收起时无需额外副作用；纯 CSS 面板显隐
  const toggle = () => setExpanded(v => !v)

  // 主状态图标：失败 > 完成 > 暂停 > 运行
  const headIcon = done ? (
    <Check size={16} className="mobile-wf-pill-icon is-completed" aria-hidden="true" />
  ) : hasFailed ? (
    <X size={16} className="mobile-wf-pill-icon is-failed" aria-hidden="true" />
  ) : isPaused ? (
    <Pause size={16} className="mobile-wf-pill-icon is-paused" aria-hidden="true" />
  ) : (
    <Loader2 size={16} className="mobile-wf-pill-icon is-running" aria-hidden="true" />
  )

  const title = done ? t('mobile.wfCompleted') : isPaused ? t('mobile.wfPaused') : t('mobile.wfRunning')

  return (
    <div className={`mobile-wf ${expanded ? 'is-expanded' : 'is-collapsed'}`} role="status">
      {/* ── 胶囊条：单行展示，点击展开/收起 ── */}
      <button
        type="button"
        className="mobile-wf-pill"
        aria-expanded={expanded}
        aria-label={expanded ? t('mobile.wfCollapse') : t('mobile.wfExpand')}
        onClick={toggle}
      >
        {headIcon}
        <span className="mobile-wf-pill-title">{title}</span>
        {hasSteps && (
          <span className="mobile-wf-pill-progress">
            {finished}/{steps.length}
          </span>
        )}
        <ChevronDown
          size={14}
          className={`mobile-wf-pill-chevron ${expanded ? 'is-open' : ''}`}
          aria-hidden="true"
        />
        {/* 移除：任意时刻可关（执行中也可），不影响后端执行 */}
        {onDismiss && (
          <span
            role="button"
            tabIndex={0}
            className="mobile-wf-close"
            aria-label={t('mobile.wfClose')}
            onClick={e => {
              e.stopPropagation()
              onDismiss()
            }}
            onKeyDown={e => {
              if ((e.key === 'Enter' || e.key === ' ') && onDismiss) {
                e.stopPropagation()
                onDismiss()
              }
            }}
          >
            <X size={14} aria-hidden="true" />
          </span>
        )}
      </button>

      {/* ── 展开面板：步骤列表 + 错误信息 + 遥控按钮 ── */}
      {expanded && (
        <div className="mobile-wf-panel">
          {hasSteps ? (
            <ul className="mobile-wf-steps">
              {steps.map(s => (
                <li key={s.id} className="mobile-wf-step">
                  <StatusIcon status={s.status} />
                  <span className="mobile-wf-step-name">{s.name}</span>
                  {typeof s.depth === 'number' && s.depth > 0 && (
                    <span className="mobile-wf-step-depth">· {s.depth}</span>
                  )}
                </li>
              ))}
            </ul>
          ) : (
            <div className="mobile-wf-empty">
              <Loader2 size={13} className="is-running" aria-hidden="true" />
              {t('mobile.wfWaitingSteps')}
            </div>
          )}

          {message && <div className="mobile-wf-error">{message}</div>}

          {!done && (
            <div className="mobile-wf-actions">
              {isPaused ? (
                <button
                  type="button"
                  className="mobile-wf-btn is-primary"
                  disabled={busy}
                  onClick={onResume}
                >
                  <Play size={14} aria-hidden="true" />
                  {t('mobile.wfResume')}
                </button>
              ) : (
                <button
                  type="button"
                  className="mobile-wf-btn is-primary"
                  disabled={busy}
                  onClick={onPause}
                >
                  <Pause size={14} aria-hidden="true" />
                  {t('mobile.wfPause')}
                </button>
              )}
              <button
                type="button"
                className="mobile-wf-btn is-danger"
                disabled={busy}
                onClick={onTerminate}
              >
                <Square size={13} aria-hidden="true" />
                {t('mobile.wfTerminate')}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
