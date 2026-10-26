/**
 * ExecIndicator —— 聊天区左上角「Exec 执行指示器」的状态契约。
 *
 * 钉住的不变量（对应任务验收项）：
 *  ① 台账有 running 一笔 → 环绕态（is-running），此时不显示完成勾
 *  ② 本轮全部结清 → 完成态（is-completed + 完成勾），环绕态退出
 *  ③ 点击头像 → 打开执行追踪面板（回调一次）
 *  ④ 完成态 60s 无新派发 → 向左移出（is-leaving）→ 动画结束从 DOM 消失；
 *     期间有新派发 → 回执行中且**计时重置**（原 60s 边界不再退场，下一笔结清重新起算）
 *  ⑤ 退场计时受管：组件卸载后不留残留定时器（不是无人管理的裸 setTimeout）
 *
 * 时间一律 fake timers（真等 60s 不可接受）。断言走 class 而不是像素/动画帧：
 * jsdom 不做布局、不跑 CSS 动画，状态语义只能由类名 + DOM 结构表达
 * （弧条 recipe 本身由 styles/exec-indicator.test.ts 按源码钉住）。
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TaskRun } from '../../core/types'
import {
  EXEC_DONE_HOLD_MS,
  EXEC_LEAVE_MS,
  ExecIndicator,
  completionMark,
  execIndicatorPhase,
} from './ExecIndicator'

/** 运行时加载 Node 的 fs（模块名变量拼接：tsconfig 未启 @types/node，字面量会 TS2307） */
const { readFileSync } = (await import('node:' + 'fs')) as {
  readFileSync: (p: string, enc: string) => string
}

function run(over: Partial<TaskRun> = {}): TaskRun {
  return {
    run_id: 'run-1',
    title: '整理本周会议纪要',
    task: '## 任务\n整理本周会议纪要并输出 Markdown',
    goal_type: 'file_operation',
    origin: null,
    attempt: 1,
    state: 'running',
    started_at: 1000,
    settled_at: null,
    duration_ms: null,
    ok: null,
    summary: null,
    ...over,
  }
}

/** 已结清一笔（成功） */
const settled = (over: Partial<TaskRun> = {}): TaskRun =>
  run({ state: 'completed', settled_at: 9000, duration_ms: 8000, ok: true, ...over })

/** 当前头像根元素的类名；未渲染即抛（免得 `?.` 把「消失」误判成通过） */
function cls(): string {
  const el = document.querySelector('.exec-indicator')
  if (!el) throw new Error('.exec-indicator 未渲染')
  return el.className
}

const mounted = () => document.querySelector('.exec-indicator') !== null

describe('ExecIndicator（聊天区左上角 Exec 执行指示器）', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    cleanup()
    vi.useRealTimers()
  })

  it('本轮无派发 → 不渲染（没有状态可展示，不占界面、不进 tab 序）', () => {
    render(<ExecIndicator runs={[]} onOpenTrace={() => {}} />)
    expect(mounted()).toBe(false)
  })

  it('① 台账有 running 一笔 → 环绕态，且不带完成勾', () => {
    const { container } = render(<ExecIndicator runs={[run()]} onOpenTrace={() => {}} />)
    expect(cls()).toContain('is-running')
    expect(cls()).not.toContain('is-completed')
    // 环绕环由 .exec-indicator::before 画（jsdom 无伪元素节点），故这里钉「态」而非「环」；
    // 环的 recipe 见 styles/exec-indicator.test.ts
    expect(container.querySelector('.exec-indicator-check')).toBeNull()
    // 固定字母角标恒在
    expect(container.querySelector('.exec-indicator-badge')?.textContent).toBe('E')
    expect(container.querySelector('.exec-indicator-logo')).not.toBeNull()
  })

  it('② 本轮全部结清 → 完成态（环绕退出 + 完成勾出现）', () => {
    const { container } = render(<ExecIndicator runs={[settled()]} onOpenTrace={() => {}} />)
    expect(cls()).toContain('is-completed')
    expect(cls()).not.toContain('is-running')
    expect(container.querySelector('.exec-indicator-check')).not.toBeNull()
    // 完成态仍要有主体：头像不能被完成标识吃掉
    expect(container.querySelector('.exec-indicator-logo')).not.toBeNull()
  })

  it('③ 点击头像 → 打开执行追踪面板（回调一次）', () => {
    const onOpenTrace = vi.fn()
    render(<ExecIndicator runs={[run()]} onOpenTrace={onOpenTrace} />)
    fireEvent.click(document.querySelector('.exec-indicator') as HTMLElement)
    expect(onOpenTrace).toHaveBeenCalledTimes(1)
  })

  it('④-a 完成态 60s 无新派发 → 向左移出 → 动画结束从 DOM 消失', () => {
    render(<ExecIndicator runs={[settled()]} onOpenTrace={() => {}} />)

    act(() => {
      vi.advanceTimersByTime(EXEC_DONE_HOLD_MS - 1)
    })
    expect(cls()).not.toContain('is-leaving') // 59.999s：还在完成态

    act(() => {
      vi.advanceTimersByTime(1)
    })
    expect(cls()).toContain('is-leaving') // 到点：向左移出（动画进行中）

    act(() => {
      vi.advanceTimersByTime(EXEC_LEAVE_MS)
    })
    expect(mounted()).toBe(false) // 动画结束：真正消失（不留不可见的死按钮）
  })

  it('④-b 期间有新派发 → 回执行中并重置计时（原 60s 边界不再退场）', () => {
    const onOpenTrace = () => {}
    const { rerender } = render(<ExecIndicator runs={[settled()]} onOpenTrace={onOpenTrace} />)

    act(() => {
      vi.advanceTimersByTime(EXEC_DONE_HOLD_MS / 2) // t=30s：计时走了一半
    })

    // 新派发：run-2 开跑 → 立刻回环绕态（不是等原计时到点才动）
    rerender(
      <ExecIndicator runs={[settled(), run({ run_id: 'run-2' })]} onOpenTrace={onOpenTrace} />,
    )
    expect(cls()).toContain('is-running')

    // 越过原来那一笔的 60s 边界：必须仍在（计时已随新派发重置）
    act(() => {
      vi.advanceTimersByTime(EXEC_DONE_HOLD_MS / 2) // t=60s
    })
    expect(mounted()).toBe(true)
    expect(cls()).toContain('is-running')

    // run-2 结清 → 完成态从这一刻重新起算 60s
    rerender(
      <ExecIndicator runs={[settled(), settled({ run_id: 'run-2' })]} onOpenTrace={onOpenTrace} />,
    )
    expect(cls()).toContain('is-completed')

    act(() => {
      vi.advanceTimersByTime(EXEC_DONE_HOLD_MS - 1)
    })
    expect(cls()).not.toContain('is-leaving') // 新一轮计时的 59.999s：不早退

    act(() => {
      vi.advanceTimersByTime(1)
    })
    expect(cls()).toContain('is-leaving') // 满 60s 才退场
  })

  it('⑤ 退场计时受管：完成态卸载后不留残留定时器', () => {
    const { unmount } = render(<ExecIndicator runs={[settled()]} onOpenTrace={() => {}} />)
    expect(vi.getTimerCount()).toBeGreaterThan(0) // 完成态确实起了计时
    unmount()
    expect(vi.getTimerCount()).toBe(0) // 卸载即清 —— 定时器不会逃逸
  })
})

