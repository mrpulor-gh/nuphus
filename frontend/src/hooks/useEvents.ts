// useEvents.ts — Event listener (nuphus-event + toolbar:action)
import { useCallback, useEffect, useRef } from 'react'
import { invoke, listen } from '../core/bridge'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type {
  ChatMessage,
  NuphusEvent,
  SecurityCheck,
  UserInputRequest,
  TimelineEntry,
  PlanData,
  PlanTask,
  TaskStatus,
} from '../core/types'
import type { MutableRefObject } from 'react'
import type { MoodState } from '../ui/MoodFace'
import { playUiSound } from '../ui/sound'

type RegionPickerMode = 'picker' | 'capture' | 'ocr' | null
type TokenUsageState = { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null

// ════════════════════════════════════════════════════════════
// toolToMood mapping table
// ════════════════════════════════════════════════════════════

const toolToMood: Record<string, MoodState> = {
  web_search: 'searching',
  web_extract: 'searching',
  Read: 'reading',
  Write: 'writing',
  Edit: 'writing',
  system_shell: 'coding',
  desktop_screenshot: 'working',
  desktop_mouse_click: 'working',
  desktop_input: 'working',
  classify_intent: 'analyzing',
  task_dispatch: 'analyzing',
}

// ════════════════════════════════════════════════════════════
// Interface — Receive useSession callbacks/refs
// ════════════════════════════════════════════════════════════

export interface EventHandlers {
  // Refs
  refs: {
    streamingMsgId: MutableRefObject<string | null>
    lastStreamingMsgId: MutableRefObject<string | null>
    executionActiveRef: MutableRefObject<boolean>
    processingRef: MutableRefObject<boolean>
    toolCallCountRef: MutableRefObject<number>
  }

  // State setters
  messages: ChatMessage[]
  setMessages: (v: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => void
  setIsProcessing: (v: boolean) => void
  setCompleted: (v: boolean) => void
  setMood: (v: MoodState) => void
  setTimeline: React.Dispatch<React.SetStateAction<TimelineEntry[]>>
  setGoalType: React.Dispatch<
    React.SetStateAction<{ type: string; label: string; confidence: number } | null>
  >
  setGoal: (v: string) => void
  setProgress: React.Dispatch<
    React.SetStateAction<{ iteration: number; max: number; calls: number }>
  >
  setExecPhase: React.Dispatch<
    React.SetStateAction<'understanding' | 'executing' | 'recording' | 'workflow' | 'retrying' | ''>
  >
  setPauseState: React.Dispatch<React.SetStateAction<{ actionId: string } | null>>
  setMainTokenUsage: React.Dispatch<
    React.SetStateAction<{
      inputTokens: number
      outputTokens: number
      cacheHitTokens: number
    } | null>
  >
  setExecTokenUsage: React.Dispatch<
    React.SetStateAction<{
      inputTokens: number
      outputTokens: number
      cacheHitTokens: number
    } | null>
  >
  setTotalDurationMs: (v: number) => void
  setTotalCalls: (v: number) => void
  setCurrentTaskDesc: (v: string) => void
  setPlanData: React.Dispatch<React.SetStateAction<PlanData | null>>
  setShowReview: (v: boolean) => void
  setShowPlannerModal: (v: boolean) => void
  setApprovalState: React.Dispatch<
    React.SetStateAction<{
      open: boolean
      kind: string
      title: string
      content: string
      actionId: string
      tenetCount: number
    }>
  >
  setTaskBubbleVisible: (v: boolean) => void
  setRefineState: React.Dispatch<
    React.SetStateAction<{ usagePercent: number; totalLimit: number } | null>
  >
  refineState: { usagePercent: number; totalLimit: number } | null
  setRefining: (v: boolean) => void
  pendingRefine: { usagePercent: number; totalLimit: number; skippedTurns: number } | null
  setPendingRefine: React.Dispatch<
    React.SetStateAction<{ usagePercent: number; totalLimit: number; skippedTurns: number } | null>
  >
  setDismissThinking: (v: boolean) => void
  setExecutionCounter: React.Dispatch<React.SetStateAction<number>>
  setModelName: (v: string) => void
  setSessionId: (v: string) => void
  setStepIndex: (v: number | ((prev: number) => number)) => void
  setSecurity: React.Dispatch<React.SetStateAction<SecurityCheck | null>>
  userInputRequest: UserInputRequest | null
  setUserInputRequest: (v: UserInputRequest | null) => void
  setRegionPickerMode: (v: RegionPickerMode) => void
  setMode?: (v: string) => void
  /** 切换 mode 后重载聊天历史（mode 联动会话视图：目标 mode 有历史则显示继续，无则空白新对话） */
  reloadChatFromBackend?: () => Promise<void>

  // Callbacks
  addMessage: (msg: ChatMessage) => void
  transitionTask?: (taskId: number, status: TaskStatus) => void
}

// ════════════════════════════════════════════════════════════
// Hook
// ════════════════════════════════════════════════════════════

export function useEvents(h: EventHandlers) {
  const unlistenRef = useRef<(() => void) | undefined>(undefined)
  const lastEventSeq = useRef(0)
  const lastEventTime = useRef(Date.now())
  const eventCountRef = useRef(0)
  const errorTimeoutsRef = useRef<number[]>([])
  // Sync refineState to a ref so event handlers always read the current value
  // (fixes stale-closure bug in refine_executing that caused infinite auto-refine loop)
  const refineStateRef = useRef(h.refineState)
  refineStateRef.current = h.refineState
  // Same stale-closure mirror for state read inside the once-registered nuphus-event /
  // mobile-user-input-resolved listeners: prompt_timeout 关闭输入弹窗（读 userInputRequest）、
  // refine_prompt 跳过累计（读 pendingRefine）、手机端回执同步关弹窗（读 userInputRequest）
  const userInputRequestRef = useRef(h.userInputRequest)
  userInputRequestRef.current = h.userInputRequest
  const pendingRefineRef = useRef(h.pendingRefine)
  pendingRefineRef.current = h.pendingRefine
  const refineActiveRef = useRef(false)
  const refineOutputRef = useRef('')
  const refineStartTimeRef = useRef(0)
  const refineMsgIdRef = useRef<string | null>(null)

  // ── 提炼状态统一复位（四个出口共用，勿在各处复制）──
  // 出口：refine_failed 事件 / forced invoke 失败兜底 / 超时 guard / 手动关闭弹窗。
  // 复位提炼 refs（refineActiveRef 卡 true 会让后续 refine_prompt 被忽略、
  // execution_started 被误判为 refine 模式）+ 全部提炼 UI + 流式 refine 气泡。
  // 仅引用稳定值（useState setter / MutableRefObject.current），可安全跨闭包使用。
  const resetRefineUI = useCallback(() => {
    const msgId = refineMsgIdRef.current
    refineActiveRef.current = false
    refineStartTimeRef.current = 0
    refineOutputRef.current = ''
    refineMsgIdRef.current = null
    h.setRefining(false)
    h.setRefineState(null)
    h.setPendingRefine(null)
    h.setIsProcessing(false)
    h.refs.executionActiveRef.current = false
    h.refs.processingRef.current = false
    if (msgId) h.setMessages(prev => prev.filter(m => m.id !== msgId))
  }, [
    h.setRefining,
    h.setRefineState,
    h.setPendingRefine,
    h.setIsProcessing,
    h.setMessages,
    h.refs.executionActiveRef,
    h.refs.processingRef,
  ])
  // mode_changed 是用户切换模式的权威广播；execution_started 的 mode 只是执行事实。
  // 记录最近一次 mode_changed 值，execution_started 不再覆盖（防迟到旧执行事件把 mode 打回旧值）
  const lastModeChangedRef = useRef<string | null>(null)

  // ── nuphus-event listener ──
  useEffect(() => {
    let cancelled = false

    listen<{ seq: number; event: NuphusEvent }>('nuphus-event', ({ seq, event }) => {
      if (cancelled) return

      if (lastEventSeq.current > 0 && seq !== lastEventSeq.current + 1) {
        console.warn(
          `[EVENT] Seq jump: expected ${lastEventSeq.current + 1}, got ${seq} (lost ${seq - lastEventSeq.current - 1})`,
        )
      }
      lastEventSeq.current = seq
      lastEventTime.current = Date.now()
      eventCountRef.current++

      const sid = () => h.refs.streamingMsgId.current || h.refs.lastStreamingMsgId.current

      // ── Shared helpers (extracted duplicated patterns) ──
      const addSystemMsg = (content: string) =>
        h.addMessage({
          id: crypto.randomUUID(),
          role: 'system' as const,
          content,
          timestamp: Date.now(),
        })

      const finishWithMessage = (content: string, mood: MoodState) => {
        const s = sid()
        // Update existing streaming message, never create new
        if (s && content) {
          h.setMessages(prev =>
            prev.map(m => (m.id === s ? { ...m, content, runtime: 'done' } : m)),
          )
        }
        h.refs.streamingMsgId.current = null
        h.refs.executionActiveRef.current = false
        h.setIsProcessing(false)
        h.setPauseState(null)
        h.setMood(mood)
      }

      switch (event.type) {
        case 'direct_response': {
          finishWithMessage((event.message || '').trim(), 'success')
          break
        }
        case 'execution_error': {
          // LLM 执行中错误：立即播放错误音效（低沉三音下行）——用户不盯屏也能感知失败
          playUiSound('error')
          const s = h.refs.streamingMsgId.current
          if (s) {
            h.setMessages(prev =>
              prev.map(m => (m.id === s && m.runtime === 'live' ? { ...m, runtime: 'done' } : m)),
            )
          }
          h.refs.streamingMsgId.current = null
          h.refs.executionActiveRef.current = false
          h.setIsProcessing(false)
          h.setCompleted(true)
          h.setGoalType(null)
          h.setPauseState(null)
          h.setMood('error')
          break
        }
        case 'session_info':
          h.setModelName(event.model)
          if (event.session_id) h.setSessionId(event.session_id)
          break
        case 'understanding_complete':
          if (event.needs_clarification && event.critiques?.length > 0) {
            addSystemMsg(`我需要了解更多信息：${event.critiques[0]}`)
          }
          break
        case 'error':
          addSystemMsg(`遇到了问题：${event.message}`)
          if (!event.from_subtask) {
            // 主执行错误：播放错误音效（子任务错误不打断主执行，不提示）
            playUiSound('error')
            h.refs.executionActiveRef.current = false
            h.setIsProcessing(false)
            h.setCompleted(true)
            h.setGoalType(null)
            h.setMood('error')
          }
          break
        case 'warning': {
          invoke('hud_update', { text: event.message, phase: 'warning' })
          // LLM 重试提醒：收到重试告警（llm_retry / llm_network_retry）播放「咚咚」提示音，
          // 中性语义「还在重试、请稍候」——不是失败，不打断不恐慌。
          // 后端重试间隔带指数退避（2s/4s/8s...），不会连续轰炸。
          if (event.code === 'llm_retry' || event.code === 'llm_network_retry') {
            playUiSound('retry')
          }
          break
        }
        // user_message_received: 手机端/插件消息需在桌面补显用户气泡；
        // 桌面自己的消息走 handleSend 乐观更新（source==='desktop'），跳过避免重复
        case 'user_message_received': {
          const isMobile = event.source === 'mobile'
          const isPlugin = event.source?.startsWith('plugin:')
          if (!isMobile && !isPlugin) break
          h.addMessage({
            id: crypto.randomUUID(),
            role: 'user' as const,
            content: event.content,
            // 图片附件必须透传——后端事件已携带 images（events.rs UserMessageReceived），
            // 此前遗漏导致手机带图消息在桌面 user 气泡只显示文字、图片丢失（实测反馈）
            ...(event.images && event.images.length > 0 ? { images: event.images } : {}),
            // 插件消息带来源徽标（插件 id），与本人/手机消息可区分（审计链可视化）
            ...(isPlugin ? { sourceLabel: event.source.slice('plugin:'.length) } : {}),
            timestamp: Date.now(),
          })
          break
        }
        // mode_changed: 手机端 /switch-mode（与桌面 set_mode 共用后端 set_mode_impl）
        // 事件双推桌面 Tauri + 手机 WS；桌面端收到后同步 mode state——ChatInputBar
        // 的 mode chip / 状态随之更新（与 execution_started 里 setMode 同源写法）。
        // ⚠️ 只更新 mode chip，不重载历史、不切换会话（2026-08-30 解耦）：
        // 输入框 mode 与当前 session 解耦——mode 只管「下次发送的归属判定」，
        // session 刷新/切换只由会话台点击 / 新建 / 继续对话触发。
        case 'mode_changed':
          if (event.mode) {
            lastModeChangedRef.current = event.mode
            h.setMode?.(event.mode)
          }
          break
        case 'execution_started': {
          if (h.refs.executionActiveRef.current) {
            break
          }
          // Refine mode: set execution state but don't create a message bubble
          // Keep refineState intact — the modal should stay open until SessionRefined
          if (refineActiveRef.current) {
            h.setIsProcessing(true)
            h.refs.executionActiveRef.current = true
            h.refs.streamingMsgId.current = null
            h.setDismissThinking(false)
            h.setExecutionCounter((c: number) => c + 1)
            h.setStepIndex(event.step_index)
            h.setGoal(event.goal)
            h.setTimeline([])
            h.setCompleted(false)
            h.setTotalDurationMs(0)
            h.setTotalCalls(0)
            h.setExecTokenUsage(null)
            h.setExecPhase('understanding')
            h.refs.lastStreamingMsgId.current = null
            if (event.mode && h.setMode && !lastModeChangedRef.current) {
              h.setMode(event.mode)
            }
            break
          }
          h.setDismissThinking(false)
          h.setRefineState(null)
          h.setIsProcessing(true)
          h.setExecutionCounter((c: number) => c + 1)
          h.setStepIndex(event.step_index)
          h.setGoal(event.goal)
          h.setTimeline([])
          h.setCompleted(false)
          h.setTotalDurationMs(0)
          h.setTotalCalls(0)
          h.setExecTokenUsage(null)
          h.setExecPhase('understanding')
          h.refs.executionActiveRef.current = true
          h.refs.lastStreamingMsgId.current = null
          const streamId = crypto.randomUUID()
          h.refs.streamingMsgId.current = streamId
          h.addMessage({
            id: streamId,
            role: 'assistant',
            content: '',
            runtime: 'live',
            timestamp: Date.now(),
          })
          // Sync current mode（mode_changed 已权威驱动过 mode 时不覆盖，防迟到旧执行事件打回旧值）
          if (event.mode && h.setMode && !lastModeChangedRef.current) {
            h.setMode(event.mode)
          }
          break
        }
        case 'execution_paused':
          h.setPauseState({ actionId: event.action_id })
          break
        case 'goal_type_identified':
          h.setGoalType({ type: event.goal_type, label: event.label, confidence: event.confidence })
          break
        case 'seed_generated':
          addSystemMsg(`New evolution seed: ${event.summary}`)
          break
        case 'tool_call_start': {
          // 暂停菜单打开期间收到工具调用 = agent 已越过暂停检查点恢复执行（如手机端追加），
          // 自动关闭桌面暂停菜单，避免弹窗残留
          h.setPauseState(null)
          h.refs.toolCallCountRef.current++
          h.setExecPhase('executing')
          h.setTimeline((prev: TimelineEntry[]) => [
            ...prev,
            {
              id: event.call_id,
              kind: 'tool_call' as const,
              toolName: event.tool_name,
              params: event.params,
              status: 'running' as const,
              durationMs: 0,
              output: '',
              fromTask: event.from_task,
            },
          ])
          h.setMood(toolToMood[event.tool_name] || 'working')

          // planner_update → task_status changed (from inline tool call)
          if (event.tool_name === 'planner_update' && !event.from_task) {
            h.setTaskBubbleVisible(true)
          }
          break
        }
        case 'tool_output_line': {
          const line = event.line + (event.line.endsWith('\n') ? '' : '\n')
          h.setTimeline((prev: TimelineEntry[]) => {
            const idx = prev.findIndex(tc => tc.id === event.call_id && tc.kind === 'tool_call')
            if (idx === -1) return prev
            const entry = prev[idx]
            const updated = {
              ...entry,
              output: (entry.output || '') + line,
              outputLines: [...(entry.outputLines || []), line],
            }
            const next = [...prev]
            next[idx] = updated
            return next
          })
          break
        }
        case 'tool_call_end': {
          h.setTimeline((prev: TimelineEntry[]) =>
            prev.map(tc =>
              tc.id === event.call_id && tc.kind === 'tool_call'
                ? {
                    ...tc,
                    status: event.success ? ('success' as const) : ('error' as const),
                    durationMs: event.duration_ms,
                    output: tc.output || event.output_preview || '',
                    outputFullSize: event.output_full_size,
                    isTruncated: event.is_truncated,
                  }
                : tc,
            ),
          )
          // planner_create complete → show modal directly
          if (event.tool_name === 'planner_create' && event.success) {
            try {
              const output = JSON.parse(event.output_preview)
              if (output.plan && output.plan_path) {
                const plan = output.plan
                h.setPlanData({
                  project: plan.project || 'nuphus',
                  topic: plan.topic || '计划详情',
                  goalType: plan.goal_type || 'code_generation',
                  requirement: plan.requirement || '',
                  status: plan.status || 'active',
                  context: plan.context || '',
                  tasks: (plan.tasks || []).map(
                    (t: {
                      id?: number
                      name?: string
                      understanding?: string
                      priority?: string
                    }) => ({
                      id: t.id || 0,
                      name: t.name || `方向-${t.id}`,
                      understanding: t.understanding || '',
                      status: 'pending' as const,
                      priority: (t.priority || 'medium') as 'high' | 'medium' | 'low',
                    }),
                  ),
                  planPath: output.plan_path,
                })
                h.setShowReview(false)
                h.setShowPlannerModal(true)
              }
            } catch (e) {
              console.error('Failed to parse planner_create output:', e)
            }
          }
          // planner_update → update task status from exec agent
          if (event.tool_name === 'planner_update' && event.success) {
            try {
              const output = JSON.parse(event.output_preview)
              if (output.task_id && output.status) {
                h.transitionTask?.(output.task_id, output.status)
              }
            } catch (e) {
              console.error('Failed to parse planner_update output:', e)
            }
          }
          // task_dispatch → update task status from plan_update embedded in output
          if (event.tool_name === 'task_dispatch' && event.success) {
            try {
              const output = JSON.parse(event.output_preview)
              const pu = output.plan_update
              if (pu) {
                if (pu.task_id && pu.status) {
                  h.transitionTask?.(pu.task_id, pu.status)
                }
                if (pu.tasks) {
                  for (const t of pu.tasks) {
                    if (t.task_id && t.status) {
                      h.transitionTask?.(t.task_id, t.status)
                    }
                  }
                }
              }
            } catch (e) {
              console.error('Failed to parse task_dispatch output:', e)
            }
          }

          // tenet_add → approval modal
          if (event.tool_name === 'tenet_add' && event.success) {
            try {
              const output = event.output_preview
              const actionIdMatch = output.match(/action_id=([^\s。]+)/)
              const actionId = actionIdMatch ? actionIdMatch[1] : ''
              if (actionId) {
                Promise.all([
                  invoke<{ title: string; content: string; kind: string }>('get_pending_details', {
                    actionId,
                  }),
                  invoke<{ count: number }>('get_tenets').catch(() => ({ count: 0 })),
                ])
                  .then(([details, tenets]) => {
                    if (!details) return
                    h.setApprovalState({
                      open: true,
                      kind: details.kind || 'tenet',
                      title: details.title,
                      content: details.content,
                      actionId,
                      tenetCount: tenets?.count ?? 0,
                    })
                  })
                  .catch(e => console.error('Failed to handle tenet_add:', e))
              }
            } catch (e) {
              console.error('Failed to parse tenet_add output:', e)
            }
          }
          // Error status auto-dismisses after 5s (timeline is cleared on new execution_started, no cleanup needed)
          if (!event.success) {
            const callId = event.call_id
            const tid = window.setTimeout(() => {
              h.setTimeline((prev: TimelineEntry[]) =>
                prev.map(tc =>
                  tc.id === callId && tc.kind === 'tool_call' && tc.status === 'error'
                    ? { ...tc, status: 'success' as const }
                    : tc,
                ),
              )
            }, 5000)
            // Store timeout IDs for cleanup
            if (!errorTimeoutsRef.current) errorTimeoutsRef.current = []
            errorTimeoutsRef.current.push(tid)
          }
          break
        }
        case 'llm_text_delta':
          // 暂停菜单打开期间收到 LLM 流式文本 = agent 已越过暂停检查点恢复执行
          // （如手机端追加后先思考再调工具），自动关闭桌面暂停菜单
          h.setPauseState(null)
          h.setMood('thinking')
          // Refine mode: route text to refineOutput instead of message bubble
          if (refineActiveRef.current) {
            // 正文 delta → 提炼气泡流式渲染（后端 RefineStreamFilter 放行 LlmTextDelta）；
            // thinking delta 忽略——提炼思考不进执行轨迹 timeline（session_refined
            // 不清 timeline，残留会一直挂到下次执行覆盖）
            if (!event.is_thinking) {
              refineOutputRef.current += event.text
              const msgId = refineMsgIdRef.current
              if (msgId) {
                h.setMessages((prev: ChatMessage[]) =>
                  prev.map(m => (m.id === msgId ? { ...m, content: refineOutputRef.current } : m)),
                )
              }
            }
            break
          }
          {
            const s = h.refs.streamingMsgId.current
            if (!event.is_thinking && !event.from_task && s) {
              h.setMessages((prev: ChatMessage[]) =>
                prev.map(m => (m.id === s ? { ...m, content: m.content + event.text } : m)),
              )
            }
            const kind = event.is_thinking ? ('thinking' as const) : ('text' as const)
            h.setTimeline((prev: TimelineEntry[]) => {
              const last = prev[prev.length - 1]
              if (last && last.kind === kind) {
                const updated = [...prev]
                updated[updated.length - 1] = { ...last, text: (last.text || '') + event.text }
                return updated
              }
              return [...prev, { id: crypto.randomUUID(), kind, text: event.text }]
            })
          }
          break
        case 'image_generated': {
          const s = h.refs.streamingMsgId.current
          if (s && event.url) {
            h.setMessages((prev: ChatMessage[]) =>
              prev.map(m => (m.id === s ? { ...m, images: [...(m.images || []), event.url] } : m)),
            )
          }
          break
        }
        case 'execution_progress':
          h.setProgress({
            iteration: event.iteration,
            max: event.max_iterations,
            calls: event.tool_calls_so_far,
          })
          break
        case 'execution_completed': {
          const finalMsg = event.output?.result_message || ''
          const s = sid()
          // Refine 模式：execution_completed 的 result_message 是后端 resume 内部
          // 生成的提炼摘要（已经 llm_text_delta → refine 气泡路由显示，且 session_refined
          // 会用 event.summary 最终更新 refine 气泡）。此时 sid() 仍指向 refine 前最后一条
          // 真实 agent 回复气泡（lastStreamingMsgId 尚未清空），若用摘要覆盖会把用户可见的
          // 最终回复替换成 refine 内容。refine 模式下跳过覆盖。
          if (s && !refineActiveRef.current) {
            // 流式期间 content 可能已累积含 think 块边界残留的空白/换行
            // （process_text_delta 跨 chunk 折叠不完美）。execution_completed 的
            // result_message 是后端 extract_think_blocks 处理过的权威干净文本——
            // finalMsg 非空时无条件覆盖，避免"中间的空格都是 thinking 的 chars"。
            const content = finalMsg.trim() ? finalMsg : '（已执行完成，未产出回复）'
            h.setMessages((prev: ChatMessage[]) =>
              prev.map(m =>
                m.id === s
                  ? {
                      ...m,
                      content: finalMsg.trim() ? finalMsg : m.content || content,
                      runtime: 'done',
                    }
                  : m,
              ),
            )
          }
          if (
            event.output?.tool_calls_count &&
            h.refs.toolCallCountRef.current < event.output.tool_calls_count
          ) {
            console.warn(
              `[EVENT] Tool call count mismatch: expected ${event.output.tool_calls_count}, got ${h.refs.toolCallCountRef.current} (lost ${event.output.tool_calls_count - h.refs.toolCallCountRef.current})`,
            )
          }
          h.refs.toolCallCountRef.current = 0
          h.setCompleted(true)
          getCurrentWindow().setFocus()
          h.setIsProcessing(false)
          h.setPauseState(null)
          h.refs.processingRef.current = false
          h.setDismissThinking(false)
          h.setTotalDurationMs(event.total_duration_ms || 0)
          h.setTotalCalls(event.total_calls || 0)
          h.refs.lastStreamingMsgId.current = h.refs.streamingMsgId.current
          h.refs.streamingMsgId.current = null
          // Refine mode: let session_refined handle state cleanup,
          // but always clear executionActiveRef so refine's internal
          // ExecutionStarted won't be blocked by the guard at L224.
          if (!refineActiveRef.current) {
            h.setRefineState(null)
          }
          h.refs.executionActiveRef.current = false
          h.setExecPhase('recording')
          invoke('hud_update', { text: '执行完成', phase: 'done' })
          setTimeout(() => h.setExecPhase(''), 2000)
          h.setCurrentTaskDesc('')
          h.setMood('success')
          setTimeout(() => h.setMood('idle'), 3000)
          break
        }
        case 'security_check':
          h.setSecurity({
            actionId: event.action_id,
            tool: event.tool,
            params: event.params,
            risk: event.risk,
            reason: event.reason,
          })
          break
        case 'user_input_request':
          h.setUserInputRequest({
            actionId: event.action_id,
            title: event.title,
            prompt: event.prompt,
            sensitive: event.sensitive,
            inputType: event.input_type,
            iconPath: event.icon_path,
            defaultName: event.default_name,
            defaultShortcut: event.default_shortcut,
            relX: event.rel_x,
            relY: event.rel_y,
            defaultNote: event.default_note,
          })
          break
        case 'prompt_timeout':
          // 后端等待超时/取消 → 清除对应 action_id 的安全弹窗与输入请求弹窗
          h.setSecurity(prev => (prev && prev.actionId === event.action_id ? null : prev))
          if (
            userInputRequestRef.current &&
            userInputRequestRef.current.actionId === event.action_id
          ) {
            h.setUserInputRequest(null)
          }
          break
        case 'agent_reminder':
          h.setTimeline((prev: TimelineEntry[]) => [
            ...prev,
            {
              id: crypto.randomUUID(),
              kind: 'reminder' as const,
              text: event.text,
              count: event.count,
              maxCount: event.max_count,
            },
          ])
          break
        case 'task_started':
          h.setTaskBubbleVisible(true)
          h.setCurrentTaskDesc(event.description)
          h.transitionTask?.(event.task_id, 'in_progress')
          h.setTimeline((prev: TimelineEntry[]) => [
            ...prev,
            {
              id: `task-${event.task_id}`,
              kind: 'task' as const,
              text: event.description,
              taskId: event.task_id,
              totalTasks: event.total_tasks,
              status: 'running',
            },
          ])
          break
        case 'task_completed':
          h.setTaskBubbleVisible(true)
          h.transitionTask?.(event.task_id, 'completed')
          h.setTimeline((prev: TimelineEntry[]) =>
            prev.map(t => {
              if (t.kind === 'task' && t.taskId === event.task_id) {
                return { ...t, status: event.success ? 'success' : 'error', summary: event.summary }
              }
              return t
            }),
          )
          break
        case 'task_list':
          h.setTaskBubbleVisible(true)
          if (event.tasks && h.setPlanData) {
            h.setPlanData((prev: PlanData | null) => {
              if (!prev) return prev
              const updatedTasks = prev.tasks?.map((t: PlanTask) => {
                const found = event.tasks.find(
                  (et: { id: number; status: string }) => et.id === t.id,
                )
                return found ? { ...t, status: found.status as TaskStatus } : t
              })
              return { ...prev, tasks: updatedTasks }
            })
          }
          break
        case 'token_usage':
          console.log(
            `[TRACE-TOKEN] source=${event.source}, input=${event.input_tokens}, output=${event.output_tokens}, cacheHit=${event.cache_hit_tokens}, eventCount=${eventCountRef.current}, execActive=${h.refs.executionActiveRef.current}, processing=${h.refs.processingRef.current}`,
          )
          ;(() => {
            const update = (setter: React.Dispatch<React.SetStateAction<TokenUsageState>>) =>
              setter((prev: TokenUsageState) => {
                const cacheHit =
                  event.cache_hit_tokens === 0xffffffff
                    ? (prev?.cacheHitTokens ?? 0)
                    : event.cache_hit_tokens
                return {
                  inputTokens: event.input_tokens,
                  outputTokens: event.output_tokens,
                  cacheHitTokens: cacheHit,
                }
              })
            if (event.source === 'main') update(h.setMainTokenUsage)
            else update(h.setExecTokenUsage)
          })()
          break
        case 'refine_prompt':
          // context_window 优先（新后端），refine_limit 兜底（旧后端兼容）
          const win = event.context_window || event.refine_limit || 0
          const pct =
            event.current_tokens && win ? Math.round((event.current_tokens / win) * 100) : 0

          // forced=true → 强制提炼，清空 pendingRefine
          if (event.forced) {
            h.setPendingRefine(null)
            if (refineActiveRef.current) break // already refining
            refineActiveRef.current = true
            // ⚠️ 本地立即置 refining（不能依赖后续 RefineExecuting 事件）：
            // refineActiveRef 已置 true，后端 RefineExecuting 到达时会被下方
            // refine_executing case 的 `if (refineActiveRef.current) break` 吞掉，
            // setRefining(true) 永不执行 → refining=false → 提炼执行中 refine
            // 弹窗仍显示可操作态（可再次触发第二次 refine）。
            h.setRefining(true)
            refineStartTimeRef.current = Date.now()
            refineOutputRef.current = ''
            const refineMsgId = crypto.randomUUID()
            refineMsgIdRef.current = refineMsgId
            h.setMessages((prev: ChatMessage[]) => [
              ...prev,
              {
                id: refineMsgId,
                role: 'refine' as const,
                content: '',
                timestamp: Date.now(),
                messageCount: 0,
                sessionId: '',
                refineStatus: 'streaming' as const,
              },
            ])
            h.setIsProcessing(true)
            h.setRefineState({ usagePercent: 0, totalLimit: 0 })
            import('../main-window/lib/api').then(({ executeSessionRefine }) => {
              executeSessionRefine().catch((err: unknown) => {
                // 后端防重拒绝（提炼进行中）说明另一端/本端已真实在提炼：
                // 不 reset（否则 refineActiveRef=false + refining=false 会破坏真实
                // 提炼的 UI 锁，弹窗/入口提前重新可触发）——等 RefineExecuting /
                // session_refined / refine_failed 事件权威收敛。
                const msg = err instanceof Error ? err.message : String(err ?? '')
                if (msg.includes('提炼进行中')) return
                // RefineFailed 事件是主复位路径；此处兜底 invoke 层失败（事件
                // 丢失/旧后端）——静默吞掉会导致 forced 弹窗永久 spinner
                resetRefineUI()
              })
            })
            break
          }

          // 提炼执行中收到新的 refine_prompt（refineActiveRef=true）：忽略——
          // 正在提炼会话，不再弹新提示/累计跳过（否则提炼结束后残留 refineState/
          // pendingRefine 会让弹窗/入口重新可触发，锁释放语义破坏）。
          if (refineActiveRef.current) break

          // 如果已经有 pendingRefine（用户已跳过弹窗）→ 只更新数据，不弹窗
          if (pendingRefineRef.current) {
            h.setPendingRefine(prev =>
              prev
                ? {
                    ...prev,
                    usagePercent: pct,
                    skippedTurns: prev.skippedTurns + 1,
                  }
                : null,
            )
            break
          }

          // 正常弹窗（现有逻辑）
          h.setRefineState({ usagePercent: pct, totalLimit: win })
          break
        case 'refine_skipped':
          // 另一端（手机/桌面）跳过了提炼：本端同步关闭弹窗 + 记录跳过（防重复弹窗）。
          // 提炼已开始则跳过无效（refine 正在执行中）。
          if (refineActiveRef.current) break
          h.setRefining(false)
          if (refineStateRef.current) {
            h.setPendingRefine({
              usagePercent: refineStateRef.current.usagePercent,
              totalLimit: refineStateRef.current.totalLimit,
              skippedTurns: 0,
            })
          }
          h.setRefineState(null)
          break
        case 'refine_executing':
          // Guard: don't reset if already refining (prevents double-call from resetting output)
          if (refineActiveRef.current) break
          refineActiveRef.current = true
          refineStartTimeRef.current = Date.now()
          refineOutputRef.current = ''
          const execRefineMsgId = crypto.randomUUID()
          refineMsgIdRef.current = execRefineMsgId
          h.setMessages((prev: ChatMessage[]) => [
            ...prev,
            {
              id: execRefineMsgId,
              role: 'refine' as const,
              content: '',
              timestamp: Date.now(),
              messageCount: 0,
              sessionId: '',
              refineStatus: 'streaming' as const,
            },
          ])
          h.setIsProcessing(true)
          // 后端确认开始提炼 → 全局提炼中状态（弹窗/按钮路径统一遮罩）
          h.setRefining(true)
          break
        case 'session_refined':
          refineActiveRef.current = false
          refineStartTimeRef.current = 0
          // 提炼完成/失败（summary 可能为空）：恢复提炼中遮罩——失败也不能让
          // 用户困在全屏遮罩里
          h.setRefining(false)
          const refinedMsgId = refineMsgIdRef.current
          refineMsgIdRef.current = null
          // Reset all execution state so the next send won't be blocked
          h.refs.executionActiveRef.current = false
          h.refs.processingRef.current = false
          h.refs.streamingMsgId.current = null
          h.refs.lastStreamingMsgId.current = null
          h.setRefineState(null)
          h.setPendingRefine(null)
          h.setIsProcessing(false)
          h.setCompleted(true)
          h.setPauseState(null)
          h.setDismissThinking(false)
          h.setMood('success')
          setTimeout(() => h.setMood('idle'), 3000)
          if (refinedMsgId) {
            h.setMessages((prev: ChatMessage[]) =>
              prev.map(m =>
                m.id === refinedMsgId
                  ? {
                      ...m,
                      content: event.summary,
                      messageCount: event.message_count,
                      sessionId: event.session_id,
                      refineStatus: 'completed' as const,
                    }
                  : m,
              ),
            )
          }
          break
        case 'refine_failed': {
          // 提炼失败（LLM key 失效/连不上/超时/空摘要）：与 refine_executing 配对的
          // 结束事件。复位提炼状态、移除流式气泡、系统消息明示失败原因（后端
          // message 已含完整描述）——缺失此分支时弹窗/遮罩永久 spinner（假死根因）。
          resetRefineUI()
          h.setCompleted(true)
          h.setPauseState(null)
          h.setDismissThinking(false)
          h.setMood('error')
          setTimeout(() => h.setMood('idle'), 3000)
          addSystemMsg(event.message || '提炼失败，会话保持不变。')
          break
        }
      }
    }).then(fn => {
      if (cancelled) return
      unlistenRef.current?.()
      unlistenRef.current = fn
    })
    return () => {
      cancelled = true
      unlistenRef.current?.()
      errorTimeoutsRef.current.forEach(clearTimeout)
      errorTimeoutsRef.current = []
    }
  }, [
    h.refs.streamingMsgId,
    h.refs.lastStreamingMsgId,
    h.refs.executionActiveRef,
    h.refs.processingRef,
    h.refs.toolCallCountRef,
    h.addMessage,
    resetRefineUI,
  ])

  // ── 移动端安全确认回执：手机端完成确认后，桌面弹窗同步关闭 ──
  // （mobile_server POST /confirm 处理后由后端广播，非 NuphusEvent 协议）
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    listen<{ action_id: string }>('mobile-security-resolved', payload => {
      if (disposed) return
      h.setSecurity(prev => (prev && prev.actionId === payload.action_id ? null : prev))
    }).then(fn => {
      if (disposed) fn()
      else unlisten = fn
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [h.setSecurity])

  // ── 移动端输入回执：手机端提交/取消 request_user_input 后，桌面弹窗同步关闭 ──
  // （mobile_server POST /user-input(-reject) 处理后由后端广播）
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    listen<{ action_id: string }>('mobile-user-input-resolved', payload => {
      if (disposed) return
      if (
        userInputRequestRef.current &&
        userInputRequestRef.current.actionId === payload.action_id
      ) {
        h.setUserInputRequest(null)
      }
    }).then(fn => {
      if (disposed) fn()
      else unlisten = fn
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [h.setUserInputRequest])

  // ── Refine timeout guard: 事件彻底丢失时的兜底复位 ──
  // 95s > 后端 90s 硬超时：正常慢提炼不被误掐；旧版只复位 isProcessing 不复位
  // refining/refineState（弹窗仍卡 spinner）+ 70s 会误杀后端还在跑的提炼。
  useEffect(() => {
    const interval = setInterval(() => {
      if (refineActiveRef.current && refineStartTimeRef.current > 0) {
        const elapsed = Date.now() - refineStartTimeRef.current
        if (elapsed > 95000) {
          console.warn('[REFINE] Timeout guard triggered, resetting refine state')
          resetRefineUI()
        }
      }
    }, 5000)
    return () => clearInterval(interval)
  }, [resetRefineUI])

  // ── Heartbeat check: detect event stream stalls ──
  useEffect(() => {
    const interval = setInterval(async () => {
      const elapsed = Date.now() - lastEventTime.current
      if (elapsed > 8000 && h.refs.processingRef.current) {
        console.warn(
          `[EVENT] No events received for ${(elapsed / 1000).toFixed(0)}s (seq=${lastEventSeq.current})`,
        )
      }
      if (elapsed > 30000 && h.refs.processingRef.current) {
        console.warn(
          '[EVENT] Event stream stalled while processing active — possible IPC channel issue',
        )
      }
    }, 5000)
    return () => clearInterval(interval)
  }, [])

  // ── toolbar:action listener ──
  useEffect(() => {
    let unlisten: (() => void) | null = null

    listen<{ action: string }>('toolbar:action', async payload => {
      console.log('[Toolbar] action received:', payload)
      switch (payload.action) {
        case 'screenshot':
          h.setRegionPickerMode('capture')
          break
        case 'picker':
          h.setRegionPickerMode('picker')
          break
        case 'template':
          h.setRegionPickerMode('capture')
          break
        case 'ocr':
          h.setRegionPickerMode('ocr')
          break
        default:
          console.warn('[Toolbar] unknown action:', payload.action)
      }
    }).then(fn => {
      unlisten = fn
    })

    return () => {
      unlisten?.()
    }
  }, [h.setRegionPickerMode])

  // ── 手动关闭「提炼中」弹窗/遮罩 ──
  // 后台提炼不中断：完成后 session_refined / refine_failed 照常落地（结果入会话）。
  const dismissRefine = useCallback(() => {
    resetRefineUI()
  }, [resetRefineUI])

  return { dismissRefine }
}
