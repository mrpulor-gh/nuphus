// useExecutionState — 前端执行态的**唯一来源**。
//
// 收敛前执行态有 5 个互不相通的来源，UI 各处各自订阅、甚至做 OR 派生：
//   1) 前端 isProcessing（事件驱动的本地布尔）
//   2) 前端 completed / execPhase（思考条文案）
//   3) 后端 is_busy（终止按钮，1.5s 轮询）
//   4) 后端 can_switch（会话 rail 锁，5s 轮询）
//   5) rail 的 hardLocked = !canSwitch || locked（把 1) 与 4) OR 起来）
//
// 现在只有一个来源：**后端 ExecutionStage**（`SignalState::execution_stage`），
// 前端经两条通道取它——
//   · 拉：`get_execution_state`（挂载即校 + 执行期轮询）
//   · 推：`nuphus-event`（execution_started / execution_completed / execution_error）
// 各 UI 只做「对同一个 stage 值的谓词派生」，不再持有自己的执行态副本。
//
// 阶段语义（与后端 nuphus::state::ExecutionStage 一一对应）：
//   idle       无执行占用
//   running    主循环在迭代中 —— 追加指令可被注入；气泡光标 / 思考条呼吸 / 终止按钮
//   finalizing 主循环已退出、后端在收尾（记忆落盘 / 自动提炼）—— 提交会被拒收退回输入框
//
// 自愈：轮询读到后端 idle 而本地仍非 idle 时复位本地态（原来只在「下次发送」时顺带
// 自愈，收尾窗口内的残留会让界面永久停在执行中）。

import { useCallback, useEffect, useRef, useState } from 'react'
import { getExecutionState } from '../main-window/lib/api'

/** 执行阶段（与后端 stage 字符串同名同义） */
export type ExecutionStage = 'idle' | 'running' | 'finalizing'

const KNOWN_STAGES: readonly ExecutionStage[] = ['idle', 'running', 'finalizing']

/** 未知/缺失值一律按 idle 处理（旧后端 / 字段缺失时不臆造执行态） */
export function normalizeExecutionStage(value: unknown): ExecutionStage {
  return KNOWN_STAGES.includes(value as ExecutionStage) ? (value as ExecutionStage) : 'idle'
}

export interface ExecutionStateOptions {
  /**
   * 是否存在「流式目标」（本轮气泡仍在流式 / 已乐观置位但后端尚未开始）。
   *
   * 用途：发送时前端会乐观置 running，而后端要在通过配置/去重校验后才进入主循环，
   * 这个空窗内轮询可能读到 idle——此时**不能**把本地态打回空闲（会把刚发出的执行
   * 判成新回合）。存在流式目标即视为「执行正在建立」，忽略这一轮 idle。
   */
  hasStreamingTarget?: () => boolean
}

export interface ExecutionState {
  /** 唯一权威值 */
  stage: ExecutionStage
  /** 派生：后端占用中（running ∨ finalizing）——终止按钮 / 会话与 mode 锁定 */
  busy: boolean
  /** 派生：主循环在迭代中（气泡光标 / 思考条呼吸） */
  running: boolean
  /** 派生：收尾中（主循环已退出、后端在落盘/提炼；提交会被拒收） */
  finalizing: boolean
  /** 事件通道：本地置位（乐观发送 / 事件到达）。真值以后端轮询收敛。 */
  setStage: (stage: ExecutionStage) => void
  /** 拉通道：读后端权威态并落本地，返回落定后的阶段 */
  refresh: () => Promise<ExecutionStage>
}

/** 常规轮询周期（idle / running）：与后端权威态对齐的上限延迟 */
const POLL_MS = 1500
/** Finalizing 期快跟周期：收尾通常秒级，快跟以便及时解锁 UI / 提示可重发 */
const FINALIZING_POLL_MS = 300

export function useExecutionState(options: ExecutionStateOptions = {}): ExecutionState {
  const [stage, setStageState] = useState<ExecutionStage>('idle')
  const stageRef = useRef<ExecutionStage>(stage)
  stageRef.current = stage
  const hasStreamingTargetRef = useRef(options.hasStreamingTarget)
  hasStreamingTargetRef.current = options.hasStreamingTarget

  const setStage = useCallback((next: ExecutionStage) => {
    stageRef.current = next
    setStageState(next)
  }, [])

  const refresh = useCallback(async (): Promise<ExecutionStage> => {
    try {
      const snapshot = await getExecutionState()
      // 无回执（IPC 未就绪 / 后端不可达）：保持当前态，不按 idle 处理——
      // 把「读不到」当「空闲」会误清正在进行的执行态。
      if (!snapshot || typeof snapshot.stage !== 'string') return stageRef.current
      const next = normalizeExecutionStage(snapshot.stage)
      // 空窗保护：后端尚未进入主循环（空闲）但本端已有流式目标 → 保持本地态，
      // 让后端下轮轮询再收敛；否则刚发出的执行会被误判为空闲。
      if (next === 'idle' && stageRef.current !== 'idle' && hasStreamingTargetRef.current?.()) {
        return stageRef.current
      }
      if (next !== stageRef.current) setStage(next)
      return next
    } catch {
      // 后端不可达：保持当前态等待下次轮询/事件（不猜测、不无依据复位）
      return stageRef.current
    }
  }, [setStage])

  // 挂载即校：界面刷新 / HMR 后恢复真实执行态（本地 state 已丢失）
  useEffect(() => {
    void refresh()
  }, [refresh])

  // ── 常驻轮询（**不得**在 idle 期停摆）──
  // 执行态是后端唯一权威，前端两条通道：事件（即时）+ 轮询（兜底）。轮询承担两件事：
  //   1) 发现「非本窗口发起」的执行：手机端 / 画布 / 门铃自动唤醒 / 定时任务都会在后端
  //      置位 stage，但不经过本窗口的事件路径——若空闲期不轮询，本端永远停在 idle，
  //      终止按钮不会出现（回归实测 2026-09：终止按钮消失、只剩发送按钮，用户无法终止）；
  //   2) 常驻自愈：后端回到 idle 而本端仍残留执行态时复位（原实现只在「下次发送」时自愈）。
  // 旧 ChatInputBar 的 is_busy 轮询同样是「挂载即查 + 事件补查」双路径，语义在此统一保留。
  useEffect(() => {
    const period = stage === 'finalizing' ? FINALIZING_POLL_MS : POLL_MS
    const timer = window.setInterval(() => {
      void refresh()
    }, period)
    return () => window.clearInterval(timer)
  }, [stage, refresh])

  return {
    stage,
    busy: stage !== 'idle',
    running: stage === 'running',
    finalizing: stage === 'finalizing',
    setStage,
    refresh,
  }
}