/**
 * 头像内容 = 项目自带 logo。契约按源码钉（同 styles/exec-indicator.test.ts 的理由）：
 * 这里比对的是「内联的三条 path 与 public/nuphus-n.svg 逐字同形」——内联是必须的
 * （stroke=currentColor 要继承主题色，<img> 做不到），代价是复制体可能悄悄漂移，
 * 故用源文件做锚，asset 一改测试就报。
 *
 * 两个路径都相对 vitest 工作目录（frontend/）：jsdom 环境的 import.meta.url 不是 file:
 * 协议，不能用它拼路径（实测报 "The URL must be of scheme file"）。
 */
describe('⑥ 头像内容 = 项目自带 logo（与 public/nuphus-n.svg 逐字同形）', () => {
  it('内联路径的 d 与 SVG 源文件完全一致（含圆角框 + 两竖线的 N 形三条）', () => {
    const svg = readFileSync('public/nuphus-n.svg', 'utf8')
    const tsx = readFileSync('src/main-window/chat/ExecIndicator.tsx', 'utf8')
    const ds = [...svg.matchAll(/\sd="([^"]+)"/g)].map(m => m[1])
    expect(ds).toHaveLength(3)
    for (const d of ds) expect(tsx).toContain(d)
  })
})

describe('ExecIndicator 状态派生（唯一判断点，组件与测试共用）', () => {
  it('阶段：无派发 = idle / 有 running = running / 全结清 = completed', () => {
    expect(execIndicatorPhase([])).toBe('idle')
    expect(execIndicatorPhase([run()])).toBe('running')
    expect(execIndicatorPhase([settled()])).toBe('completed')
    // 串行执行铁律：同轮内一笔结清才开下一笔 —— 有 running 就是执行中
    expect(execIndicatorPhase([settled(), run({ run_id: 'run-2' })])).toBe('running')
  })

  it('失败 / 中断同样算「结清」：不把终态漏判成执行中', () => {
    expect(execIndicatorPhase([run({ state: 'failed', ok: false })])).toBe('completed')
    expect(execIndicatorPhase([run({ state: 'interrupted' })])).toBe('completed')
  })

  it('退场锚点 = 最新结清那一笔的 run_id（新派发/下一笔结清都会换掉它）', () => {
    expect(completionMark([])).toBe('')
    expect(completionMark([run()])).toBe('') // 还在跑：没有可锚定的结清笔
    expect(completionMark([settled({ run_id: 'run-1' })])).toBe('run-1')
    expect(completionMark([settled({ run_id: 'run-1' }), run({ run_id: 'run-2' })])).toBe('run-1')
    expect(completionMark([settled({ run_id: 'run-1' }), settled({ run_id: 'run-2' })])).toBe(
      'run-2',
    )
  })
})
