/**
 * OutlinePanel.tsx — 画布右下角结构大纲面板（替代 MiniMap）
 *
 * 纯展示组件：steps 树 / 选中 id / 状态 map 全部 props 注入，状态合成在 CanvasPage。
 * - 有步骤：全层级缩进树（kind 图标 + 名称 + 容器「N 步」徽章 + 右侧状态点），
 *   点击行 → onLocate（跨层切换 + 选中 + 闪烁 + 居中由 CanvasPage.locateNode 闭环）
 * - 空工作流：画布操作向导卡（快捷键 kbd 列表）
 * 子步骤提取复用 projection 的 containerLanes/laneSteps（if 双泳道 then/else 均展示，
 * 多泳道容器插入泳道标题行，避免 then/else 两簇混排不可辨）。
 */

import type { WorkflowStep } from '../../core/types'
import { containerLanes, laneSteps, stepKind } from './projection'
import { KIND_ICONS } from './nodes/StepNode'
import { CONTAINER_ICONS } from './nodes/ContainerNode'
import type { StepVisualStatus } from './types'
import { useLanguage } from '../../locales'
import { nodeKindLabel } from './presentation'

/** 大纲状态点取值 = 节点视觉状态全集（snapshot/run_history 合成 + 容器聚合在 CanvasPage 完成） */
export type OutlineDotState = StepVisualStatus['state']

interface OutlinePanelProps {
  /** 编辑缓冲 steps 树（反映未保存修改） */
  steps: WorkflowStep[]
  selectedId: string | null
  /** stepId → 状态点（无状态不渲染点） */
  statuses: ReadonlyMap<string, OutlineDotState>
  onLocate: (stepId: string) => void
}

/** 每级缩进（px） */
const INDENT = 12

const DOT_TITLES: Record<OutlineDotState, [string, string]> = {
  running: ['运行中', 'Running'],
  retrying: ['重试中', 'Retrying'],
  success: ['上次运行成功', 'Previous run succeeded'],
  skipped: ['上次运行跳过', 'Skipped in previous run'],
  error: ['失败', 'Failed'],
  paused: ['已暂停', 'Paused'],
}

/** 容器直接子步骤数（所有泳道合计，对齐 projection.childCount / ContainerNode 徽章） */
function childCount(step: WorkflowStep): number {
  const c = containerLanes(step)
  if (!c) return 0
  return c.lanes.reduce((n, l) => n + laneSteps(step, l.id).length, 0)
}

/** 全树步骤总数（标题「结构 · N 步」） */
function countAll(steps: WorkflowStep[]): number {
  let n = 0
  for (const s of steps) {
    n += 1
    const c = containerLanes(s)
    if (c) for (const l of c.lanes) n += countAll(laneSteps(s, l.id))
  }
  return n
}

interface OutlineRowProps {
  step: WorkflowStep
  depth: number
  selectedId: string | null
  statuses: ReadonlyMap<string, OutlineDotState>
  onLocate: (stepId: string) => void
}

function OutlineRow({ step, depth, selectedId, statuses, onLocate }: OutlineRowProps) {
  const { t, lang } = useLanguage()
  const kind = stepKind(step)
  const container = containerLanes(step, t)
  const Icon = container ? CONTAINER_ICONS[container.kind] : (KIND_ICONS[kind] ?? KIND_ICONS.custom)
  const state = statuses.get(step.id)
  const multiLane = !!container && container.lanes.length > 1
  return (
    <>
      <button
        type="button"
        className={`wfc-outline-row${selectedId === step.id ? ' is-selected' : ''}`}
        style={{ paddingLeft: `calc(var(--space-2) + ${depth * INDENT}px)` }}
        title={step.name || step.id}
        onClick={() => onLocate(step.id)}
      >
        <span
          className="wfc-outline-icon"
          data-kind={kind}
          title={t('workflowCanvas.node.type', nodeKindLabel(kind, t))}
        >
          <Icon size={12} aria-hidden="true" />
        </span>
        <span className="wfc-outline-name">{step.name || step.id}</span>
        {container && (
          <span className="wfc-badge wfc-badge--count">
            {childCount(step)} {lang === 'zh' ? '步' : childCount(step) === 1 ? 'step' : 'steps'}
          </span>
        )}
        {state && (
          <span
            className={`wfc-dot wfc-dot--${state}`}
            title={DOT_TITLES[state][lang === 'zh' ? 0 : 1]}
          />
        )}
      </button>
      {container?.lanes.map(lane => (
        <div key={lane.id}>
          {multiLane && (
            <div
              className="wfc-outline-lane"
              style={{ paddingLeft: `calc(var(--space-2) + ${(depth + 1) * INDENT}px)` }}
            >
              {lane.title}
            </div>
          )}
          {laneSteps(step, lane.id).map(child => (
            <OutlineRow
              key={child.id}
              step={child}
              depth={depth + 1}
              selectedId={selectedId}
              statuses={statuses}
              onLocate={onLocate}
            />
          ))}
        </div>
      ))}
    </>
  )
}

/** 空工作流向导（与 CanvasPage 键盘表/面包屑 hint 一一对应） */
const GUIDE: { keys: string[]; desc: [string, string] }[] = [
  { keys: ['双击'], desc: ['容器进入子层', 'Open a container'] },
  { keys: ['Alt', '←'], desc: ['返回父层', 'Go to parent layer'] },
  { keys: ['N'], desc: ['添加步骤', 'Add a step'] },
  { keys: ['Enter'], desc: ['编辑选中节点', 'Edit selected node'] },
  { keys: ['Delete'], desc: ['删除选中', 'Delete selection'] },
  { keys: ['Ctrl', 'S'], desc: ['保存', 'Save'] },
  { keys: ['Ctrl', 'Z'], desc: ['撤销 / 重做', 'Undo / redo'] },
  { keys: ['R'], desc: ['运行', 'Run'] },
]

export function OutlinePanel({ steps, selectedId, statuses, onLocate }: OutlinePanelProps) {
  const { lang } = useLanguage()
  const languageIndex = lang === 'zh' ? 0 : 1
  if (steps.length === 0) {
    return (
      <div className="wfc-outline">
        <div className="wfc-outline-head">{lang === 'zh' ? '画布操作' : 'Canvas shortcuts'}</div>
        <div className="wfc-outline-guide">
          {GUIDE.map(g => (
            <div key={g.desc[0]} className="wfc-outline-guide-row">
              <span className="wfc-outline-guide-keys">
                {g.keys.map(k => (
                  <kbd key={k} className="kbd">
                    {k === '双击' && lang !== 'zh' ? 'Double-click' : k}
                  </kbd>
                ))}
              </span>
              <span className="wfc-outline-guide-desc">{g.desc[languageIndex]}</span>
            </div>
          ))}
        </div>
      </div>
    )
  }
  return (
    <div className="wfc-outline">
      <div className="wfc-outline-head">
        {lang === 'zh'
          ? `结构 · ${countAll(steps)} 步`
          : `Outline · ${countAll(steps)} ${countAll(steps) === 1 ? 'step' : 'steps'}`}
      </div>
      <div className="wfc-outline-tree">
        {steps.map(s => (
          <OutlineRow
            key={s.id}
            step={s}
            depth={0}
            selectedId={selectedId}
            statuses={statuses}
            onLocate={onLocate}
          />
        ))}
      </div>
    </div>
  )
}
