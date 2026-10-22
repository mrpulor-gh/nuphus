import { useCallback, useEffect, useRef, useState } from 'react'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { wfGateStatus } from '../lib/api'

/** 等主窗口就绪后再检查，避免与启动初始化争用网络；每次启动只自动检查一次。 */
const STARTUP_DELAY_MS = 5_000
/** 每个端点的超时会触发 updater 的下一镜像降级，不能依赖其默认无限等待。 */
const CHECK_TIMEOUT_MS = 10_000
/** 下载允许慢速链路，但整体等待有上限；连续十秒无进度时显示停滞提示。 */
const DOWNLOAD_TIMEOUT_MS = 600_000
const STALL_MS = 10_000
const DISMISSED_VERSION_KEY = 'nuphus:update-dismissed-version'

type Status = 'idle' | 'checking' | 'available' | 'latest' | 'downloading' | 'error'
interface UpdateState {
  status: Status
  update: Update | null
  progress: number
  stalled: boolean
  error: unknown
  downloaded: boolean
  installGuard: 'busy' | 'unavailable' | null
}

/** 仅记住用户延后的版本；存储不可用时仍允许本次窗口内收起提示。 */
function readDismissedVersion(): string | null {
  try {
    return localStorage.getItem(DISMISSED_VERSION_KEY)
  } catch {
    return null
  }
}

/** 释放 Rust 侧的更新资源；窗口退出时 IPC 可能已经不可用。 */
function closeUpdate(update: Update | null) {
  if (update) void update.close().catch(() => {})
}

/**
 * 主窗口持有更新状态，启动检查、提示条和手动更新页共用同一份结果。
 * 关闭更新弹窗不会丢失下载进度，也不会允许第二次下载并发启动。
 * 自动检查只读取版本信息；只有 install 的用户点击入口可以下载和安装。
 */
export function useAppUpdate(ready: boolean, executionBusy = false) {
  const [state, setState] = useState<UpdateState>({
    status: 'idle',
    update: null,
    progress: 0,
    stalled: false,
    error: null,
    downloaded: false,
    installGuard: null,
  })
  const [dismissedVersion, setDismissedVersion] = useState(readDismissedVersion)
  const alive = useRef(false)
  const busy = useRef(false)
  const checked = useRef(false)
  const current = useRef<Update | null>(null)
  const downloaded = useRef(false)
  const taskBusy = useRef(executionBusy)
  taskBusy.current = executionBusy
  const stallTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const publish = useCallback((patch: Partial<UpdateState>) => {
    if (alive.current) setState(previous => ({ ...previous, ...patch }))
  }, [])

  const clearStallTimer = useCallback(() => {
    if (stallTimer.current !== null) clearTimeout(stallTimer.current)
    stallTimer.current = null
  }, [])

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      clearStallTimer()
      /** 在途请求结束后自行释放，避免下载仍在使用资源时提前 close。 */
      if (!busy.current) {
        closeUpdate(current.current)
        current.current = null
      }
    }
  }, [clearStallTimer])

  const checkVersion = useCallback(async () => {
    if (busy.current || !alive.current) return
    checked.current = true
    busy.current = true
    publish({ status: 'checking', error: null })
    try {
      const found = await check({ timeout: CHECK_TIMEOUT_MS })
      if (!alive.current) {
        closeUpdate(found)
        return
      }
      closeUpdate(current.current)
      current.current = found
      downloaded.current = false
      publish({
        update: found,
        status: found ? 'available' : 'latest',
        downloaded: false,
        installGuard: null,
      })
    } catch (error) {
      /** 后台失败不会弹窗；用户打开更新页时仍能查看原因并手动重试。 */
      publish({ status: 'error', error })
    } finally {
      busy.current = false
      if (!alive.current) {
        closeUpdate(current.current)
        current.current = null
      }
    }
  }, [publish])

  useEffect(() => {
    if (!ready || !('__TAURI_INTERNALS__' in window) || checked.current) return
    const timer = setTimeout(() => {
      if (!checked.current) void checkVersion()
    }, STARTUP_DELAY_MS)
    return () => clearTimeout(timer)
  }, [ready, checkVersion])

  /** 后端全局闸门同时覆盖 Agent 执行/收尾和独立工作流；下载前后各复查一次。 */
  const ensureIdle = useCallback(async () => {
    if (!alive.current) return false
    if (taskBusy.current) {
      publish({ status: 'available', installGuard: 'busy' })
      return false
    }
    try {
      const execution = await wfGateStatus()
      if (!alive.current) return false
      if (
        !execution ||
        typeof execution.locked !== 'boolean' ||
        !['idle', 'agent', 'workflow'].includes(execution.reason)
      ) {
        publish({ status: 'available', installGuard: 'unavailable' })
        return false
      }
      if (execution.locked || execution.reason !== 'idle' || taskBusy.current) {
        publish({ status: 'available', installGuard: 'busy' })
        return false
      }
      return true
    } catch {
      /** 无法确认空闲时不执行安装；用户可稍后再次点击，不自动重启。 */
      publish({ status: 'available', installGuard: 'unavailable' })
      return false
    }
  }, [publish])

  const install = useCallback(async () => {
    const update = current.current
    if (!update || busy.current || !alive.current || taskBusy.current) return
    busy.current = true
    const bump = () => {
      clearStallTimer()
      if (!alive.current) return
      publish({ stalled: false })
      stallTimer.current = setTimeout(() => publish({ stalled: true }), STALL_MS)
    }
    try {
      publish({ installGuard: null })
      if (!(await ensureIdle())) return
      publish({
        status: 'downloading',
        progress: downloaded.current ? 100 : 0,
        stalled: false,
        error: null,
      })
      if (!downloaded.current) {
        bump()
        let contentLength: number | undefined
        let downloadedBytes = 0
        await update.download(
          event => {
            bump()
            if (event.event === 'Started') contentLength = event.data.contentLength
            if (event.event === 'Progress') {
              downloadedBytes += event.data.chunkLength
              publish({
                progress: contentLength
                  ? Math.min(100, (downloadedBytes / contentLength) * 100)
                  : 0,
              })
            }
            if (event.event === 'Finished') publish({ progress: 100 })
          },
          { timeout: DOWNLOAD_TIMEOUT_MS },
        )
        downloaded.current = true
        publish({ downloaded: true, progress: 100 })
      }
      clearStallTimer()
      publish({ stalled: false })
      if (!(await ensureIdle())) return
      await update.install({ restartAfterInstall: true })
      await relaunch()
    } catch (error) {
      publish({ status: 'error', error })
    } finally {
      clearStallTimer()
      publish({ stalled: false })
      busy.current = false
      if (!alive.current) {
        closeUpdate(current.current)
        current.current = null
      }
    }
  }, [publish, clearStallTimer, ensureIdle])

  const dismissNotice = () => {
    const version = state.update?.version
    if (!version) return
    setDismissedVersion(version)
    try {
      localStorage.setItem(DISMISSED_VERSION_KEY, version)
    } catch {
      /** 持久化失败不影响更新与本次窗口内的延后选择。 */
    }
  }

  return {
    ...state,
    installBlocked: executionBusy,
    installGuard: executionBusy ? ('busy' as const) : state.installGuard,
    noticeDismissed: state.update?.version === dismissedVersion,
    dismissNotice,
    checkVersion,
    install,
  }
}

export type AppUpdate = ReturnType<typeof useAppUpdate>
