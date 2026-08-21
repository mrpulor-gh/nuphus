import { useEffect, useRef } from 'react'

/**
 * 屏幕常亮（Screen Wake Lock）：任务执行中防止手机屏幕自动熄灭。
 *
 * 约束：
 * - 仅安全上下文（HTTPS / localhost）可用——局域网 HTTP 直连下 navigator.wakeLock
 *   为 undefined，此时静默降级（屏幕仍按系统设置自动熄灭）。
 * - 切后台 / 手动锁屏 / 低电量时浏览器会强制释放，无法阻止；回前台且仍执行中时
 *   本 hook 自动重新请求。
 */
export function useWakeLock(running: boolean): void {
  const lockRef = useRef<WakeLockSentinel | null>(null)

  useEffect(() => {
    // 特性检测：非安全上下文（HTTP 局域网直连）下 wakeLock 不可用，静默跳过
    if (typeof navigator === 'undefined' || !('wakeLock' in navigator)) return
    const wakeLock = navigator.wakeLock

    let disposed = false

    const acquire = async () => {
      try {
        const lock = await wakeLock.request('screen')
        if (disposed) {
          void lock.release()
          return
        }
        lockRef.current = lock
        lock.addEventListener('release', () => {
          if (lockRef.current === lock) lockRef.current = null
        })
      } catch {
        // 请求失败（权限拒绝 / 页面不可见 / 系统拒绝）静默降级
      }
    }

    const release = () => {
      const lock = lockRef.current
      lockRef.current = null
      if (lock) void lock.release()
    }

    if (running && document.visibilityState === 'visible') {
      void acquire()
    } else {
      release()
    }

    // 回前台且仍执行中 → 重新请求（切后台会被浏览器强制释放）
    const onVisibility = () => {
      if (document.visibilityState === 'visible' && running) void acquire()
    }
    document.addEventListener('visibilitychange', onVisibility)

    return () => {
      disposed = true
      release()
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [running])
}
