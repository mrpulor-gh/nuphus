/**
 * ProblemsPanel.tsx — 校验问题列表（设计文档 3.3）
 * 底部可折叠（类 VSCode Problems）：级别 / 节点 / 规则描述 / 定位。
 * 双标签页：前端校验（L2 实时）/ 后端校验（L3 保存阻断原文）。
 */

import { useMemo, useState } from 'react'
import { ChevronDown, ChevronUp, CircleAlert, TriangleAlert, Crosshair } from 'lucide-react'
import type { Problem } from './validate'

interface ProblemsPanelProps {
  problems: Problem[]
  /** 后端报告原文（wf_save 阻断 / 手动"检查"） */
  backendReport: { errors: string[]; warnings: string[] } | null
  /** 定位：下钻到节点所在层 + 居中闪烁 */
  onLocate: (stepId: string) => void
  /** 节点显示名解析 */
  nameOf: (stepId: string) => string
}

export function ProblemsPanel({ problems, backendReport, onLocate, nameOf }: ProblemsPanelProps) {
  const [collapsed, setCollapsed] = useState(false)
  const [tab, setTab] = useState<'local' | 'backend'>('local')

  const errorCount = useMemo(
    () => problems.filter(p => p.level === 'error').length + (backendReport?.errors.length ?? 0),
    [problems, backendReport],
  )
  const warnCount = useMemo(
    () =>
      problems.filter(p => p.level === 'warning').length + (backendReport?.warnings.length ?? 0),
    [problems, backendReport],
  )

  return (
    <div className={`wfc-problems${collapsed ? ' is-collapsed' : ''}`}>
      <div className="wfc-problems-bar">
        <button type="button" className="wfc-problems-toggle" onClick={() => setCollapsed(c => !c)}>
          {collapsed ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
          <span>问题</span>
          {errorCount > 0 && (
            <span className="wfc-problems-count wfc-problems-count--error">
              <CircleAlert size={11} /> {errorCount}
            </span>
          )}
          {warnCount > 0 && (
            <span className="wfc-problems-count wfc-problems-count--warning">
              <TriangleAlert size={11} /> {warnCount}
            </span>
          )}
          {errorCount === 0 && warnCount === 0 && <span className="wfc-problems-ok">无问题</span>}
        </button>
        <div className="wfc-problems-tabs">
          <button
            type="button"
            className={`wfc-chip${tab === 'local' ? ' is-active' : ''}`}
            onClick={() => setTab('local')}
          >
            前端校验
          </button>
          <button
            type="button"
            className={`wfc-chip${tab === 'backend' ? ' is-active' : ''}`}
            onClick={() => setTab('backend')}
          >
            后端校验
            {backendReport
              ? `（${backendReport.errors.length + backendReport.warnings.length}）`
              : ''}
          </button>
        </div>
      </div>

      {!collapsed && (
        <div className="wfc-problems-list">
          {tab === 'local' && problems.length === 0 && (
            <div className="wfc-problems-empty">结构校验通过（保存时将由后端做权威校验）</div>
          )}
          {tab === 'local' &&
            problems.map((p, i) => (
              <div
                className={`wfc-problem wfc-problem--${p.level}`}
                key={`${p.rule}-${p.stepId ?? ''}-${i}`}
              >
                <span className="wfc-problem-icon">
                  {p.level === 'error' ? <CircleAlert size={12} /> : <TriangleAlert size={12} />}
                </span>
                <span className="wfc-problem-rule">{p.rule}</span>
                <span className="wfc-problem-node">{p.stepId ? nameOf(p.stepId) : '—'}</span>
                <span className="wfc-problem-msg" title={p.message}>
                  {p.message}
                </span>
                {p.stepId && (
                  <button
                    type="button"
                    className="wfc-icon-btn"
                    title="定位到节点"
                    onClick={() => onLocate(p.stepId!)}
                  >
                    <Crosshair size={12} />
                  </button>
                )}
              </div>
            ))}
          {tab === 'backend' &&
            (backendReport ? (
              <>
                {backendReport.errors.map((m, i) => (
                  <div className="wfc-problem wfc-problem--error" key={`be-${i}`}>
                    <span className="wfc-problem-icon">
                      <CircleAlert size={12} />
                    </span>
                    <span className="wfc-problem-rule">L3</span>
                    <span className="wfc-problem-msg" title={m}>
                      {m}
                    </span>
                  </div>
                ))}
                {backendReport.warnings.map((m, i) => (
                  <div className="wfc-problem wfc-problem--warning" key={`bw-${i}`}>
                    <span className="wfc-problem-icon">
                      <TriangleAlert size={12} />
                    </span>
                    <span className="wfc-problem-rule">L3</span>
                    <span className="wfc-problem-msg" title={m}>
                      {m}
                    </span>
                  </div>
                ))}
                {backendReport.errors.length === 0 && backendReport.warnings.length === 0 && (
                  <div className="wfc-problems-empty">后端权威校验通过</div>
                )}
              </>
            ) : (
              <div className="wfc-problems-empty">尚未运行过后端校验（保存或点击「检查」触发）</div>
            ))}
        </div>
      )}
    </div>
  )
}
