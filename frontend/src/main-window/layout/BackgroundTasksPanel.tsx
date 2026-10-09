// BackgroundTasksPanel — 后台任务账本的可见性与手动结束入口
//
// 产品原则：**不杀可以，但不能看不见**。「强制终止」只杀 retain=false 的前台
// 进程；被显式保留（background=true）或超时转后台的长任务会继续跑。若用户
// 无从看见它们，就会重跑同一个长命令 —— 那是后台任务机制最典型的反噬。
//
// 入口只在**确有保留任务**时出现（数量为 0 不占界面）。刷新策略：
//   · 列表：面板打开时轮询（LIST_POLL_MS），关闭即停止（无谓 invoke 一律不做）
//   · 数量：挂载、执行态转空闲、强制终止后、以及面板打开/结束任务后各查一次
//     （全部是事件驱动，没有常驻轮询）
//
// 面板与入口都复用既有的浮层视觉语言：底走 var(--panel-bg)（与
// .session-rail-drawer / .wfst-panel 同一按键）、border/radius/shadow 同族，
// 不引新颜色键、不写死 rgba（回归测试 frontend/src/styles/opacity-system.test.ts）。

import { useCallback, useEffect, useRef, useState } from 'react'
import type { BackgroundTaskView } from '../../core/types'
import { useLanguage } from '../../locales'
// 图标一律经 ui/Icons.tsx 出口（eslint no-restricted-imports：新代码零违规）
import { IconClock3, IconFile, IconSquare, IconTerminal, IconX } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { showAppFeedback } from '../../ui/islandChannel'
import { CompactModal } from './CompactModal'
import {
  backendErrorMessage,
  countRetainedBackgroundTasks,
  killBackgroundTask,
  listBackgroundTasks,
} from '../lib/api'
import {
  formatElapsed,
  notifyBackgroundTasksChanged,
  subscribeBackgroundTasksChanged,
} from '../lib/backgroundTasks'
import type { ExecutionStage } from '../../hooks/useExecutionState'
import '../../styles/background-tasks.css'

/** 面板打开时的列表轮询周期。取 2s：足够让「进程已自己退出」及时从清单消失，
 *  又不至于把 IPC 打成噪音（用户可见窗口内才在跑）。 */
const LIST_POLL_MS = 2000

interface BackgroundTasksPanelProps {
  /**
   * 执行阶段（唯一执行态，见 useExecutionState）。
   *
   * 用途：**执行转空闲**是「可能刚产生了一条保留后台任务」的唯一可靠信号
   * （长任务超时的那一刻并没有事件推给前端）。用它把数量刷新做成事件驱动，
   * 而不是常驻轮询。
   */
  executionStage: ExecutionStage
}

