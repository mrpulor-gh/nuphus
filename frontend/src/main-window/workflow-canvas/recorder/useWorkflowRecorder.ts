/**
 * useWorkflowRecorder.ts — 录制会话状态机（CanvasInner 内嵌编排）
 *
 * 状态机：off →(begin, 自动恢复 pending 进度)→ idle →(pick)→ capturing →(捕获成功/超时/取消)→ idle / pending
 *        pending →(confirmPending 真实入画布+draft+1+canvas_step_id 绑定)→ idle；(discardPending)→ idle
 *        idle →(abortSession)→ off；(completeSession 成功)→ off（终稿落盘 + pending 自动清理）
 *        idle →(saveProgress)→ off（drafts 落盘 pending，下次 begin 恢复继续）
 *        idle →(草稿面板 updateDraft/deleteDraft/clearDrafts)→ idle（编辑仅改 draft；删除/清空双向联动
 *        画布节点；画布手动删节点也会按 canvas_step_id 反向删对应草稿）
 *
 * 边界纪律：
 * - capturing 期间浮层收起/不可点（RecorderBar 捕获态只留「取消」）
 * - hook 捕获返回若 window_title 属于 Nuphus 自身 → 判误捕获，提示重试
 * - 关闭画布/卸载自动 rec_cancel + rec_abort，防 hook 滞留
 * - pending 恢复：无文件/损坏视为空会话，不打断录制入口；恢复的 canvas_step_id 若节点
 *   不在画布（未 Ctrl+S 保存过 workflow），删除草稿时 remove_step 失败 → 仅删 draft + 提示
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type { WorkflowStep } from '../../../core/types'
import * as api from './recorderApi'
import { buildSteps, isSelfCapture, hasNavigateDraft, NAVIGATE_PREREQ_MESSAGE } from './recorderMap'
import type { IntentPayload, RecAction, RecDraft, RecPending, RecStatus } from './recorderTypes'

/** browser_click CDP 捕获轮询间隔（与前端 hook 超时语义对齐：~500ms） */
const BROWSER_CLICK_POLL_INTERVAL_MS = 500
/** browser_click 捕获总等待上限（对齐录制捕获 hook 默认 60s） */
const BROWSER_CLICK_TIMEOUT_MS = 60_000

/** C3：网页点击/内容获取动作必须已录「打开网址」步骤（CDP 浏览器非原生浏览器）。
 *  browser_navigate 自身不需要前置。 */
const NAV_PREREQ_ACTIONS: RecAction[] = ['browser_click', 'browser_extract']

interface UseWorkflowRecorderOptions {
  workflowId: string
  readOnly: boolean
  notify: (msg: string) => void
  /** 入画布（applyEdit add_step 管线）：成功返回新节点 id；被拒/只读返回 null */
  insertStep: (req: InsertedRecordedStep) => Promise<string | null>
  /** 可选：删除画布节点（applyEdit remove_step 管线）。返回是否成功；节点不存在/只读 → false */
  removeStep?: (stepId: string) => Promise<boolean>
  /** 可选（C3）：画布（非本录制会话）前面是否已有 browser_navigate 步骤。
   *   pick/confirmPending 的网页点击/内容获取前置检查 = 会话草稿 navigate || 画布 navigate。 */
  hasCanvasNavigateStep?: () => boolean
}

/** 录制生成节点入画布请求（CanvasPage.addRecordedStep 消费；name 由意图生成非空） */
export interface InsertedRecordedStep {
  kind: 'tool' | 'sleep' | 'chat'
  /** 画布节点名 = 用户意图（CanvasPage 侧截断） */
  name: string
  do: WorkflowStep['do']
  /** 节点 capture 变量名（find_image 步骤输出供后续点击步骤引用） */
  capture?: string
}

export interface RecDraftPatch {
  intent?: string
  variable?: boolean
  exceptionNote?: string
}

