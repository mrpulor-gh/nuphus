/**
 * 空闲调度：把非关键路径任务（如大 chunk 预取）推迟到浏览器空闲时执行。
 *
 * 优先 requestIdleCallback，并用 timeout 兜底，保证浏览器长期繁忙时任务仍会执行；
 * 内核不支持时退化为 setTimeout，延迟不低于 2s。
 * 两种路径都不会同步执行 task——预取绝不落在应用启动的关键路径上。
 */
export function scheduleIdle(task: () => void, fallbackDelayMs = 2000): void {
  if (typeof window === 'undefined') return
  if (typeof window.requestIdleCallback === 'function') {
    window.requestIdleCallback(() => task(), { timeout: fallbackDelayMs })
    return
  }
  window.setTimeout(task, fallbackDelayMs)
}
