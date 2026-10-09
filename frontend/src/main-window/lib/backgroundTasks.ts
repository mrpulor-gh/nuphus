// backgroundTasks — 后台任务面板的**纯逻辑**与「账本可能变了」信号总线
//
// 这里刻意只放纯函数 + 极小的发布订阅：
//   · 纯函数（formatElapsed / reportHasKillFailure）能被单测直接钉死，
//     不必把 React 渲染 + IPC 一起拖进 jsdom；
//   · 信号总线让 `useAgentControl.handleInterrupt` 能在**不改动 useSession
//     依赖装配**（那条链路上有他人在途改动）的前提下，通知已挂载的面板去刷新。
//     代价是多一个模块级订阅者 —— 但它只有一个消费者（App 里那个面板），
//     且挂载/卸载都走标准订阅清理，不存在悬挂监听。

/**
 * 后端 interrupt 汇报里的「未能终止」标记。
 *
 * 为什么用中文子串做判定而不是结构化字段：后端 `interrupt` 返回的是给人读的
 * 事实句（`lifecycle.rs`），它承诺**永不出现**「已全部停止」这类撒谎措辞，
 * 但并没有为前端预留机器可解析的结构。前端要做的只是「这句话里有没有失败面」，
 * 子串判定足够；万一后端改写文案，最坏结果是降级为 info 提示（少一次警告），
 * 而不会误报——方向是安全的。
 */
const KILL_FAILURE_MARK = '未能终止'

/** 汇报句里是否存在「有进程没杀掉」的事实（有则必须以 warning 呈现） */
export function reportHasKillFailure(report: string | null | undefined): boolean {
  return typeof report === 'string' && report.includes(KILL_FAILURE_MARK)
}

/**
 * 已运行时长本地格式化：`45s` / `12m31s` / `2h05m`。
 *
 * 用后端给的 `elapsed_ms` 而不是前端自算 `now - started_at_ms`：两者都会随
 * 列表刷新而变，但只有前者与后端「已跑多久」的口径完全一致（后端有自己的
 * now_ms），自算会在两次刷新之间漂移，用户会看到时长跳变。
 *
 * `null` / 非有限 / 负数一律退化为 `0s`——账本数据来自 IPC，宁可显示 0 也不
 * 让 `NaN` 之类的脏串漏到界面上。
 */
export function formatElapsed(ms: number | null | undefined): string {
  if (typeof ms !== 'number' || !Number.isFinite(ms) || ms < 0) return '0s'
  const totalSeconds = Math.floor(ms / 1000)
  const seconds = totalSeconds % 60
  const minutes = Math.floor(totalSeconds / 60) % 60
  const hours = Math.floor(totalSeconds / 3600)
  if (hours > 0) {
    return `${hours}h${String(minutes).padStart(2, '0')}m`
  }
  if (minutes > 0) {
    return `${minutes}m${String(seconds).padStart(2, '0')}s`
  }
  return `${seconds}s`
}

// ── 「账本可能变了」信号总线 ─────────────────────────────────────────

type Listener = () => void

const listeners = new Set<Listener>()

/**
 * 通知已挂载的面板：后台任务账本可能变了，去重新拉一次。
 *
 * 触发点刻意只有「真的会变」的时机（强制终止后、面板打开、刚结束一个任务），
 * **不做无条件轮询**——面板关闭时不该有 invoke 在飞。
 */
export function notifyBackgroundTasksChanged(): void {
  for (const listener of [...listeners]) listener()
}

/** 订阅账本变化通知；返回退订函数（useEffect 的清理位用） */
export function subscribeBackgroundTasksChanged(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