export interface UseWorkflowRecorderResult {
  status: RecStatus
  capturingAction: RecAction | null
  pending: RecPending | null
  drafts: RecDraft[]
  begin: () => Promise<void>
  pick: (action: RecAction) => Promise<void>
  cancelCapture: () => void
  /** browser_click 意图面板「重新捕获」：丢弃当前捕获，复用同一步序号重新走 CDP 捕获 */
  recaptureBrowserClick: () => Promise<void>
  /** browser_extract 意图面板「选择目标元素」：复用 rec_browser 捕获选元素，不生成点击（C2） */
  captureBrowserExtractElement: () => Promise<void>
  confirmPending: (payload: IntentPayload) => Promise<boolean>
  discardPending: () => void
  abortSession: () => Promise<void>
  completeSession: (notes?: string) => Promise<api.RecCompleteResult | null>
  /** 保存进度：drafts 落盘 pending + 会话回 off（下次 begin 自动恢复） */
  saveProgress: (notes?: string) => Promise<boolean>
  /** 草稿编辑：仅改 draft（intent/variable/exception_note），不动画布节点 */
  updateDraft: (index: number, patch: RecDraftPatch) => void
  /** 草稿删除：有 canvas_step_id 先联动删画布节点，失败/无绑定也删 draft */
  deleteDraft: (index: number) => Promise<void>
  /** 画布节点被删除后按 canvas_step_id 反查删除对应草稿（CanvasPage.deleteSelected 联动调用） */
  removeDraftByStepId: (stepId: string) => void
  /** 清空全部草稿 + 删除 pending 文件（画布录制节点先逐个联动删除） */
  clearDrafts: () => Promise<void>
}

