/**
 * ExecIndicator — 聊天区左上角「Exec 执行指示器」（圆形头像 + 执行状态）
 *
 * 回答一个问题：Leader 派发内部子 agent（`task_dispatch`）时，当前在跑什么状态。
 * 头像为主体、右下角一枚固定 `E` 角标（社交平台好友消息风格），点击进执行追踪面板。
 *
 * ── 数据源（单一，不另造状态判断）────────────────────────────
 * `TaskRun[]` = 后端 `agent::task_run` 台账的本轮快照（`NuphusEvent::TaskRuns`
 * → `useEvents` 的 `task_runs` 分支 → `useSession.taskRuns`）。与 TaskBubble 面板、
 * 执行面板 task 行**同一份数据**：台账每次变迁推全量快照，前端是纯投影
 * （不推断 id、不配对事件、不维护状态机）。
 *
 * 为什么不用 `timeline` 里的 tool_call / task 行推断：那是混合流（thinking / text /
 * tool_call / 历史行交叠），要判断「哪一笔在跑」只能靠 `task-${run_id}` 前缀试配对 ——
 * 正是后端注释里点名删掉的「软耦合」。台账已把 lifecycle 收敛成权威字段，直接读它。
 *
 * ── 状态机 ──────────────────────────────────────────────────
 *   idle      本轮无派发                 → 不渲染（没有任何状态可展示）
 *   running   有 running 一笔            → 环绕动画（见 exec-indicator.css）
 *   completed 本轮有派发且全部结清        → 环绕停止成闭环 + 完成勾，并起 60s 退场计时
 *
 * 60s 计时锚点 = 「最新结清那一笔」的 `run_id`（`completionMark`）：
 *   · 新的 task_dispatch 派发 = 新 run_id → phase 回 running，计时随 effect 重建而重置；
 *   · 下一笔结清 = mark 变化 → 计重新从该笔结清起算 60s；
 *   · 组件卸载 → effect cleanup 清掉两个定时器（不留无人管理的裸 setTimeout）。
 * 结清到进入完成态之间不计「已过时间」——后台台账不会为纯展示需求记观察时刻，
 * 而「进入完成态后 60s」正是需求口径。
 *
 * 退场动画时长与 JS 定时器共用同一个常量（经 CSS 变量注入，见 `EXEC_LEAVE_MS`），
 * 避免「CSS 改了动画时长、JS 还按旧值卸载」的漂移。
 */

import { useEffect, useState, type CSSProperties } from 'react'
import type { TaskRun } from '../../core/types'
import { IconCheck } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import '../../styles/exec-indicator.css'

/** 完成态驻留时长：结清后这段时间内没有新派发即退场（需求口径 60s） */
export const EXEC_DONE_HOLD_MS = 60_000

/** 退场（向左移出）动画时长：注入 CSS 变量 `--exec-leave-duration`，JS/CSS 单一来源 */
export const EXEC_LEAVE_MS = 420

export type ExecIndicatorPhase = 'idle' | 'running' | 'completed'

/**
 * 台账快照 → 指示器阶段（**唯一判断点**，组件与测试共用）。
 *
 * Nuphus 串行执行是铁律（同轮内一笔结清才开下一笔），故同时 running 的至多一笔，
 * `some(running)` 即等价于「最新一笔在跑」——不引入「并行」表述。
 */
export function execIndicatorPhase(runs: readonly TaskRun[]): ExecIndicatorPhase {
  if (runs.some(r => r.state === 'running')) return 'running'
  return runs.length > 0 ? 'completed' : 'idle'
}

/**
 * 「最新结清的一笔」身份 = 60s 退场计时的锚点（无则空串）。
 *
 * 台账保序（创建序 = 展示序，见 src/agent/task_run.rs:100），故从尾部取第一条非 running。
 * run_id 进程内单调且不复用 → 新派发必然换掉这个值，计时得以判定「该不该重置」。
 */
export function completionMark(runs: readonly TaskRun[]): string {
  for (let i = runs.length - 1; i >= 0; i--) {
    if (runs[i].state !== 'running') return runs[i].run_id
  }
  return ''
}

/** 退场进度：`mark` 是「哪一笔结清」的身份 —— 换了笔的陈旧条目一律忽略 */
type Retire = { mark: string; step: 'leaving' | 'gone' }

export interface ExecIndicatorProps {
  /** ExecAgent 执行生命周期快照（后端 task_runs 台账的本轮投影） */
  runs: readonly TaskRun[]
  /** 点击头像 → 打开执行追踪面板（App 层沿用既有 showExecTrace 开关） */
  onOpenTrace: () => void
}

export function ExecIndicator({ runs, onOpenTrace }: ExecIndicatorProps) {
  const { t } = useLanguage()
  const phase = execIndicatorPhase(runs)
  const mark = completionMark(runs)
  const [retire, setRetire] = useState<Retire | null>(null)

  useEffect(() => {
    // 执行中 / 本轮无派发：清计时并回到「在场」——新派发必须能立刻把完成态拉回执行中
    if (phase !== 'completed') {
      setRetire(null)
      return
    }
    const hold = setTimeout(() => setRetire({ mark, step: 'leaving' }), EXEC_DONE_HOLD_MS)
    const gone = setTimeout(
      () => setRetire({ mark, step: 'gone' }),
      EXEC_DONE_HOLD_MS + EXEC_LEAVE_MS,
    )
    // 依赖变化（新派发 / 下一笔结清）与组件卸载都走这里：定时器不会逃逸
    return () => {
      clearTimeout(hold)
      clearTimeout(gone)
    }
  }, [phase, mark])

  if (phase === 'idle') return null

  const step = retire?.mark === mark ? retire.step : null
  if (phase === 'completed' && step === 'gone') return null

  return (
    <button
      type="button"
      className={`exec-indicator is-${phase}${step === 'leaving' ? ' is-leaving' : ''}`}
      // 退场动画时长由 TS 常量注入：CSS 只消费，不另写一份数值
      style={{ '--exec-leave-duration': `${EXEC_LEAVE_MS}ms` } as CSSProperties}
      onClick={onOpenTrace}
      aria-label={t(phase === 'running' ? 'execIndicator.running' : 'execIndicator.completed')}
      title={t('execIndicator.openTrace')}
    >
      {/* 品牌字标 = 项目 logo（frontend/public/nuphus-n.svg 的三条路径，逐字同形）。
          此处必须**内联**而非 <img src>：stroke=currentColor 要跟随主题色，
          <img> 无法继承 CSS color（与 mobile/components/MessageList.tsx 同做法）。 */}
      <svg className="exec-indicator-logo" viewBox="0 0 256 256" fill="none" aria-hidden="true">
        <g stroke="currentColor" strokeWidth={24} strokeLinecap="round" fill="none">
          <path d="M64 20 H192 A44 44 0 0 1 236 64 V156" />
          <path d="M200 236 H64 A44 44 0 0 1 20 192 V64 A44 44 0 0 1 64 20" />
          <path d="M80 180 L80 76 M176 180 L176 76" />
        </g>
      </svg>

      {phase === 'completed' && (
        <span className="exec-indicator-check" aria-hidden="true">
          <IconCheck size={10} strokeWidth={3} />
        </span>
      )}

      {/* 固定字母角标：标识这是 **E**xec 的执行指示器（不随状态换字母） */}
      <span className="exec-indicator-badge" aria-hidden="true">
        E
      </span>
    </button>
  )
}