export function BackgroundTasksPanel({ executionStage }: BackgroundTasksPanelProps) {
  const { t } = useLanguage()

  const [retainedCount, setRetainedCount] = useState(0)
  const [open, setOpen] = useState(false)
  const [tasks, setTasks] = useState<BackgroundTaskView[]>([])
  const [listError, setListError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [confirmTask, setConfirmTask] = useState<BackgroundTaskView | null>(null)
  const [killingId, setKillingId] = useState<string | null>(null)

  /** 刷新保留项数量（轻量命令）。失败一律静默：入口的可见性不该把用户逼进错误态，
   *  真正要看见失败的时刻是用户主动点「结束」时（那里有独立的错误呈现）。 */
  const refreshCount = useCallback(async () => {
    try {
      const n = await countRetainedBackgroundTasks()
      setRetainedCount(typeof n === 'number' && n > 0 ? n : 0)
    } catch {
      /* 计数失败不弹窗：面板本体才是用户主动打开时的权威视图 */
    }
  }, [])

  /** 拉列表。`silent` 时（轮询）不置 loading，避免每 2s 闪一次骨架。 */
  const refreshList = useCallback(async (silent = false) => {
    if (!silent) setLoading(true)
    try {
      const rows = await listBackgroundTasks()
      setTasks(Array.isArray(rows) ? rows : [])
      setListError(null)
    } catch (e) {
      // 后端不可达就明说，不能静默成一个空列表（空列表会被读成「没有后台任务」）
      setListError(backendErrorMessage(e))
    } finally {
      if (!silent) setLoading(false)
    }
  }, [])

  // ── 数量刷新：挂载 / 执行转空闲 / 强制终止后 ──
  useEffect(() => {
    void refreshCount()
  }, [refreshCount])

  const wasBusyRef = useRef(executionStage !== 'idle')
  useEffect(() => {
    const busy = executionStage !== 'idle'
    // 转空闲那一刻 = 一轮执行刚结束，长任务超时形成的保留项此刻才可能出现
    if (wasBusyRef.current && !busy) {
      void refreshCount()
    }
    wasBusyRef.current = busy
  }, [executionStage, refreshCount])

  useEffect(() => subscribeBackgroundTasksChanged(() => void refreshCount()), [refreshCount])

  // ── 列表轮询：仅面板打开时 ──
  useEffect(() => {
    if (!open) return
    void refreshList()
    const timer = window.setInterval(() => void refreshList(true), LIST_POLL_MS)
    return () => window.clearInterval(timer)
  }, [open, refreshList])

  // 面板开着且最后一个保留项被外部（后端自行结束 / 其它入口）清掉时，
  // 让入口随之消失——否则会留下一个点进去空空如也的死入口。
  useEffect(() => {
    if (open && retainedCount === 0) {
      setOpen(false)
    }
  }, [open, retainedCount])

  const handleToggle = useCallback(() => {
    // 副作用放在 updater **外面**：updater 在 StrictMode 下会被调用两次，
    // 在里面发请求 = 一次点击打两次 IPC。
    const next = !open
    setOpen(next)
    if (next) void refreshCount()
  }, [open, refreshCount])

  const handleConfirmKill = useCallback(async () => {
    const target = confirmTask
    if (!target) return
    setKillingId(target.id)
    try {
      const result = await killBackgroundTask(target.id)
      setConfirmTask(null)
      // 后端返回的是事实句（已终止 / 已自行结束），原样透出——本地化文案
      // 说不清「到底杀没杀成功」，不该拿模板覆盖事实。
      // IPC 返回 null（无响应体）时**只说已发出请求**：不拿模板冒充事实句。
      showAppFeedback(result ?? t('backgroundTasks.killRequested'), 'success')
      notifyBackgroundTasksChanged()
      await Promise.all([refreshList(true), refreshCount()])
    } catch (e) {
      showAppFeedback(`${t('backgroundTasks.killFailed')}: ${backendErrorMessage(e)}`, 'error')
    } finally {
      setKillingId(null)
    }
  }, [confirmTask, refreshCount, refreshList, t])

  // 入口：数量为 0 时完全不渲染（不占界面、不进 tab 序）
  if (retainedCount === 0) return null

  return (
    <>
      <button
        className="bgt-entry"
        onClick={handleToggle}
        aria-expanded={open}
        // 无障碍名不能只靠可见文本——按钮里只有图标 + 数字，读屏会把它念成「2」。
        // 显式给 label：控件名 + 数量都在，语义与视觉一致。
        aria-label={t('backgroundTasks.entryLabel', String(retainedCount))}
        title={t('backgroundTasks.entryTitle')}
      >
        <IconTerminal size={13} />
        <span className="bgt-entry-count">{retainedCount}</span>
      </button>

      {open && (
        <div className="bgt-panel" role="dialog" aria-label={t('backgroundTasks.title')}>
          <div className="bgt-header">
            <div className="bgt-header-left">
              {/* IconTerminal 是手绘 SVG，只接 size；着色类挂外层 span */}
              <span className="bgt-header-icon">
                <IconTerminal size={14} />
              </span>
              <span className="bgt-title">{t('backgroundTasks.title')}</span>
            </div>
            <button
              className="bgt-close-btn"
              onClick={() => setOpen(false)}
              aria-label={t('backgroundTasks.close')}
              title={t('backgroundTasks.close')}
            >
              <IconX size={12} />
            </button>
          </div>

          <div className="bgt-list">
            {listError ? (
              <div className="bgt-error" role="alert">
                {listError}
              </div>
            ) : loading && tasks.length === 0 ? (
              <div className="bgt-empty">{t('backgroundTasks.loading')}</div>
            ) : tasks.length === 0 ? (
              <div className="bgt-empty">{t('backgroundTasks.empty')}</div>
            ) : (
              tasks.map(task => (
                <div className="bgt-row" key={task.id}>
                  <div className="bgt-row-main">
                    <div className="bgt-cmd" title={task.command}>
                      {task.command}
                    </div>
                    <div className="bgt-meta">
                      <span className="bgt-pid">PID {task.pid}</span>
                      <span className="bgt-dot">·</span>
                      <span className="bgt-elapsed">
                        <IconClock3 size={10} /> {formatElapsed(task.elapsed_ms)}
                      </span>
                      {task.retain && (
                        <span className="bgt-badge">{t('backgroundTasks.badgeRetained')}</span>
                      )}
                    </div>
                    {task.output_path && (
                      <div className="bgt-output" title={task.output_path}>
                        <IconFile size={10} />
                        <span className="bgt-output-path">{task.output_path}</span>
                      </div>
                    )}
                  </div>
                  <Button
                    variant="danger"
                    size="sm"
                    className="bgt-kill-btn"
                    loading={killingId === task.id}
                    onClick={() => setConfirmTask(task)}
                  >
                    <IconSquare size={11} />
                    {t('backgroundTasks.kill')}
                  </Button>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      <CompactModal
        open={confirmTask !== null}
        onClose={() => setConfirmTask(null)}
        title={t('backgroundTasks.confirmTitle')}
        size="sm"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setConfirmTask(null)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="danger"
              size="sm"
              loading={killingId !== null}
              onClick={() => void handleConfirmKill()}
            >
              {t('backgroundTasks.kill')}
            </Button>
          </>
        }
      >
        <p className="bgt-confirm-text">
          {t('backgroundTasks.confirmBody', confirmTask?.command ?? '')}
        </p>
      </CompactModal>
    </>
  )
}
