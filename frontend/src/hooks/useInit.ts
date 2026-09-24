// useInit — 应用初始化与 Toast 通知
import { useState, useCallback, useEffect } from 'react'
import { invoke, listen } from '../core/bridge'
import type { ChatMessage, TimelineEntry } from '../core/types'
import {
  getChatHistory,
  getCurrentMode,
  getTools,
  getMemoryStats,
  isLlmConfigured,
  type HistoryMessage,
  type HistoryTraceItem,
} from '../main-window/lib/api'
import { showAppFeedback } from '../ui/islandChannel'
import { foldAssistantHistory } from '../core/progressMessages'

type InitStatus = 'pending' | 'loading' | 'done' | 'error'

export interface Toast {
  id: string
  message: string
  type: 'info' | 'error' | 'warning' | 'success'
}

/** 初始化时使用的工具依赖 */
export interface InitDeps {
  setMessages: (msgs: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => void
  setModelName: (name: string) => void
  setSessionId: (id: string) => void
  messagesRestoredRef: React.MutableRefObject<boolean>
  /** 启动时同步后端权威 mode（镜像恢复结果 leader/workflow/custom） */
  setMode: (mode: string) => void
}

/**
 * 已标记的过程说明逐条保留；旧数据继续折叠连续 assistant 轮次。
 * 空工具轮次不生成气泡，traceItems 合并供执行回溯使用。
 */
/** 导出供 useSession.reloadChatFromBackend 复用（Session Shelf 切换后刷新） */
export function foldHistoryAssistants(msgs: HistoryMessage[]): HistoryMessage[] {
  return foldAssistantHistory(msgs)
}

/** 后端 HistoryTraceItem → 前端 TimelineEntry（执行回溯面板渲染用） */
/** 导出供 useSession.reloadChatFromBackend 复用 */
export function toTimelineEntry(ti: HistoryTraceItem): TimelineEntry {
  if (ti.kind === 'tool') {
    let params: unknown
    try {
      params = ti.params ? JSON.parse(ti.params) : undefined
    } catch {
      params = ti.params
    }
    return {
      id: ti.call_id || crypto.randomUUID(),
      kind: 'tool_call',
      toolName: ti.name || '',
      status: ti.status === 'ok' ? 'success' : ti.status === 'fail' ? 'error' : 'running',
      params,
      output: '',
      durationMs: 0,
    }
  }
  return {
    id: crypto.randomUUID(),
    kind: ti.kind === 'thinking' ? 'thinking' : 'text',
    text: ti.text || '',
  }
}

export function useInit(deps: InitDeps) {
  const { setMessages, setModelName, setSessionId, messagesRestoredRef, setMode } = deps

  // ── App lifecycle state ──
  const [appState, setAppState] = useState<'loading' | 'ready' | 'error'>('loading')
  const [initError, setInitError] = useState<{
    kind: string
    message: string
    detail?: string
  } | null>(null)

  const [initItems, setInitItems] = useState<{ key: string; label: string; status: InitStatus }[]>([
    { key: 'memory', label: 'Memory', status: 'pending' },
    { key: 'tools', label: 'Tools', status: 'pending' },
    { key: 'model', label: 'Model', status: 'pending' },
    { key: 'ocr', label: 'OCR', status: 'pending' },
  ])

  const [fadeOut, setFadeOut] = useState(false)

  const [startupStats, setStartupStats] = useState<{ tools: number; memories: number }>({
    tools: 0,
    memories: 0,
  })

  // ── Toast ──
  // 轻反馈的唯一入口：应用窗口在前台 → 应用内 island；不在前台/最小化 → HUD
  // 独立窗口（分流与队列编排都在 ui/islandChannel.ts，调用点无需感知通道差异）
  const showToast = useCallback((message: string, type: Toast['type'] = 'info') => {
    showAppFeedback(message, type)
  }, [])

  // ── Helpers ──
  function updateInitItem(key: string, status: InitStatus) {
    setInitItems(prev => prev.map(i => (i.key === key ? { ...i, status } : i)))
  }

  const runInitialization = useCallback(async () => {
    try {
      setAppState('loading')
      setInitItems(prev => prev.map(i => ({ ...i, status: 'pending' as InitStatus })))

      // 1. Tools: check LLM config
      invoke('splash_status_update', { text: 'Checking configuration…' }).catch(() => {})
      updateInitItem('tools', 'loading')
      try {
        const configured = await isLlmConfigured()
        updateInitItem('tools', configured ? 'done' : 'error')
      } catch {
        updateInitItem('tools', 'error')
      }

      // 2. Memory: restore chat history + context limit
      invoke('splash_status_update', { text: 'Restoring memory…' }).catch(() => {})
      updateInitItem('memory', 'loading')
      try {
        const history = await getChatHistory()
        if (history && history.length > 0) {
          // 保留过程说明，并兼容旧版连续 assistant 历史的折叠行为。
          const folded = foldHistoryAssistants(history)
          setMessages(
            folded.map(h => ({
              id: h.message_id ?? crypto.randomUUID(),
              kind: h.kind,
              message_id: h.message_id,
              role: h.role as ChatMessage['role'],
              content: h.content,
              images: h.images && h.images.length > 0 ? h.images : undefined,
              audio: h.audio && h.audio.length > 0 ? h.audio : undefined,
              // 历史 refine 消息（提炼摘要）视为已完成——ChatPanel 据此渲染 SessionDivider
              ...(h.role === 'refine' ? { refineStatus: 'completed' as const } : {}),
              // 历史消息时间由后端透传（session Message 创建时间）；旧数据缺失时兜底当前时间
              timestamp: h.timestamp ?? Date.now(),
              // 执行过程（思考/文本/工具调用）→ 气泡执行回溯入口数据
              ...(h.traceItems && h.traceItems.length > 0
                ? { traceItems: h.traceItems.map(toTimelineEntry) }
                : {}),
            })),
          )
          messagesRestoredRef.current = true
        }
      } catch {}
      updateInitItem('memory', 'done')

      // 2.5 Mode: 启动同步后端权威 mode（镜像恢复 leader/workflow/custom）——
      // 输入框 mode chip 与后端 current_mode 一致；会话视图由 get_chat_history
      // 按 current_mode 返回（有历史则显示，无历史则欢迎页，不自动进入会话）
      try {
        const m = await getCurrentMode()
        if (m) setMode(m)
      } catch {}

      // 3. Model: preload Candle embed model
      invoke('splash_status_update', { text: 'Loading model…' }).catch(() => {})
      updateInitItem('model', 'loading')
      try {
        await invoke('preload_model')
      } catch {}
      updateInitItem('model', 'done')

      // 4. OCR: preload OCR model
      invoke('splash_status_update', { text: 'Loading tools…' }).catch(() => {})
      updateInitItem('ocr', 'loading')
      try {
        await invoke('preload_ocr')
      } catch {}
      updateInitItem('ocr', 'done')

      // Stats (non-blocking)
      try {
        const [tools, stats] = await Promise.all([
          getTools().catch(() => null),
          getMemoryStats().catch(() => null),
        ])
        setStartupStats({
          tools: tools?.length ?? 0,
          memories: stats?.total_entries ?? 0,
        })
      } catch {}
    } catch (e: any) {
      setInitError({ kind: 'init_failed', message: e.message || String(e) })
      setAppState('error')
    }
  }, [setMessages, messagesRestoredRef])

  // Initialize on mount — splash window shown independently by Tauri
  useEffect(() => {
    runInitialization()
      .then(() => {
        setAppState('ready')
        setFadeOut(true)
        setTimeout(() => setFadeOut(false), 300)
      })
      .catch(() => {
        // init error already handled in runInitialization (setAppState('error'))
      })
      .finally(() => {
        // Always close splash + show main (error screen or ready UI)
        invoke('finish_startup').catch(() => {})
      })
  }, [runInitialization])

  // splash「后台下载」跳过：模型仍在后台下载，主界面立即可用。
  // （splash_skip_download 关闭 splash 后广播此事件；本监听让 appState 立即转 ready，
  //  不再等待 preload_model / preload_ocr 阻塞完成）
  useEffect(() => {
    let unl: (() => void) | undefined
    let mounted = true
    ;(async () => {
      try {
        const u = await listen('splash:skipped', () => {
          if (!mounted) return
          setAppState('ready')
          setFadeOut(true)
          setTimeout(() => setFadeOut(false), 300)
        })
        if (mounted) unl = u
      } catch {
        // 事件不可用（异常环境）：退化为等待模型下载完成，行为同旧版
      }
    })()
    return () => {
      mounted = false
      unl?.()
    }
  }, [setAppState])

  const refreshModelInfo = useCallback(async () => {
    try {
      const { getCurrentConfig } = await import('../main-window/lib/api')
      const cfg = await getCurrentConfig()
      if (cfg?.model) setModelName(cfg.model)
    } catch {}
  }, [setModelName])

  return {
    appState,
    initError,
    initItems,
    fadeOut,
    startupStats,
    setAppState,
    setInitError,
    setInitItems,
    showToast,
    runInitialization,
    refreshModelInfo,
  }
}
