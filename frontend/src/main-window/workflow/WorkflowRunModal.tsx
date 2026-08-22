import { useState } from 'react'
import { IconWorkflow, IconChevronDown, IconChevronRight } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { CompactModal } from '../layout/CompactModal'
import type { WorkflowItem, WorkflowStep, Action } from '../../core/types'
import '../../styles/workflow-modal.css'

interface WorkflowRunModalProps {
  open: boolean
  workflow: WorkflowItem | null
  onRun: (id: string) => void
  onCancel: () => void
  running?: boolean
}

/** 获取步骤类型标签 */
function stepKindLabel(do_: Action): string {
  if ('tool' in do_) return 'Tool'
  if ('seq' in do_) return 'Seq'
  if ('loop' in do_) return 'Loop'
  if ('if' in do_) return 'If'
  if ('call' in do_) return 'Call'
  if ('wait' in do_) return 'Wait'
  if ('chat' in do_) return 'Chat'
  if ('script' in do_) return 'Script'
  if ('assert' in do_) return 'Assert'
  if ('mcp' in do_) return 'MCP'
  if ('sleep' in do_) return 'Sleep'
  if ('break' in do_) return 'Break'
  if ('continue' in do_) return 'Continue'
  return ''
}

/** 获取嵌套子步骤（V2 do：seq / loop.do / if.then / if.else / wait.auto） */
function getChildren(step: WorkflowStep): WorkflowStep[] {
  const d = step.do as any
  if (d?.seq) return d.seq
  if (d?.loop?.do) return d.loop.do
  if (d?.if?.then) return d.if.then
  if (d?.if?.else) return d.if.else
  // wait: { wait: string, auto: WorkflowStep[] }
  if (d?.auto) return d.auto
  return []
}

/** 获取循环信息 */
function getLoopInfo(step: WorkflowStep): string | null {
  const d = step.do as any
  const loop = d?.loop
  if (!loop) return null
  if (loop.repeat != null) return `重复 ${loop.repeat} 次`
  if (loop.for_each) return `遍历 ${loop.for_each.as || loop.for_each.items?.var || 'item'}`
  if (loop.until) return `循环直到条件满足`
  if (loop.max != null) return `最多 ${loop.max} 次`
  return null
}

export function WorkflowRunModal({
  open,
  workflow,
  onRun,
  onCancel,
  running,
}: WorkflowRunModalProps) {
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null)

  if (!open || !workflow) return null

  return (
    <CompactModal
      open={open}
      onClose={onCancel}
      title={`工作流 · ${workflow.steps.length} 步`}
      icon={<IconWorkflow size={14} />}
      size="auto"
      footer={
        <>
          <div className="wcf-footer-left">
            <Button variant="ghost" size="sm" onClick={onCancel}>
              取消
            </Button>
          </div>
          <div className="wcf-footer-right">
            <Button
              variant="primary"
              size="sm"
              loading={running}
              onClick={() => onRun(workflow.id)}
            >
              启动
            </Button>
          </div>
        </>
      }
    >
      {/* 步骤列表 — 保留原有的 step card 结构 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        {workflow.steps.map((step, i) => {
          const children = getChildren(step)
          const hasKids = children.length > 0
          const kind = stepKindLabel(step.do)
          const loopInfo = getLoopInfo(step)
          return (
            <div key={step.id || i} className={`wcf-step-card ${hasKids ? 'has-children' : ''}`}>
              <div
                className="wcf-step-header"
                onClick={() => hasKids && setExpandedIdx(prev => (prev === i ? null : i))}
              >
                <div
                  className="wcf-step-header-inner"
                  style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}
                >
                  {hasKids ? (
                    <span className="wcf-chevron">
                      {expandedIdx === i ? (
                        <IconChevronDown size={12} />
                      ) : (
                        <IconChevronRight size={12} />
                      )}
                    </span>
                  ) : (
                    <span className="wcf-step-dot" />
                  )}
                  <span className="wcf-step-name">{step.name}</span>
                </div>
                {kind && (
                  <span
                    style={{
                      fontSize: 11,
                      color: 'var(--spark-dim)',
                      fontFamily: 'var(--font-mono)',
                      flexShrink: 0,
                    }}
                  >
                    {kind}
                  </span>
                )}
              </div>

              {step.description && (
                <div
                  style={{
                    padding: '0 14px 10px',
                    fontSize: 12,
                    color: 'var(--spark-secondary)',
                    lineHeight: 1.5,
                  }}
                >
                  {step.description}
                </div>
              )}

              {hasKids && expandedIdx === i && (
                <div className="wcf-children">
                  {children.map((child, ci) => (
                    <div
                      key={child.id || ci}
                      className="wcf-child-row"
                      style={{ cursor: 'default', padding: '4px 0' }}
                    >
                      <span className="wcf-child-index">{ci + 1}.</span>
                      <span className="wcf-step-name" style={{ fontSize: 13 }}>
                        {child.name}
                      </span>
                    </div>
                  ))}
                  {loopInfo && <div className="wcf-loop-info">{loopInfo}</div>}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </CompactModal>
  )
}
