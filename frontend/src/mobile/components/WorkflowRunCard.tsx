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
import {
  Check as CheckIcon,
  Circle as CircleIcon,
  Pause as PauseIcon,
  Play as PlayIcon,
  X as XIcon,
} from 'lucide'
import { ChevronDown, Loader2, Square, X } from 'lucide-react'
import { MorphIcon } from 'morphicons/react'
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

/** 步骤状态图标映射（running 呼吸 / completed 绿 / failed 红）。
 *  状态切换位用 MorphIcon：换图标即形变（spring snappy），
 *  running 保留 Loader2 + CSS 旋转（帧动画，非形态变换）。 */
function StatusIcon({ status }: { status: WorkflowRunStep['status'] }) {
  if (status === 'running') {
    return <Loader2 size={14} className="mobile-wf-step-icon is-running" aria-hidden="true" />
  }
  const [icon, tone, size] =
    status === 'paused'
      ? [PauseIcon, 'is-paused', 14]
      : status === 'completed'
        ? [CheckIcon, 'is-completed', 14]
        : status === 'failed'
          ? [XIcon, 'is-failed', 14]
          : [CircleIcon, 'is-pending', 12]
  return (
    <MorphIcon icon={icon} size={size} spring="snappy" className={`mobile-wf-step-icon ${tone}`} />
  )
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

  // 主状态图标：失败 > 完成 > 暂停 > 运行（切换位走 MorphIcon 形变；
  // running 保留 Loader2 + CSS 旋转——帧动画，非形态变换）
  const headIconData = done ? CheckIcon : hasFailed ? XIcon : isPaused ? PauseIcon : null
  const headTone = done
    ? 'is-completed'
    : hasFailed
      ? 'is-failed'
      : isPaused
        ? 'is-paused'
        : 'is-running'
  const headIcon = headIconData ? (
    <MorphIcon
      icon={headIconData}
      size={16}
      spring="snappy"
      className={`mobile-wf-pill-icon ${headTone}`}
    />
  ) : (
    <Loader2 size={16} className="mobile-wf-pill-icon is-running" aria-hidden="true" />
  )

  const title = done
    ? t('mobile.wfCompleted')
    : isPaused
      ? t('mobile.wfPaused')
      : t('mobile.wfRunning')

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
                  <MorphIcon icon={PlayIcon} size={14} spring="snappy" />
                  {t('mobile.wfResume')}
                </button>
              ) : (
                <button
                  type="button"
                  className="mobile-wf-btn is-primary"
                  disabled={busy}
                  onClick={onPause}
                >
                  <MorphIcon icon={PauseIcon} size={14} spring="snappy" />
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
