// useEvents.ts — Event listener (nuphus-event + toolbar:action)
import { useCallback, useEffect, useRef } from 'react'
import { invoke, listen } from '../core/bridge'
import { debugEnabled } from '../core/debug'
import { continueReplyAfterUser } from '../core/progressMessages'
import type { RefineState, RefineTier } from './useExecutionUI'
import { autoRaiseForceThreshold } from './useExecutionUI'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type {
  ChatMessage,
  NuphusEvent,
  SecurityCheck,
  UserInputRequest,
  TimelineEntry,
  PlanData,
  PlanTask,
  TaskRun,
  TurnMeta,
} from '../core/types'
import type { MutableRefObject } from 'react'
import type { ExecutionStage } from './useExecutionState'
import type { MoodState } from '../ui/MoodFace'
import { playUiSound } from '../ui/sound'
import { showAppFeedbackByHudPhase } from '../ui/islandChannel'
import type { ApiHealthState, ApiHealthEventKind, ApiHealthIncident } from '../core/types'
import { requestWorkflowEnhancedModeRefresh } from '../main-window/workflow-canvas/enhancedModeEvents'

type RegionPickerMode = 'picker' | 'capture' | 'ocr' | null
type TokenUsageState = {
  inputTokens: number
  outputTokens: number
  cacheHitTokens: number
  /** 解码速度 tok/s（exec 源事件携带；无数据时为 undefined） */
  genTps?: number
  /** 首 token 延迟毫秒（exec 源事件携带；无数据时为 undefined） */
  ttftMs?: number
} | null

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
    /** 本轮 token 累加基准（同步可读）：token_usage 连续到达时以闭包 prev 累加会漏加 */
    turnMetaTokensRef: MutableRefObject<TurnMeta>
    /** 用户已点击强制中断；置位后迟到的 tool_call 事件不再把 mood 打回执行中 */
    interruptedRef: MutableRefObject<boolean>
    /** ChatPanel 贴底跟随的 followReset 回填位：execution_started / execution_completed 调 */
    stickyFollowResetRef: MutableRefObject<(() => void) | null>
  }

  // State setters
  messages: ChatMessage[]
  setMessages: (v: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => void
  /**
   * 执行态置位（**唯一来源**，见 useExecutionState）。
   * 事件语义 → 阶段：execution_started → 'running'；execution_completed / error /
   * direct_response → 'finalizing'（本轮输出收敛，后端随后才结束收尾）；提炼复位 → 'idle'。
   */
  setExecutionStage: (stage: ExecutionStage) => void
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
  setAppendQueue: React.Dispatch<React.SetStateAction<string[]>>
  apiHealth: ApiHealthState
  setApiHealth: React.Dispatch<React.SetStateAction<ApiHealthState>>
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
  /** 本轮元数据（耗时 / token / 步数）——三处接入点共用的唯一数据源 */
  setTurnMeta: React.Dispatch<React.SetStateAction<TurnMeta | null>>
  /** 执行中实时工具调用累计 */
  setLiveTurnToolCalls: React.Dispatch<React.SetStateAction<number>>
  setPlanData: React.Dispatch<React.SetStateAction<PlanData | null>>
  /** ExecAgent 执行生命周期快照（task 面板唯一数据源） */
  setTaskRuns: React.Dispatch<React.SetStateAction<TaskRun[]>>
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
  setRefineState: React.Dispatch<React.SetStateAction<RefineState | null>>
  refineState: RefineState | null
  setRefining: (v: boolean) => void
  pendingRefine: (RefineState & { skippedTurns: number }) | null
  setPendingRefine: React.Dispatch<
    React.SetStateAction<(RefineState & { skippedTurns: number }) | null>
  >
  setDismissThinking: (v: boolean) => void
  setExecutionCounter: React.Dispatch<React.SetStateAction<number>>
  setModelName: (v: string) => void
  /** 当前模型名：仅用于识别"模型真的切换了"，不触发渲染 */
  modelName?: string
  /** 作废 refine 提示时一并清掉大窗口强制线的本地草稿 */
  setForceDraftForRefine?: (v: number | null) => void
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
  // 当前模型名：模型切换时要靠它判断"真的换了"，才能作废基于旧窗口的
  // refine 提示（见 session_info case）
  const modelNameRef = useRef(h.modelName ?? '')
  modelNameRef.current = h.modelName ?? ''
  const refineActiveRef = useRef(false)
  const refineOutputRef = useRef('')
  const refineStartTimeRef = useRef(0)
  const refineMsgIdRef = useRef<string | null>(null)
  const progressTurnRef = useRef<{ session_id: string; turn_id: string } | null>(null)
  const progressIdsRef = useRef(new Set<string>())

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
    // 提炼结束（失败/超时/手动关闭）：RefineGuard 已/即将释放执行态 → 本端归零，
    // 后端真值由执行态轮询复核（不一致会被下一轮轮询纠正）。
    h.setExecutionStage('idle')
    h.refs.executionActiveRef.current = false
    h.refs.processingRef.current = false
    if (msgId) h.setMessages(prev => prev.filter(m => m.id !== msgId))
  }, [
    h.setRefining,
    h.setRefineState,
    h.setPendingRefine,
    h.setExecutionStage,
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
    /** 连续重试计数：单次 → 瞬时脉冲；≥2 次 → 常驻 degraded（持续性问题才占位） */
    let retryStreak = 0
    /** 事件聚合：同 kind 合并计数（分类说明 + ×N + 首末时间），不产生重复行 */
    const upsertIncident = (
      prev: ApiHealthState,
      kind: ApiHealthEventKind,
      summary: string,
      impact: 'none' | 'partial' | 'failed',
      now: number,
    ): ApiHealthIncident[] => {
      // ?? [] 兼容 HMR 旧 state（records → incidents 迁移期间内存中的旧对象无该字段）
      const list = prev.incidents ?? []
      const idx = list.findIndex(i => i.kind === kind)
      if (idx < 0) {
        return [
          ...list,
          { kind, count: 1, firstAt: now, lastAt: now, lastSummary: summary, impact },
        ]
      }
      const next = [...list]
      const cur = next[idx]
      next[idx] = { ...cur, count: cur.count + 1, lastAt: now, lastSummary: summary, impact }
      // 分类种类天然有界（≤6），仍按最近发生保留 8 条兜底
      return next.sort((a, b) => a.lastAt - b.lastAt).slice(-8)
    }
    const setHealth = (
      status: ApiHealthState['status'],
      summary?: string,
      kind?: ApiHealthEventKind,
      impact?: 'none' | 'partial' | 'failed',
    ) => {
      h.setApiHealth(prev => {
        const now = Date.now()
        const nextStableSince = status === 'stable' ? (prev.stableSince ?? now) : null
        // 语义保持兼容：未显式指定时按 status 推导（offline=disconnect/failed，其余=retry/partial）
        const effKind: ApiHealthEventKind = kind ?? (status === 'offline' ? 'disconnect' : 'retry')
        const effImpact = impact ?? (status === 'offline' ? 'failed' : 'partial')
        const incidents = summary
          ? upsertIncident(prev, effKind, summary, effImpact, now)
          : (prev.incidents ?? [])
        return {
          ...prev,
          status,
          stableSince: nextStableSince,
          lastTransitionAt: now,
          // 恢复稳定 / 连接断开：上一次的重试进度已失效 → 清空（rail 不再显示过时数字）
          retry: status === 'stable' || status === 'offline' ? null : prev.retry,
          consecutiveFailures:
            status === 'offline'
              ? prev.consecutiveFailures + 1
              : status === 'stable'
                ? 0
                : prev.consecutiveFailures,
          incidents,
          unreadCount: summary ? prev.unreadCount + 1 : prev.unreadCount,
        }
      })
    }

    /** 瞬时事件脉冲（传输截断 / 单次重试）：一次性动效 + 聚合记录，不改变常驻 status */
    const setPulse = (kind: ApiHealthEventKind, summary?: string) => {
      const at = Date.now()
      h.setApiHealth(prev => ({
        ...prev,
        pulse: { kind, at },
        incidents: summary
          ? upsertIncident(prev, kind, summary, 'partial', at)
          : (prev.incidents ?? []),
        unreadCount: summary ? prev.unreadCount + 1 : prev.unreadCount,
      }))
      // 1.6s 后清除脉冲（仅清同一次，避免误清新到达的事件）
      setTimeout(() => {
        if (cancelled) return
        h.setApiHealth(prev => (prev.pulse?.at === at ? { ...prev, pulse: null } : prev))
      }, 1600)
    }

    /**
     * 收到真实 LLM 活动（text delta / tool call / 执行完成）= 连接已被证明可用 → 直接稳定。
     * 原实现每次活动都重置 10s 观察计时器，执行中高频活动使计时器永不到期，
     * 导致全程显示「正在连接」（2026-09-08 修正：取消观察窗续期）。
     */
    const observeStable = () => {
      retryStreak = 0
      h.setApiHealth(prev => {
        // 短路仅限「已 stable 且无待清的重试进度」：单次重试时 status 从未离开 stable
        // （retryStreak < 2 只走 setPulse），若此处按 status 一刀切返回，retry 永远不会被清，
        // `retry 1/3` 会永久驻留在 rail 上。retry 非空时必须走下面的清理分支。
        if (prev.status === 'stable' && !prev.retry) return prev
        const now = Date.now()
        // 异常 → 恢复：记一条 recovered（时间线闭环：异常与恢复成对出现）
        const wasUnhealthy = prev.status === 'degraded' || prev.status === 'offline'
        return {
          ...prev,
          status: 'stable',
          stableSince: prev.stableSince ?? now,
          consecutiveFailures: 0,
          retry: null,
          lastTransitionAt: now,
          incidents: wasUnhealthy
            ? upsertIncident(prev, 'recovered', '连接已恢复', 'none', now)
            : (prev.incidents ?? []),
        }
      })
    }

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

      if (event.type === 'execution_started')
        h.setApiHealth(prev => ({
          ...prev,
          currentTurnId: prev.currentTurnId + 1,
          status:
            prev.status === 'offline' || prev.status === 'unknown' ? 'connecting' : prev.status,
          lastTransitionAt: Date.now(),
        }))
      if (event.type === 'tool_call_start') observeStable()
      if (event.type === 'llm_text_delta') observeStable()
      if (event.type === 'execution_completed') {
        observeStable()
        h.setApiHealth(prev => ({ ...prev, unreadCount: 0 }))
      }
      if (
        event.type === 'warning' &&
        (event.code === 'llm_retry' || event.code === 'llm_network_retry')
      ) {
        // 结构化进度（后端 attempt / max_attempts）→ rail 的「retry 1/3」实时数字。
        // 直接取事件字段，不解析 message 文案；老后端不带该对字段时保持原值（不显示数字，也不编造）
        if (event.attempt != null && event.max_attempts != null) {
          const attempt = event.attempt
          const max = event.max_attempts
          h.setApiHealth(prev => ({ ...prev, retry: { attempt, max, at: Date.now() } }))
        }
        // 单次重试 = 瞬时脉冲（一闪而过）；连续 ≥2 次 = 常驻 degraded（持续性问题才占位）
        retryStreak += 1
        if (retryStreak >= 2)
          setHealth('degraded', '模型连接持续波动，系统正在恢复', 'retry', 'partial')
        else setPulse('retry', '模型连接出现波动，系统正在恢复')
      }
      if (event.type === 'warning' && event.code === 'stream_truncated') {
        // 传输截断 = 瞬时事件（一闪而过）+ 记录进 rail；不常驻 degraded
        setPulse('truncated', event.message || '响应传输中断，已保留部分内容')
      }
      if (event.type === 'error' && !event.from_subtask) setHealth('offline', '模型请求未能完成')

      const sid = () => h.refs.streamingMsgId.current || h.refs.lastStreamingMsgId.current

      // ── 本轮元数据同步到流式气泡 ──
      // turnMeta 是唯一数据源，但消息气泡读的是 `msg.meta`。若只在 execution_completed
      // 一次性落定，执行中的 token / 步数在气泡上不可见（ctx 弹窗有实时通道、气泡没有）。
      // 故每次 turnMeta 增量都把**同一字段**合并进当前流式消息的 meta：
      // 执行中实时可见、完成后被 completedMeta 权威覆盖、刷新后由 HistoryMessage.meta 恢复。
      // 只合并不覆盖：保留尚未下发的字段（如 startedAtMs 由 execution_started 单独写入）。
      const syncDraftMeta = (patch: TurnMeta) => {
        const s = h.refs.streamingMsgId.current
        if (!s) return
        h.setMessages(prev =>
          prev.map(m => (m.id === s ? { ...m, meta: { ...(m.meta ?? {}), ...patch } } : m)),
        )
      }

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
            continueReplyAfterUser(prev, s).map(m =>
              m.id === s ? { ...m, content, runtime: 'done' } : m,
            ),
          )
        }
        h.refs.streamingMsgId.current = null
        h.refs.executionActiveRef.current = false
        // 本轮输出已收敛（direct_response）：后端仍可能在收尾 → Finalizing
        h.setExecutionStage('finalizing')
        h.setPauseState(null)
        h.setAppendQueue([])
        h.setMood(mood)
      }

      switch (event.type) {
        case 'session_changed':
        case 'new_chat_broadcast':
          progressTurnRef.current = null
          progressIdsRef.current.clear()
          // 会话增强偏好由后端按 Workflow session 保存。会话切换或新建后让所有
          // 已挂载的入口重新读取权威状态，避免按钮仍展示上一会话的开关与配置徽标。
          requestWorkflowEnhancedModeRefresh()
          break
        case 'direct_response': {
          finishWithMessage((event.message || '').trim(), 'success')
          break
        }
        case 'execution_error': {
          const isInterrupted = h.refs.interruptedRef.current
          // 用户中断：不播错误音效、mood 回到 idle（中断不是失败）；
          // 其余 LLM 错误：立即播放错误音效（低沉三音下行）——用户不盯屏也能感知失败
          if (!isInterrupted) playUiSound('error')
          const s = h.refs.streamingMsgId.current
          if (s) {
            h.setMessages(prev =>
              prev.map(m => (m.id === s && m.runtime === 'live' ? { ...m, runtime: 'done' } : m)),
            )
          }
          h.refs.streamingMsgId.current = null
          h.refs.executionActiveRef.current = false
          h.refs.interruptedRef.current = false
          // 执行失败/中断：本轮收敛，后端随后结束收尾（Finalizing → 轮询收敛 idle）
          h.setExecutionStage('finalizing')
          h.setCompleted(true)
          h.setGoalType(null)
          h.setPauseState(null)
          h.setAppendQueue([])
          h.setMood(isInterrupted ? 'idle' : 'error')
          break
        }
        case 'session_info': {
          // 模型切换 → 上下文窗口变了，待确认的 refine 提示全部作废。
          // usagePercent 是按旧窗口算的，tier/阈值也基于旧档位；留着会让用户
          // 拿过期数据做决定（例如在 1M 模型的提示里拖滑杆，而当前已是 128K
          // 模型——那个滑杆对 small 档毫无作用）。清除比保留更诚实。
          const modelChanged = event.model && event.model !== modelNameRef.current
          if (modelChanged && (refineStateRef.current || pendingRefineRef.current)) {
            h.setRefineState(null)
            h.setPendingRefine(null)
            h.setForceDraftForRefine?.(null)
            addSystemMsg('模型已切换，上下文提示已按新的上下文窗口重新计算。')
          }
          h.setModelName(event.model)
          // ⚠️ SessionInfo.session_id 是**每轮随机生成的事件关联 id**（后端
          // uuid::new_v4()），不是会话 id——会话 id 的唯一权威来源是
          // execution_started.session_id（见该 case 的写入）。曾经在此处
          // 消费它导致点评等消费关联到假会话（2026-10 纠正，勿再加回）。
          break
        }
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
            h.setExecutionStage('finalizing')
            h.setCompleted(true)
            h.setAppendQueue([])
            h.setGoalType(null)
            h.setMood('error')
          }
          break
        case 'warning': {
          showAppFeedbackByHudPhase(event.message, 'warning')
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
          progressTurnRef.current =
            event.session_id && event.turn_id
              ? { session_id: event.session_id, turn_id: event.turn_id }
              : null
          // 真实会话 id 的唯一权威来源：execution_started.session_id（后端
          // rt.session().id）——点评提交（useAgentControl submitExecutionRating）
          // 等消费方据此关联后端会话记录。SessionInfo.session_id 是每轮随机
          // 事件关联 id、**不是会话 id**（曾误消费，点评关联到假会话，2026-10 纠）。
          if (event.session_id) h.setSessionId(event.session_id)
          progressIdsRef.current.clear()
          // Refine mode: set execution state but don't create a message bubble
          // Keep refineState intact — the modal should stay open until SessionRefined
          if (refineActiveRef.current) {
            h.setExecutionStage('running')
            h.refs.executionActiveRef.current = true
            h.refs.streamingMsgId.current = null
            h.setDismissThinking(false)
            h.setExecutionCounter((c: number) => c + 1)
            h.setStepIndex(event.step_index)
            h.setGoal(event.goal)
            h.setTimeline([])
            // 新一轮 = 新的一面墙：task 面板按「轮」归零，不与上一轮的派发混在一起
            h.setTaskRuns([])
            h.setCompleted(false)
            h.setTotalDurationMs(0)
            h.setTotalCalls(0)
            // 本轮元数据重置为「后端权威起点」——刷新后此值仍指向真实起点，
            // 前端据此推算耗时不会归零。token/步数随后续事件累加。
            h.refs.turnMetaTokensRef.current = {}
            h.setTurnMeta(event.started_at_ms != null ? { startedAtMs: event.started_at_ms } : null)
            h.setLiveTurnToolCalls(0)
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
          h.setExecutionStage('running')
          // 新轮次开始：恢复贴底跟随（用户可能停在上一轮的上翻位置），后续流式 delta
          // 继续下拉。refine 分支不建流式气泡（上方已 break），无需 reset。
          h.refs.stickyFollowResetRef.current?.()
          h.setExecutionCounter((c: number) => c + 1)
          h.setStepIndex(event.step_index)
          h.setGoal(event.goal)
          h.setTimeline([])
          // 新一轮 = 新的一面墙：task 面板按「轮」归零，不与上一轮的派发混在一起
          h.setTaskRuns([])
          h.setCompleted(false)
          h.setTotalDurationMs(0)
          h.setTotalCalls(0)
          // 本轮元数据重置为「后端权威起点」（同 refine 分支），token/步数随后续事件累加
          h.refs.turnMetaTokensRef.current = {}
          h.setTurnMeta(event.started_at_ms != null ? { startedAtMs: event.started_at_ms } : null)
          h.setLiveTurnToolCalls(0)
          h.setExecTokenUsage(null)
          h.setExecPhase('understanding')
          h.refs.executionActiveRef.current = true
          h.refs.lastStreamingMsgId.current = null
          // 新一轮执行开始：清除中断标记（此前中断状态已收敛）
          h.refs.interruptedRef.current = false
          const streamId = crypto.randomUUID()
          h.refs.streamingMsgId.current = streamId
          h.addMessage({
            id: streamId,
            role: 'assistant',
            content: '',
            runtime: 'live',
            timestamp: Date.now(),
          })
          // 起点立刻落到气泡：执行中即可用 `Date.now() - startedAtMs` 推算耗时，
          // 刷新后（durationMs 缺失）也能从 msg.meta.startedAtMs 恢复推算
          if (event.started_at_ms != null) syncDraftMeta({ startedAtMs: event.started_at_ms })
          // Sync current mode（mode_changed 已权威驱动过 mode 时不覆盖，防迟到旧执行事件打回旧值）
          if (event.mode && h.setMode && !lastModeChangedRef.current) {
            h.setMode(event.mode)
          }
          break
        }
        case 'execution_paused':
          h.setPauseState({ actionId: event.action_id })
          break
        case 'append_queue_updated':
          h.setAppendQueue(event.messages)
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
          // 步数**不在这里累加**：后端已在 emit ToolCallStart 时累加进 SignalState，
          // 并经 get_execution_state 快照下发。前端自己数会在刷新 / 丢事件后与实际不符。
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
          h.setMood(
            h.refs.interruptedRef.current ? 'idle' : toolToMood[event.tool_name] || 'working',
          )
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
          // 注意：这里不再解析任何"任务状态"。task_dispatch 的生命周期由服务端台账
          // 记账并经 `task_runs` 快照下发（见本文件 task_runs 分支），
          // 历史上从 Exec 摘要里刨 plan_update JSON 的软耦合已删除。

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
        case 'assistant_progress': {
          const turn = progressTurnRef.current
          if (
            refineActiveRef.current ||
            !h.refs.executionActiveRef.current ||
            h.refs.interruptedRef.current ||
            !turn ||
            turn.session_id !== event.session_id ||
            turn.turn_id !== event.turn_id ||
            progressIdsRef.current.has(event.message_id) ||
            !event.text.trim()
          )
            break
          progressIdsRef.current.add(event.message_id)
          const draftId = h.refs.streamingMsgId.current
          if (!draftId) break
          h.setMessages(prev => {
            if (prev.some(m => m.message_id === event.message_id)) return prev
            prev = continueReplyAfterUser(prev, draftId)
            const index = prev.findIndex(m => m.id === draftId)
            if (index < 0) return prev
            const draft = prev[index]
            const progress: ChatMessage = {
              id: event.message_id,
              reply_id: draft.reply_id ?? draft.id,
              message_id: event.message_id,
              kind: 'progress',
              role: 'assistant',
              content: event.text,
              timestamp: event.timestamp,
              runtime: 'done',
            }
            return [
              ...prev.slice(0, index),
              progress,
              event.replaces_draft
                ? { ...draft, content: '', timestamp: event.timestamp }
                : draft.content
                  ? draft
                  : { ...draft, timestamp: event.timestamp },
              ...prev.slice(index + 1),
            ]
          })
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
                continueReplyAfterUser(prev, s).map(m =>
                  m.id === s ? { ...m, content: m.content + event.text } : m,
                ),
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
          // 同步本轮元数据的「步数」维度：迭代轮次 + 工具调用累计。
          // 只更新对应字段，保留 started_at_ms 起点与其它已累加的 token 值。
          h.setTurnMeta(prev => ({
            ...(prev ?? {}),
            iterations: event.iteration,
            toolCalls: event.tool_calls_so_far,
          }))
          // 同值落到流式气泡：气泡的「步数」chip 读 msg.meta.toolCalls
          syncDraftMeta({ iterations: event.iteration, toolCalls: event.tool_calls_so_far })
          h.setLiveTurnToolCalls(event.tool_calls_so_far)
          break
        case 'execution_completed': {
          const finalMsg = event.output?.result_message || ''
          const s = sid()
          // ── 本轮元数据：以后端权威值落定 ──
          // meta / durationMs / total_calls 全部来自后端（execution_completed 事件），
          // 前端**不参与计算、不补任何本地值**——步数与耗时都由后端单一来源给出。
          // 执行中已累加的字段（token / startedAtMs）沿用 prev，不用 undefined 冲掉。
          const completedMeta: TurnMeta = {
            ...(event.meta ?? {}),
            durationMs: event.meta?.durationMs ?? event.total_duration_ms,
            toolCalls: event.meta?.toolCalls ?? event.total_calls ?? undefined,
          }
          // 与执行中已累积的元数据合并：缺失字段沿用 prev，不用 undefined 冲掉已有值
          h.setTurnMeta(prev => ({
            ...prev,
            ...completedMeta,
            startedAtMs: completedMeta.startedAtMs ?? prev?.startedAtMs,
            durationMs: completedMeta.durationMs ?? prev?.durationMs,
            toolCalls: completedMeta.toolCalls || prev?.toolCalls,
          }))
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
              continueReplyAfterUser(prev, s).map(m =>
                m.id === s
                  ? {
                      ...m,
                      content: finalMsg.trim() ? finalMsg : m.content || content,
                      runtime: 'done',
                      // 完成元数据落到该轮 assistant 气泡：历史消息重开（applyHistory
                      // 由后端 HistoryMessage.meta 回填）与实时路径共用同一字段，
                      // 消息底部 <TurnMetaBar> 读它，无需另建一处计时。
                      meta: completedMeta,
                    }
                  : m,
              ),
            )
          }
          // 步数一致性由后端单一来源保证（SignalState 累加 + execution_completed 下发），
          // 前端不参与计数，故不再需要「本地计数 vs 后端计数」的 mismatch 比对。
          h.setCompleted(true)
          // 完成任务瞬间补拉一次（非执行态唯一的自动下拉）：成果落地即下拉展示——
          // 用户上翻+空闲不会被拉，两样和睦共处。refine 内部子执行不产 chat 气泡，
          // 跳过以免把正在翻历史的用户拽回底部。
          if (!refineActiveRef.current) {
            h.refs.stickyFollowResetRef.current?.()
          }
          getCurrentWindow().setFocus()
          // 最终回复已到达，但后端随即进入收尾（记忆落盘 / 自动提炼）——
          // 阶段为 Finalizing 而非 idle：收尾期提交必须被拒收退回输入框，
          // 不能再把它当「空闲」受理（原缺陷：收尾期追加被静默入队且永不执行）。
          h.setExecutionStage('finalizing')
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
          // 「执行完成」：原 HUD 相位 'done' → island success（映射见 islandChannel）
          showAppFeedbackByHudPhase('执行完成', 'done')
          setTimeout(() => h.setExecPhase(''), 2000)
          h.setMood('success')
          setTimeout(() => h.setMood('idle'), 3000)
          break
        }
        case 'security_check':
          if (event.tool === 'desktop_action_approval') {
            let details: { title?: string; content?: string } = {}
            try {
              const parsed = JSON.parse(event.params)
              if (parsed && typeof parsed === 'object') details = parsed
            } catch {
              // The trusted host normally sends a compact display payload.
            }
            h.setApprovalState({
              open: true,
              kind: 'desktop_action',
              title: typeof details.title === 'string' ? details.title : event.reason,
              content: typeof details.content === 'string' ? details.content : event.reason,
              actionId: event.action_id,
              tenetCount: 0,
            })
            break
          }
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
            defaultStage: event.default_stage,
          })
          break
        case 'prompt_timeout':
          // 后端等待超时/取消 → 清除对应 action_id 的安全弹窗与输入请求弹窗
          h.setSecurity(prev => (prev && prev.actionId === event.action_id ? null : prev))
          h.setApprovalState(prev =>
            prev.actionId === event.action_id ? { ...prev, open: false } : prev,
          )
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
        // ExecAgent 执行生命周期**全量快照**（后端 agent::task_run）。
        // 前端是纯投影：不推断 id、不配对事件、不维护状态机——丢失一个快照最多旧一帧，
        // 下一次变迁自愈。执行面板的 task 行与 TaskBubble 共用这一份数据。
        case 'task_runs': {
          const runs = event.runs
          h.setTaskRuns(runs)
          h.setTaskBubbleVisible(runs.length > 0)
          h.setTimeline((prev: TimelineEntry[]) => {
            const byId = new Map(runs.map(r => [r.run_id, r]))
            const rowFor = (r: TaskRun): TimelineEntry => ({
              id: `task-${r.run_id}`,
              kind: 'task' as const,
              runId: r.run_id,
              text: r.title,
              status:
                r.state === 'running'
                  ? ('running' as const)
                  : r.ok
                    ? ('success' as const)
                    : ('error' as const),
              summary: r.summary ?? undefined,
            })
            const known = new Set<string>()
            const updated = prev.map(t => {
              if (t.kind !== 'task' || !t.runId) return t
              known.add(t.runId)
              const r = byId.get(t.runId)
              return r ? { ...rowFor(r), id: t.id } : t
            })
            const added = runs.filter(r => !known.has(r.run_id)).map(rowFor)
            return [...updated, ...added]
          })
          break
        }
        case 'token_usage':
          // 事件级 trace：**默认静音**（见 core/debug.ts）——每个 token_usage 事件一行，
          // 与 IPC 日志叠加会把 DevTools 打满。排查：DevTools 里
          // localStorage.setItem('nuphus:debug','1') 后刷新
          if (debugEnabled()) {
            console.log(
              `[TRACE-TOKEN] source=${event.source}, input=${event.input_tokens}, output=${event.output_tokens}, cacheHit=${event.cache_hit_tokens}, eventCount=${eventCountRef.current}, execActive=${h.refs.executionActiveRef.current}, processing=${h.refs.processingRef.current}`,
            )
          }
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
                  genTps: event.gen_tps ?? prev?.genTps,
                  ttftMs: event.ttft_ms ?? prev?.ttftMs,
                }
              })
            // 三分类，不是二分类：`main` 归主指示器；`exec` 归 ctx 弹窗（dispatch / 子任务
            // 执行）；leader / workflow 等其它源都不吸收。此前是「非 main 即 exec」的兜底，
            // 会把 Profile/Workflow 的会话规模也塞进 exec 槽，污染 ctx 弹窗那套整组指标。
            if (event.source === 'main') update(h.setMainTokenUsage)
            else if (event.source === 'exec') update(h.setExecTokenUsage)
            // 本轮元数据的 token 维度：**只累加执行中（本轮活跃）的事件**。
            // token_usage 是「每次 LLM 调用后」的用量，同轮多次调用逐条到达，累加即为
            // 本轮总量（与后端 TurnMeta::add_usage 语义一致）。空闲态残留事件不计入，
            // 不污染下一轮的起点与总量。
            // 且只累加**单次调用用量**源（exec = 主轮/子任务单次消耗，workflow = 工作流
            // 单次消耗）：main 源是「主上下文占用」快照（input=会话规模、cache=哨兵
            // 0xffffffff、tps/ttft=None），把它一并累加会把上下文规模翻倍计进本轮
            // token、把 4294967295（0xffffffff 哨兵）计进 cacheHit，并用 undefined
            // 覆盖掉 exec 刚写进的 tps/ttft —— 三者都让 turnMeta 的读数变成废数。
            const is_per_call_usage = event.source === 'exec' || event.source === 'workflow'
            if (h.refs.executionActiveRef.current && is_per_call_usage) {
              // 累加后的绝对值一并同步给流式气泡：气泡读 msg.meta，拿不到 turnMeta 的闭包值，
              // 必须在同一次 setState 里算出结果再分发（两处各自累加会因批处理读到同一 prev）。
              const nextTokens: TurnMeta = {
                inputTokens:
                  (h.refs.turnMetaTokensRef.current.inputTokens ?? 0) + event.input_tokens,
                outputTokens:
                  (h.refs.turnMetaTokensRef.current.outputTokens ?? 0) + event.output_tokens,
                cacheHitTokens:
                  (h.refs.turnMetaTokensRef.current.cacheHitTokens ?? 0) + event.cache_hit_tokens,
                genTps: event.gen_tps,
                ttftMs: event.ttft_ms,
              }
              h.refs.turnMetaTokensRef.current = nextTokens
              h.setTurnMeta(prev => ({ ...(prev ?? {}), ...nextTokens }))
              syncDraftMeta(nextTokens)
            }
          })()
          break
        case 'refine_prompt': {
          // context_window 优先（新后端），refine_limit 兜底（旧后端兼容）
          const win = event.context_window || event.refine_limit || 0
          const pct =
            event.current_tokens && win ? Math.round((event.current_tokens / win) * 100) : 0
          // 分档与强制线由后端下发：档位决定 UI 形态（仅 large 给 slider），
          // 阈值范围以后端 LARGE_FORCE_MIN/MAX 为唯一权威，前端不硬编码。
          const tier = (event.tier || 'medium') as RefineTier
          // force_threshold 由后端下发（2026-10 修复：此前后端未发该字段，
          // fallback 0.8 导致弹窗恒显 80%）；0.5 = 后端 LARGE_FORCE_DEFAULT，
          // 仅旧后端（无 tier/force_* 字段的远古版本）兼容路径。
          const forceThreshold =
            typeof event.force_threshold === 'number' ? event.force_threshold : 0.5
          const forceMin = typeof event.force_min === 'number' ? event.force_min : 0.5
          const forceMax = typeof event.force_max === 'number' ? event.force_max : 0.8

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
            h.setExecutionStage('running')
            // 清零用水位占位：forced 路径直接执行、不展示询问态，
            // tier/阈值字段随便填即可（UI 不会读它）
            h.setRefineState({
              usagePercent: 0,
              totalLimit: 0,
              tier,
              forceThreshold,
              forceMin,
              forceMax,
            })
            import('../main-window/lib/api').then(({ executeSessionRefine }) => {
              // forced 自动提炼是「主流程自己广播、前端代跑」的机器触发路径：
              // maybe_refine_session 在轮次收尾期（busy 仍被主流程持有）才发
              // RefinePrompt{forced:true}，前端 invoke 可能先于主流程释放 busy 抵达 →
              // 后端 busy CAS 抢占失败，以「当前轮次仍在收尾」拒绝（refine.rs）。
              // 机器触发路径不能静默丢弃（丢弃后本轮不再提炼，直到下次上下文再次超限），
              // 故对「收尾」拒绝做有界重试（收尾窗口毫秒~秒级，2s 上限覆盖）。
              // 用户手动路径（useExecutionUI.handleRefine）不重试：直接给明确错误提示。
              const RETRY_DELAY_MS = 400
              const RETRY_MAX = 5
              const attempt = (n: number) => {
                executeSessionRefine().catch((err: unknown) => {
                  // 已被其它出口复位（refine_failed / 超时 guard / 手动关闭）：放弃重试
                  if (!refineActiveRef.current) return
                  const msg = err instanceof Error ? err.message : String(err ?? '')
                  // 后端防重拒绝（提炼进行中）说明另一端/本端已真实在提炼：
                  // 不 reset（否则 refineActiveRef=false + refining=false 会破坏真实
                  // 提炼的 UI 锁，弹窗/入口提前重新可触发）——等 RefineExecuting /
                  // session_refined / refine_failed 事件权威收敛。
                  if (msg.includes('提炼进行中')) return
                  // 主流程收尾尚未结束（'收尾' 命中 refine.rs busy 抢占拒绝文案）：稍候重试
                  if (msg.includes('收尾') && n < RETRY_MAX) {
                    window.setTimeout(() => attempt(n + 1), RETRY_DELAY_MS)
                    return
                  }
                  // RefineFailed 事件是主复位路径；此处兜底 invoke 层失败（事件
                  // 丢失/旧后端）——静默吞掉会导致 forced 弹窗永久 spinner
                  resetRefineUI()
                })
              }
              attempt(0)
            })
            break
          }

          // 提炼执行中收到新的 refine_prompt（refineActiveRef=true）：忽略——
          // 正在提炼会话，不再弹新提示/累计跳过（否则提炼结束后残留 refineState/
          // pendingRefine 会让弹窗/入口重新可触发，锁释放语义破坏）。
          if (refineActiveRef.current) break

          // 如果已经有 pendingRefine（用户已跳过弹窗）→ 只更新数据，不弹窗
          if (pendingRefineRef.current) {
            const pending = pendingRefineRef.current
            const nextPending = {
              ...pending,
              usagePercent: pct,
              // 必须用本次事件下发的 force_threshold（= 后端此刻真正生效的强制线），
              // 不能沿用 pending 里的旧快照。旧快照是「跳过那一刻」的值，而滑块拖动
              // 只写后端 + 组件本地 forceDraft，从不回写 refineState/pendingRefine，
              // 于是快照会长期偏低甚至偏高。autoRaiseForceThreshold 以
              // `forceThreshold >= minPct` 作为「后端已够高、无需抬升」的判据，
              // 喂旧快照会误判：用户明明调到 80%，快照还停在 0.5，下一轮 usage 把
              // 下限顶到 70% → 误判为「需要抬升」→ 把后端从 0.80 改写成 0.70，
              // 用户的设置被静默降级。此处对齐权威值即可消除该降级。
              forceThreshold,
              skippedTurns: pending.skippedTurns + 1,
            }
            h.setPendingRefine(cur => (cur ? nextPending : null))
            // usagePercent 把滑块下限顶高、而后端 forceThreshold 还停在旧值时
            // 自动抬配置：滑块显示 / 后端配置 / 实际行为三者一致（静默自愈）
            void autoRaiseForceThreshold(nextPending)
            break
          }

          // 正常弹窗（现有逻辑）；tier / 阈值一并带上，供弹窗决定是否给 slider
          const nextRefineState: RefineState = {
            usagePercent: pct,
            totalLimit: win,
            tier,
            forceThreshold,
            forceMin,
            forceMax,
          }
          h.setRefineState(nextRefineState)
          // 同上：低于当前滑块下限的旧 active 配置在此 reconcile 抬升
          void autoRaiseForceThreshold(nextRefineState)
          break
        }
        case 'refine_skipped':
          // 另一端（手机/桌面）跳过了提炼：本端同步关闭弹窗 + 记录跳过（防重复弹窗）。
          // 提炼已开始则跳过无效（refine 正在执行中）。
          if (refineActiveRef.current) break
          h.setRefining(false)
          if (refineStateRef.current) {
            // 整份带走（含 tier 与强制线）：跨端同步同样要保留调节能力
            h.setPendingRefine({ ...refineStateRef.current, skippedTurns: 0 })
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
          h.setExecutionStage('running')
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
          // 提炼完成：RefineGuard 释放执行态（后端归 Idle）→ 本端同步归零
          h.setExecutionStage('idle')
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
          } else {
            // H2 兜底：95s 超时 guard 已删过流式气泡（refineMsgIdRef 清空）——
            // 但后端提炼可能还在跑（本地端点预算 max(配置,900s)+60s，远大于 95s），
            // SessionRefined 到达时**补一条**已完成摘要气泡。不猜时间阈值：
            // 「后台成功、前台无感」比任何 guard 数字之争都更该消除。
            h.setMessages((prev: ChatMessage[]) => [
              ...prev,
              {
                id: crypto.randomUUID(),
                role: 'refine' as const,
                content: event.summary,
                timestamp: Date.now(),
                messageCount: event.message_count,
                sessionId: event.session_id,
                refineStatus: 'completed' as const,
              },
            ])
          }
          // 提炼后会话已压缩：主指示器的旧快照（提炼前用量）作废，置 null 走
          // 「--」未知态——下一轮 TokenUsage 官方读数到达前不显示过期百分比
          // （与换会话时的处置一致，避免「已提炼仍显示 75%」）。
          h.setMainTokenUsage(null)
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
