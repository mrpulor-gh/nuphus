// useInit — 应用初始化与 Toast 通知
import { useState, useCallback, useEffect } from 'react'
import { invoke } from '../core/bridge'
import type { ChatMessage, TimelineEntry } from '../core/types'
import {
  getChatHistory,
  getTools,
  getMemoryStats,
  isLlmConfigured,
  type HistoryMessage,
  type HistoryTraceItem,
} from '../main-window/lib/api'

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
}

/**
 * 历史粒度对齐（对齐手机端 foldHistoryAssistants）：后端 session 每轮
 * push_assistant 一条 → 一次执行的多轮 agent 循环返回多条 → 刷新后一个
 * agent 回复被拆成多个气泡。折叠连续 assistant（中间无 user 间隔）为一条，
 * content 取组内最后非空 content（最终回复）。
 * 与手机端差异：桌面端保留 traceItems（折叠时合并）——气泡执行回溯入口需要
 * 展示对应轮次的完整执行过程（思考/文本/工具调用）。
 */
function foldHistoryAssistants(msgs: HistoryMessage[]): HistoryMessage[] {
  const out: HistoryMessage[] = []
  for (const m of msgs) {
    const prev = out[out.length - 1]
    if (m.role === 'assistant' && prev && prev.role === 'assistant') {
      out[out.length - 1] = {
        ...prev,
        content: m.content && m.content.trim() ? m.content : prev.content,
        traceItems: [...(prev.traceItems || []), ...(m.traceItems || [])],
      }
    } else {
      out.push(m)
    }
  }
  return out
}

/** 后端 HistoryTraceItem → 前端 TimelineEntry（执行回溯面板渲染用） */
function toTimelineEntry(ti: HistoryTraceItem): TimelineEntry {
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
  const { setMessages, setModelName, setSessionId, messagesRestoredRef } = deps

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
  const showToast = useCallback((message: string, type: Toast['type'] = 'info') => {
    const phaseMap: Record<string, string> = {
      info: 'info',
      error: 'error',
      warning: 'warning',
      success: 'success',
    }
    invoke('hud_update', { text: message, phase: phaseMap[type] || 'info' }).catch(e =>
      console.warn('[Toast] hud_update failed:', e),
    )
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
          // 折叠连续 assistant（一次执行的多轮循环）为一条，只显示最终回复
          // （对齐手机端 foldHistoryAssistants）；桌面端保留 traceItems 供气泡
          // 执行回溯入口展示该轮完整执行过程。
          const folded = foldHistoryAssistants(history)
          setMessages(
            folded.map(h => ({
              id: crypto.randomUUID(),
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