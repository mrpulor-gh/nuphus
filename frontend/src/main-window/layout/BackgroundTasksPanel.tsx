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
import { listen } from '../../core/bridge'
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
  /** 列表拉取时刻（本地插值基准）：后端 `elapsed_ms` 仍是权威值，本地只在其上
   *  叠加「自上次拉取以来的增量」，用于面板打开时的秒级平滑走动，不另算起点。 */
  const listFetchedAtRef = useRef(Date.now())
  /** 面板打开时每秒重渲染一次：让「已运行 / 剩余」平滑推进（值仍来自后端+增量） */
  const [, forceTick] = useState(0)
  useEffect(() => {
    if (!open) return
    const id = window.setInterval(() => forceTick(n => n + 1), 1000)
    return () => window.clearInterval(id)
  }, [open])

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

  /** 拉列表。`silent` 时（轮询）不置 loading，避免每 2s 闪一次骨架。
   *
   *  轮询顺带把**保留项数字对齐到列表**（2026-10-11 修复）：数字与列表此前是
   *  两路独立真相——`count_retained_background_tasks` 只在事件驱动点刷新，而列表
   *  每 2s 刷新一次。结果：任务自然结束 / 被看门狗强杀时列表条目消失了，胶囊数字
   *  却停在旧值，面板也不会自动关（自动关判的是 `retainedCount`）。这里用刚拉回来
   *  的列表直接派生数字：与列表同源，不新增命令或事件通道，也不会有第二处口径。 */
  const refreshList = useCallback(async (silent = false) => {
    if (!silent) setLoading(true)
    try {
      const rows = await listBackgroundTasks()
      const list = Array.isArray(rows) ? rows : []
      setTasks(list)
      setListError(null)
      // 轮询顺带**把保留项数字对齐到列表**（2026-10-11 修复）：数字与列表此前是
      // 两路独立真相——`count_retained_background_tasks` 只在事件驱动点刷新，而
      // 列表每 2s 刷新一次。结果：任务自然结束 / 被看门狗强杀时列表条目消失了，
      // 胶囊数字却停在旧值，面板也不会自动关（自动关判的是 `retainedCount`）。
      // 这里用刚拉回来的列表直接派生数字：与列表同源，不新增命令或事件通道。
      setRetainedCount(list.filter(t => t.retain).length)
      listFetchedAtRef.current = Date.now()
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

  // ── 后端账本变更事件（2026-10-11，事件驱动的显示链路）──
  // 根因：retain 翻转（超时/取消转后台）发生在**工具执行中**，而「执行转空闲」
  // 信号往往在其后才到；两者一旦错过或颠倒，保留项数字就永远停在 0，直到用户
  // 刷新页面——那正是「发起后台任务后看不见胶囊，刷新才出现」的症状。
  // 后端在 register / set_retain 翻转 / unregister（含看门狗强杀、退出清算、
  // 面板手动结束）时广播本事件；载荷为空，收到后走既有 IPC 重拉，不冗余快照。
  //
  // 只拉数字不拉列表：面板关着时无需列表（列表轮询本就只在打开时跑）；
  // 面板开着时 refreshList 的 2s 轮询会把数字与列表重新对齐。
  useEffect(() => {
    let unlisten: (() => void) | null = null
    let cancelled = false
    void listen<{ seq: number; event: { type: string } }>('nuphus-event', ({ event }) => {
      if (event?.type === 'background_tasks_changed') void refreshCount()
    }).then(fn => {
      // 卸载竞态：listen 是异步的，组件可能已卸载才拿到取消函数
      if (cancelled) fn()
      else unlisten = fn
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [refreshCount])

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
      {/* 锚点：唯一 fixed 元素，接管胶囊的定位与堆叠职责（见 background-tasks.css）。
          面板在锚点内向左展开，右缘对齐胶囊左缘——避开桌面工具条的命中区。 */}
      <div className="bgt-anchor">
        <button
          className="bgt-entry"
          onClick={handleToggle}
          aria-expanded={open}
          // 无障碍名不能只靠可见文本——按钮里只有图标 + 数字，读屏会把它念成「2」。
          // 显式给 label：控件名 + 数量都在，语义与视觉一致。
          // 视觉 tips 走自定义 .bgt-tip（hover/focus 呼出）；原生 title 会与它
          // 双重呼出，故不再设置。
          aria-label={t('backgroundTasks.entryLabel', String(retainedCount))}
        >
          <span className="task-track-shimmer" aria-hidden="true" />
          <IconTerminal size={13} />
          <span className="bgt-entry-count">{retainedCount}</span>
          <span className="bgt-tip" role="tooltip">
            {t('backgroundTasks.entryTip')}
          </span>
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
                {/* 小字说明：后台任务多为超时转入的验证/构建/测试长命令（大王钦定文案） */}
                <span className="bgt-hint">{t('backgroundTasks.hint')}</span>
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
                tasks.map(task => {
                  // 本地插值：后端 elapsed_ms + 自上次拉取以来的增量（秒级平滑走动）
                  const elapsedMs = task.elapsed_ms + (Date.now() - listFetchedAtRef.current)
                  // 上限：后端未给字段（旧版本）→ 0 = 本行不渲染，不用 NaN 编造状态
                  const lifetimeMs = Number.isFinite(task.max_lifetime_secs)
                    ? task.max_lifetime_secs * 1000
                    : 0
                  const remainingMs = Math.max(0, lifetimeMs - elapsedMs)
                  // 主行：调用方填的标题优先；没填才回落命令（普通用户看不懂裸命令）
                  const headline = task.title?.trim() ? task.title : task.command
                  return (
                    <div className="bgt-row" key={task.id}>
                      <div className="bgt-row-main">
                        {/* 1｜在干什么（唯一的大字行） */}
                        <div className="bgt-headline" title={task.title ?? task.command}>
                          {headline}
                        </div>
                        {/* 2｜动态：状态 · 已运行 · 剩余上限（上限缺失则整段省略） */}
                        <div className="bgt-meta">
                          <span
                            className={`bgt-state bgt-state--${task.retain ? 'retained' : 'foreground'}`}
                          >
                            {task.retain
                              ? t('backgroundTasks.stateRetained')
                              : t('backgroundTasks.stateRunning')}
                          </span>
                          <span className="bgt-dot">·</span>
                          <span className="bgt-elapsed">
                            <IconClock3 size={10} />{' '}
                            {t('backgroundTasks.elapsed', formatElapsed(elapsedMs))}
                          </span>
                          {lifetimeMs > 0 && (
                            <>
                              <span className="bgt-dot">·</span>
                              <span className="bgt-remaining">
                                {remainingMs > 0
                                  ? t('backgroundTasks.remaining', formatElapsed(remainingMs))
                                  : t('backgroundTasks.expiring')}
                              </span>
                            </>
                          )}
                        </div>
                        {/* 3｜次要一行（小字）：命令原文（有标题时才有必要）/ 输出 / PID，
                             全量信息在 hover 里——普通用户扫前两行即可 */}
                        <div
                          className="bgt-sub"
                          title={[
                            task.command,
                            `PID ${task.pid}`,
                            t(
                              'backgroundTasks.startedAt',
                              new Date(task.started_at_ms).toLocaleString(),
                            ),
                            task.output_path ?? '',
                          ]
                            .filter(Boolean)
                            .join('\n')}
                        >
                          {task.title?.trim() ? (
                            <span className="bgt-sub-cmd">{task.command}</span>
                          ) : null}
                          {task.output_path ? (
                            <>
                              {task.title?.trim() && <span className="bgt-dot">·</span>}
                              <IconFile size={10} />
                              <span className="bgt-output-path">{task.output_path}</span>
                            </>
                          ) : null}
                          <span className="bgt-dot">·</span>
                          <span className="bgt-pid">PID {task.pid}</span>
                        </div>
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
                  )
                })
              )}
            </div>
          </div>
        )}
      </div>

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
