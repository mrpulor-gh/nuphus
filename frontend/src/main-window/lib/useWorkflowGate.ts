// useWorkflowGate.ts — 全局执行闸门查询 hook（WorkflowPage / CanvasPage 共用）
//
// 大王铁律：无并行机制、禁止并行、系统操作更禁止并行。
// 任意执行态（Agent 跑代码 / workflow 执行中）禁止启动任何工作流、禁止进入画布。
//
// 后端权威源 wf_gate_status 合成 active_run + Agent busy：
//   locked=true 时 UI 应禁用「运行 / 画布 / 录制」入口并提示
//   「当前有任务执行中，暂不可用！」。
//
// 轮询 1.5s（同 ChatInputBar 先例，页面挂载期间持续感知执行开始/结束；
// 查询失败保持上一状态，不因瞬时错误误解锁）。

import { useCallback, useEffect, useRef, useState } from 'react'
import { wfGateStatus, type WfGateStatus } from './api'

const IDLE: WfGateStatus = { locked: false, reason: 'idle' }

export interface WorkflowGate {
  status: WfGateStatus
  locked: boolean
  /** 'workflow' | 'agent' | 'idle' */
  reason: WfGateStatus['reason']
  /** 立即重查（点击入口前先同步一次，缩小轮询窗口内的竞态） */
  refresh: () => Promise<WfGateStatus>
}

export function useWorkflowGate(pollMs = 1500): WorkflowGate {
  const [status, setStatus] = useState<WfGateStatus>(IDLE)
  const statusRef = useRef<WfGateStatus>(IDLE)
  statusRef.current = status

  const refresh = useCallback(async (): Promise<WfGateStatus> => {
    try {
      const s = (await wfGateStatus()) ?? IDLE
      if (statusRef.current.locked !== s.locked || statusRef.current.reason !== s.reason) {
        statusRef.current = s
        setStatus(s)
      }
      return s
    } catch {
      // 查询瞬时失败：保持上一状态（错误方向宁可锁着，由后端闸门兜底放行/拦截）
      return statusRef.current
    }
  }, [])

  useEffect(() => {
    void refresh()
    // pollMs <= 0：仅挂载查一次，不启动轮询（常驻组件按需开启用，如 ChatInputBar
    // 仅在 workflow 模式需要感知执行态；避免非 workflow 场景空转 IPC）
    if (pollMs <= 0) return
    const timer = setInterval(() => void refresh(), pollMs)
    return () => clearInterval(timer)
  }, [refresh, pollMs])

  return { status, locked: status.locked, reason: status.reason, refresh }
}