export function useWorkflowRecorder({
  workflowId,
  readOnly,
  notify,
  insertStep,
  removeStep,
  hasCanvasNavigateStep,
}: UseWorkflowRecorderOptions): UseWorkflowRecorderResult {
  const [status, setStatus] = useState<RecStatus>('off')
  const [capturingAction, setCapturingAction] = useState<RecAction | null>(null)
  const [drafts, setDrafts] = useState<RecDraft[]>([])
  const [pending, setPending] = useState<RecPending | null>(null)

  const statusRef = useRef<RecStatus>(status)
  statusRef.current = status
  const draftsRef = useRef<RecDraft[]>(drafts)
  draftsRef.current = drafts
  const pendingRef = useRef<RecPending | null>(null)
  pendingRef.current = pending
  const cancelArmedRef = useRef(false)
  const busyRef = useRef(false)
  /** removeStep 回调经 ref 转发（避免 CanvasPage 每次 selectedId 变化导致 deleteDraft 重建） */
  const removeStepRef = useRef<UseWorkflowRecorderOptions['removeStep']>(removeStep)
  removeStepRef.current = removeStep
  /** C3 画布 navigate 判定回调经 ref 转发（pick/confirmPending 只读最新画布态） */
  const hasCanvasNavigateRef =
    useRef<UseWorkflowRecorderOptions['hasCanvasNavigateStep']>(hasCanvasNavigateStep)
  hasCanvasNavigateRef.current = hasCanvasNavigateStep

  const resetSessionLocal = useCallback(() => {
    setCapturingAction(null)
    setPending(null)
  }, [])

  const backIdle = useCallback(
    (msg?: string) => {
      if (msg) notify(msg)
      setCapturingAction(null)
      setPending(null)
      setStatus('idle')
    },
    [notify],
  )

  /** C3 前置检查：网页点击/内容获取前必须已有「打开网址」步骤（本会话草稿 或 画布既有 navigate）。 */
  const browserNavigateReady = useCallback(() => {
    return hasNavigateDraft(draftsRef.current) || hasCanvasNavigateRef.current?.() === true
  }, [])

  /** 开始录制会话：rec_set_workflow → 自动检测 pending 恢复上次进度（无/损坏视为空会话） */
  const begin = useCallback(async () => {
    if (statusRef.current !== 'off' || busyRef.current) return
    busyRef.current = true
    try {
      await api.recSetWorkflow(workflowId)
      let restored = 0
      try {
        const pendingFile = await api.recLoadPending()
        if (
          pendingFile.exists &&
          Array.isArray(pendingFile.steps) &&
          pendingFile.steps.length > 0
        ) {
          // 恢复：canvas_step_id 原样保留（若对应节点已不在画布，删除草稿时降级为仅删 draft）
          setDrafts(pendingFile.steps)
          restored = pendingFile.steps.length
        } else {
          setDrafts([])
        }
      } catch {
        // pending 不存在 / 读取失败 / 文件损坏 → 正常开启空会话，不打断录制入口
        setDrafts([])
      }
      setStatus('idle')
      if (restored > 0) {
        notify(`已恢复上次录制进度 ${restored} 步（草稿列表可查看/删除）`)
      }
    } catch (e) {
      notify(`无法开始录制：${String(e)}`)
    } finally {
      busyRef.current = false
    }
  }, [workflowId, notify])

  /** capturing 中取消当前捕获（会话仍 active，可继续录下一动作）。
   *  hook 类中断 rec_start；browser_click 类中断 poll 循环并清页面捕获结果（幂等）。 */
  const cancelCapture = useCallback(() => {
    cancelArmedRef.current = true
    void api.recCancel().catch(() => {})
    void api.recBrowserCaptureClickCancel().catch(() => {})
  }, [])

  /** browser 元素 CDP 捕获：注入页面监听 → ~500ms 轮询（上限 60s）→ 捕获成功
   *  setPending 进入意图面板；取消（cancelArmedRef）/超时/错误 → backIdle 回 idle。
   *  need_reinject（整页导航注入丢失）→ 自动再 start（幂等重注入）继续等；
   *  error（disabled/不可定位）→ 提示后继续等下一次点击（capturing 态保留）。
   *  action 只允许 browser_click / browser_extract：
   *  - browser_click：用户点页面元素 → selector 生成 browser_click 步骤
   *  - browser_extract：用户点页面元素作「选择目标元素」→ selector 供 browser_exec h.extract 提取 */
  const runBrowserElementCapture = useCallback(
    async (action: 'browser_click' | 'browser_extract', stepNo: number) => {
      let captureUrl: string | undefined
      try {
        const started = await api.recBrowserCaptureClickStart()
        if (cancelArmedRef.current) {
          backIdle('已取消捕获')
          return
        }
        captureUrl = started.url
        const deadline = Date.now() + BROWSER_CLICK_TIMEOUT_MS
        while (!cancelArmedRef.current && Date.now() < deadline) {
          const r = await api.recBrowserCaptureClickPoll()
          if (cancelArmedRef.current) {
            backIdle('已取消捕获')
            return
          }
          if (r.captured && r.selector) {
            setPending({
              action,
              stepNo,
              browserCapture: {
                selector: r.selector,
                tag: r.tag ?? '',
                text: r.text ?? '',
                href: r.href ?? null,
                url: captureUrl,
              },
            })
            setStatus('pending')
            return
          }
          if (r.need_reinject) {
            // 页面刷新/整页导航 → 注入丢失，自动重注入后继续等待用户点击
            try {
              const again = await api.recBrowserCaptureClickStart()
              if (cancelArmedRef.current) {
                backIdle('已取消捕获')
                return
              }
              if (again.url) captureUrl = again.url
            } catch {
              // start 失败（页面正在关闭等）：交由下一轮 poll 报错退出
            }
            continue
          }
          if (r.error) {
            // disabled / 无法生成稳定 selector：提示后继续等（capturing 保留可取消/重试）
            notify(r.error)
            continue
          }
          // captured:false → 继续等
          await new Promise<void>(res => setTimeout(res, BROWSER_CLICK_POLL_INTERVAL_MS))
        }
        if (cancelArmedRef.current) {
          backIdle('已取消捕获')
        } else {
          backIdle('等待网页元素捕获超时（60s），本次未捕获。请重试')
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e)
        if (cancelArmedRef.current) {
          backIdle('已取消捕获')
        } else {
          // 后端已给中文指引（会话未初始化/浏览器未就绪/无页面/about:blank/不可注入页/连接异常）
          backIdle(`网页元素捕获失败：${msg}`)
        }
      }
    },
    [backIdle, notify],
  )

  /** browser_click 意图面板「重新捕获」：丢弃当前捕获回 capturing，复用同一 stepNo 重新走 CDP 捕获 */
  const recaptureBrowserClick = useCallback(async () => {
    const p = pendingRef.current
    if (!p || p.action !== 'browser_click' || busyRef.current) return
    busyRef.current = true
    cancelArmedRef.current = false
    setPending(null)
    setCapturingAction('browser_click')
    setStatus('capturing')
    try {
      await runBrowserElementCapture('browser_click', p.stepNo)
    } finally {
      busyRef.current = false
    }
  }, [runBrowserElementCapture])

  /** browser_extract「选择目标元素」：从整页提取意图面板进入元素捕获（C2）。
   *  复用 rec_browser_capture_click_*（点击元素 = 选定元素），不生成点击步骤。 */
  const captureBrowserExtractElement = useCallback(async () => {
    const p = pendingRef.current
    if (!p || p.action !== 'browser_extract' || busyRef.current) return
    busyRef.current = true
    cancelArmedRef.current = false
    setPending(null)
    setCapturingAction('browser_extract')
    setStatus('capturing')
    try {
      await runBrowserElementCapture('browser_extract', p.stepNo)
    } finally {
      busyRef.current = false
    }
  }, [runBrowserElementCapture])

  /** 选择录制动作：hook 捕获 / overlay 框选 / CDP 捕获 / 纯填写 */
  const pick = useCallback(
    async (action: RecAction) => {
      if (statusRef.current !== 'idle' || busyRef.current) return
      // C3 前置检查：网页点击/内容获取前必须先录「打开网址」步骤（CDP 浏览器非原生浏览器）。
      // navigate/sleep/hook/overlay 动作无需前置，直接继续。
      if (NAV_PREREQ_ACTIONS.includes(action) && !browserNavigateReady()) {
        notify(NAVIGATE_PREREQ_MESSAGE)
        return
      }
      busyRef.current = true
      cancelArmedRef.current = false
      setCapturingAction(action)
      setStatus('capturing')
      const stepNo = draftsRef.current.length + 1
      try {
        if (action === 'browser_navigate' || action === 'browser_extract' || action === 'sleep') {
          // 无需捕获：直接进意图面板（navigate 填 URL；extract 填获取说明/sleep 填秒数。
          // extract 面板可再点「选择目标元素」进入 C2 元素级提取）
          setPending({ action, stepNo })
          setStatus('pending')
          return
        }
        if (action === 'browser_click') {
          // CDP 注入捕获：自动注入 → 轮询真实点击 → selector 进意图面板
          await runBrowserElementCapture('browser_click', stepNo)
          return
        }
        if (action === 'region' || action === 'find_image') {
          const mode = action === 'region' ? 'rec_region' : 'rec_template'
          // 消费可能残留的旧 capture result（如上一轮截图未取走），避免误判为本次框选结果
          await api.takeCaptureResult().catch(() => {})
          await api.startOverlayMask(mode)
          const raw = await api.pollOverlayResult()
          if (cancelArmedRef.current) {
            backIdle('已取消框选')
            return
          }
          if (!raw || raw.cancelled) {
            backIdle(raw?.cancelled ? '已取消框选' : '框选超时或未完成')
            return
          }
          if (!raw.path || !raw.region) {
            backIdle('框选未返回有效结果')
            return
          }
          setPending({
            action,
            stepNo,
            overlay: { path: raw.path, rect: raw.region, base64: raw.base64 ?? null },
          })
          setStatus('pending')
          return
        }
        // hook 类：click/scroll/hotkey/text（text 先捕获「定位点击」）
        const hookKind = action === 'text' ? 'click' : action
        const ev = await api.recStart(hookKind)
        if (cancelArmedRef.current) {
          backIdle('已取消捕获')
          return
        }
        if (isSelfCapture(ev.window_title)) {
          // 先让 150ms 窗口给「取消」按钮的 DOM 事件落定，避免把主动取消误报成误捕获
          await new Promise<void>(r => setTimeout(r, 150))
          if (cancelArmedRef.current) {
            backIdle('已取消捕获')
            return
          }
          backIdle(
            '误捕获：操作落在 Nuphus 自身窗口，本次未记录。请先切换到目标应用窗口，再重新选择动作开始。',
          )
          return
        }
        setPending({ action, stepNo, event: ev })
        setStatus('pending')
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e)
        if (cancelArmedRef.current) {
          backIdle('已取消捕获')
        } else if (/timeout/i.test(msg)) {
          backIdle('等待超时（默认 60s），本次未捕获。请重试')
        } else if (/cancelled/i.test(msg)) {
          backIdle('已取消捕获')
        } else {
          backIdle(`捕获失败：${msg}`)
        }
      } finally {
        busyRef.current = false
      }
    },
    [backIdle, notify, browserNavigateReady, runBrowserElementCapture],
  )

  /** 意图面板确认：一个 pending 动作展开为 1+ 个真实 Action，逐个 applyEdit 入画布 + draft 绑定 */
  const confirmPending = useCallback(
    async (payload: IntentPayload) => {
      const p = pending
      if (!p) return false
      if (!payload.intent.trim()) {
        notify('请填写操作意图（这步在做什么），便于后续泛化')
        return false
      }
      // C3 兜底：pending 形成后若 navigate 步骤被删/无（草稿已清空）→ 阻止确认
      if (NAV_PREREQ_ACTIONS.includes(p.action) && !browserNavigateReady()) {
        notify(NAVIGATE_PREREQ_MESSAGE)
        return false
      }
      const builtAll = buildSteps(p, payload)
      if (builtAll.length === 0) {
        notify('无法构造真实执行步骤')
        return false
      }
      const bound: RecDraft[] = []
      for (const built of builtAll) {
        const stepId = await insertStep({
          kind: built.kind,
          name: built.name,
          do: built.do,
          capture: built.capture,
        })
        if (!stepId) {
          notify(
            bound.length === 0
              ? '步骤未加入画布（只读或插入被拒绝）'
              : `本步部分节点未加入画布（画布状态变化或只读）：已加入 ${bound.length} 个，请检查画布与草稿`,
          )
          break
        }
        // 画布节点绑定到 draft：删除草稿时联动 remove_step（恢复/编辑均保留该绑定）
        bound.push({ ...built.draft, canvas_step_id: stepId })
      }
      if (bound.length === 0) return false
      setDrafts(prev => [...prev, ...bound])
      setPending(null)
      setCapturingAction(null)
      setStatus('idle')
      return true
    },
    [pending, insertStep, notify, browserNavigateReady],
  )

  /** 意图面板取消本步（丢弃待确认捕获，回到待命） */
  const discardPending = useCallback(() => {
    setPending(null)
    setCapturingAction(null)
    setStatus('idle')
  }, [])

  /** 放弃整个录制会话（画布不可改/用户取消时由外层 askConfirm 后调用） */
  const abortSession = useCallback(async () => {
    cancelArmedRef.current = true
    try {
      await api.recCancel()
    } catch {}
    try {
      await api.recAbort()
    } catch {}
    try {
      await api.recBrowserCaptureClickCancel()
    } catch {}
    setDrafts([])
    setPending(null)
    setCapturingAction(null)
    setStatus('off')
  }, [])

  /** 完成：steps(step_draft 数组) + user_notes → rec_complete 生成 record-draft JSON */
  const completeSession = useCallback(
    async (notes?: string) => {
      const steps = draftsRef.current
      if (steps.length === 0) {
        notify('还没有录制任何步骤，无法完成。请至少录制一步后再点「完成」')
        return null
      }
      if (statusRef.current === 'off') return null
      try {
        const res = await api.recComplete(steps, notes?.trim() || null)
        notify(
          `录制完成：${res.path}（${res.step_count} 步）· 可交给 WorkflowAgent 阅读微调；画布改动为未保存编辑，请 Ctrl+S 保存`,
        )
        setDrafts([])
        setPending(null)
        setCapturingAction(null)
        setStatus('off')
        return res
      } catch (e) {
        notify(`完成失败：${String(e)}`)
        return null
      }
    },
    [notify],
  )

  /** 保存进度：drafts 落盘 pending + 会话回 off（后端 rec_save_pending 写盘后也回 idle） */
  const saveProgress = useCallback(
    async (notes?: string): Promise<boolean> => {
      const steps = draftsRef.current
      if (steps.length === 0) {
        notify('还没有录制任何步骤，无法保存进度。请先录制至少一步')
        return false
      }
      if (statusRef.current === 'off') return false
      try {
        const res = await api.recSavePending(steps, notes?.trim() || null)
        setDrafts([])
        setPending(null)
        setCapturingAction(null)
        setStatus('off')
        notify(`录制进度已保存：${res.path}，下次打开此工作流点录制可继续`)
        return true
      } catch (e) {
        notify(`保存进度失败：${String(e)}`)
        return false
      }
    },
    [notify],
  )

  /** 草稿编辑：仅改 draft 的意图语义字段（intent/variable/exception_note），不动画布节点 */
  const updateDraft = useCallback((index: number, patch: RecDraftPatch) => {
    setDrafts(prev =>
      prev.map((d, i) => {
        if (i !== index) return d
        const next: RecDraft = { ...d }
        if (typeof patch.intent === 'string') next.intent = patch.intent.trim()
        if (typeof patch.exceptionNote === 'string') {
          const t = patch.exceptionNote.trim()
          next.exception_note = t ? t : null
        }
        if (typeof patch.variable === 'boolean') {
          next.params = { ...(next.params ?? {}), variable: patch.variable }
        }
        return next
      }),
    )
  }, [])

  /** 草稿删除：有 canvas_step_id 先 await 画布 remove_step；成功/节点不存在/无绑定都删 draft */
  const deleteDraft = useCallback(
    async (index: number): Promise<void> => {
      const target = draftsRef.current[index]
      if (!target) return
      const stepNo = index + 1
      if (target.canvas_step_id) {
        let removed = false
        try {
          const fn = removeStepRef.current
          removed = fn ? await fn(target.canvas_step_id) : false
        } catch {
          removed = false
        }
        setDrafts(prev => prev.filter((_, i) => i !== index))
        notify(
          removed
            ? `已删除第 ${stepNo} 步草稿（画布节点同步删除）`
            : `第 ${stepNo} 步草稿已删除；画布节点未能同步删除（可能只读或节点不在画布），可手动检查`,
        )
        return
      }
      setDrafts(prev => prev.filter((_, i) => i !== index))
      notify(`第 ${stepNo} 步草稿已删除；该步画布节点请手动删除（草稿未记录节点绑定）`)
    },
    [notify],
  )

  /** 画布节点被删除 → 反向联动删对应草稿（canvas_step_id 匹配即移除，无则空操作） */
  const removeDraftByStepId = useCallback((stepId: string) => {
    setDrafts(prev => {
      if (!prev.some(d => d.canvas_step_id === stepId)) return prev
      return prev.filter(d => d.canvas_step_id !== stepId)
    })
  }, [])

  /** 清空全部草稿：先逐个联动删画布录制节点（成功/节点不存在都算通过；失败降级仅删 draft），
   *  再删 pending 文件 + 清空内存（防删文件失败留下旧进度下次误恢复）。画布删除可 Ctrl+Z 逐条撤销。 */
  const clearDrafts = useCallback(async (): Promise<void> => {
    const targets = draftsRef.current
    const fn = removeStepRef.current
    let removed = 0
    let skipped = 0 // 无绑定 / 节点不在画布（恢复场景正常降级，不算失败）
    let failed = 0
    if (fn && targets.length > 0) {
      for (const d of targets) {
        if (!d.canvas_step_id) {
          skipped += 1
          continue
        }
        try {
          const ok = await fn(d.canvas_step_id)
          if (ok) removed += 1
          else skipped += 1 // removeStep false = 节点不存在（或只读会话已被兜底终止）
        } catch {
          failed += 1
        }
      }
    }
    try {
      await api.recDiscardPending()
    } catch (e) {
      notify(`清空草稿失败：${String(e)}`)
      return
    }
    setDrafts([])
    setPending(null)
    setCapturingAction(null)
    const removedText =
      targets.length === 0
        ? '草稿已清空'
        : `草稿已清空：画布同步删除 ${removed} 个录制节点${skipped > 0 ? `（${skipped} 个无需/未能定位画布节点，仅删草稿）` : ''}${failed > 0 ? `；${failed} 个画布节点删除失败，请手动检查` : ''}`
    notify(`${removedText}（画布删除可 Ctrl+Z 撤销）`)
  }, [notify])

  // ── readOnly 兜底：运行中/旧格式只读 → 立即结束会话（防编辑-执行竞态）──
  useEffect(() => {
    if (readOnly && statusRef.current !== 'off') {
      cancelArmedRef.current = true
      void api.recCancel().catch(() => {})
      void api.recAbort().catch(() => {})
      void api.recBrowserCaptureClickCancel().catch(() => {})
      setDrafts([])
      setPending(null)
      setCapturingAction(null)
      setStatus('off')
      notify('画布已进入只读（运行中/格式不兼容），录制会话已结束')
    }
  }, [readOnly, notify])

  // ── capturing 中 Esc = 取消捕获（rec_cancel 幂等）；按键录制由 hook 自行收口 ──
  useEffect(() => {
    if (status !== 'capturing') return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        cancelCapture()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [status, cancelCapture])

  // ── 卸载兜底：关闭画布时若 capturing/会话 active → rec_cancel + rec_abort 防 hook 滞留 ──
  useEffect(() => {
    return () => {
      cancelArmedRef.current = true
      void api.recCancel().catch(() => {})
      void api.recAbort().catch(() => {})
      void api.recBrowserCaptureClickCancel().catch(() => {})
    }
  }, [])

  return {
    status,
    capturingAction,
    pending,
    drafts,
    begin,
    pick,
    cancelCapture,
    recaptureBrowserClick,
    captureBrowserExtractElement,
    confirmPending,
    discardPending,
    abortSession,
    completeSession,
    saveProgress,
    updateDraft,
    deleteDraft,
    removeDraftByStepId,
    clearDrafts,
  }
}
