/**
 * 收尾期（ExecutionStage::Finalizing）行为契约 —— 大王 2026-09 明确要求。
 *
 * 背景（原缺陷链）：主循环已退出、后端仍在收尾（记忆落盘 / 自动提炼）时提交消息，
 * 原来的实现会**静默入队**为追加指令，但此时已无消费方（队列只在主循环迭代边界
 * drain）→ 消息永不执行、残留队列锁死会话切换、用户按提示重发还会被判
 * `is_duplicate_of_last` 丢弃（整轮有效去重）。
 *
 * 现契约：收尾期提交必须**显式拒收**，且
 *   ① 原文原样退回输入框，不产生「不会被执行」的气泡；
 *   ② 用户随即重发同一文本必须正常进入新回合（不被去重吞掉）；
 *   ③ 后端回到 idle 时前端残留执行态被复位（常驻自愈）。
 *
 * 这里断言的都是可观察结果：输入框 value、onSend 的实参、messages 内容、hook 的 stage。
 */
import { createRef } from 'react'
import { act, fireEvent, render, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatInputBar } from '../main-window/chat/ChatInputBar'
import { ChatPanel } from '../main-window/chat/ChatPanel'
import { ThemeProvider } from '../hooks/useTheme'
import { useSession } from '../hooks/useSession'
import { useExecutionState } from '../hooks/useExecutionState'

const FINALIZING_MSG = '收尾期的指令'

// api wrapper 全量 stub：ChatPanel / useSession / useInit 挂载期会拉配置、历史、上下文限额等读数。
vi.mock('../main-window/lib/api', async importOriginal => {
  const actual = await importOriginal<Record<string, unknown>>()
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(actual)) {
    out[key] = typeof value === 'function' ? vi.fn(async () => undefined) : value
  }
  return out
})

vi.mock('../core/bridge', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
  invoke: vi.fn(async () => undefined),
}))

// jsdom 无 DOMMatrix/canvas：PDF 渲染器在 ChatPanel 子链路里被 import，stub 掉
vi.mock('pdfjs-dist', () => ({
  GlobalWorkerOptions: {},
  getDocument: vi.fn(),
  version: 'stub',
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onDragDropEvent: vi.fn(() => Promise.resolve(() => {})),
    setFocus: vi.fn(),
  }),
}))
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (p: string) => p,
  invoke: vi.fn(async () => undefined),
}))

// jsdom 未实现：ChatPanel 的滚动到底 / 语音等链路会触达
Object.defineProperty(window.Element.prototype, 'scrollTo', { value: () => {}, writable: true })

import * as api from '../main-window/lib/api'

/** 后端执行态快照（stage 为权威，其余为派生字段） */
function snapshot(stage: 'idle' | 'running' | 'finalizing') {
  return { stage, busy: stage !== 'idle', append_accepting: stage === 'running' }
}

describe('收尾期（Finalizing）拒绝追加', () => {
  beforeEach(() => {
    vi.mocked(api.isLlmConfigured).mockResolvedValue(true as never)
    vi.mocked(api.getExecutionState).mockResolvedValue(snapshot('idle') as never)
    vi.mocked(api.getChatHistory).mockResolvedValue([] as never)
    vi.mocked(api.getTools).mockResolvedValue([] as never)
    vi.mocked(api.getMemoryStats).mockResolvedValue({ total_entries: 0 } as never)
    vi.mocked(api.getCurrentConfig).mockResolvedValue(null as never)
    vi.mocked(api.getCurrentMode).mockResolvedValue('leader' as never)
  })

  it('① 输入框：消息被拒 → 原文回填、无气泡、不开启新执行', async () => {
    // 后端返回拒收（未受理：未入队、未记去重基准）
    const onSend = vi.fn(async () => ({
      ok: false,
      rejected: 'finalizing',
      message: '正在收尾，请稍后重发（内容已退回输入框）',
    }))
    render(
      <ThemeProvider>
        <ChatPanel
          messages={[]}
          executionStage="finalizing"
          onSend={onSend}
          startupStats={{ tools: 0, memories: 0 }}
          pendingRefine={null}
          setPendingRefine={vi.fn()}
        />
      </ThemeProvider>,
    )
    const ta = document.querySelector('.chat-input') as HTMLTextAreaElement
    expect(ta).toBeTruthy()

    // 输入并发送
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLTextAreaElement.prototype,
      'value',
    )?.set
    setter?.call(ta, FINALIZING_MSG)
    fireEvent.input(ta)
    fireEvent.keyDown(ta, { key: 'Enter' })

    // 发送发生了（原文一字不改地交给后端）
    await waitFor(() => expect(onSend).toHaveBeenCalledWith(FINALIZING_MSG, undefined, undefined))
    // 且被拒后原文原样回到输入框（不是被清空 / 不是被丢弃）
    await waitFor(() => expect(ta.value).toBe(FINALIZING_MSG))
    expect(onSend).toHaveBeenCalledTimes(1)
  })

  it('② 重发同一文本不被吞：两次都提交，第二次进入新回合', async () => {
    const { result } = renderHook(() => useSession())
    await act(async () => {
      await Promise.resolve()
    })

    // 上一轮主循环已退出、后端仍在收尾 → 提交前预检即判收尾
    vi.mocked(api.getExecutionState).mockResolvedValue(snapshot('finalizing') as never)
    let first: Awaited<ReturnType<typeof result.current.handleSend>> | undefined
    await act(async () => {
      first = await result.current.handleSend(FINALIZING_MSG)
    })
    expect(first?.rejected).toBe('finalizing')
    expect(vi.mocked(api.processInput)).not.toHaveBeenCalled() // 未受理：连后端都没打
    expect(result.current.messages).toHaveLength(0) // 不产生「不会被执行」的气泡

    // 收尾结束（阶段回到 idle）→ 同一文本重发：必须进入新回合
    vi.mocked(api.getExecutionState).mockResolvedValue(snapshot('idle') as never)
    vi.mocked(api.processInput).mockResolvedValue({
      success: true,
      message: '好的',
      steps_count: 0,
    } as never)
    let second: Awaited<ReturnType<typeof result.current.handleSend>> | undefined
    await act(async () => {
      second = await result.current.handleSend(FINALIZING_MSG)
    })

    expect(second?.ok).toBe(true)
    expect(vi.mocked(api.processInput)).toHaveBeenCalledTimes(1)
    // 同一个文本被真正送进新回合（未被任何去重 / 残留守卫吞掉）
    expect(vi.mocked(api.processInput).mock.calls[0][0]).toBe(FINALIZING_MSG)
    await waitFor(() =>
      expect(
        result.current.messages.some(m => m.role === 'user' && m.content === FINALIZING_MSG),
      ).toBe(true),
    )
  })

  it('②b 终止按钮：执行中（running）必须可见，收尾/空闲不显示（防「无法终止」回归）', async () => {
    const renderBar = (stage: 'idle' | 'running' | 'finalizing') =>
      render(
        <ThemeProvider>
          <ChatInputBar
            input=""
            onInputChange={vi.fn()}
            onInputKeyDown={vi.fn()}
            textareaRef={createRef<HTMLTextAreaElement>()}
            imageInputRef={createRef<HTMLInputElement>()}
            executionStage={stage}
            pauseState={null}
            refineState={null}
            tokenUsage={null}
            mainTokenUsage={null}
            execTokenUsage={null}
            totalDurationMs={undefined}
            totalCalls={undefined}
            mood="idle"
            contextLimit={undefined}
            security={null}
            mode="leader"
            onSetMode={vi.fn()}
            modelLabel="DeepSeek"
            modelName="deepseek-chat"
            effort={null}
            supportedEfforts={[]}
            onEffortChange={vi.fn()}
            onModelSwitch={vi.fn()}
            onSend={vi.fn()}
            onFileSelect={vi.fn()}
            onImageAttach={vi.fn()}
            projectDir=""
            hints={['输入框提示']}
            hintIndex={0}
            hintFade={false}
            pendingReferences={[]}
            pendingImages={[]}
            pendingFiles={[]}
            onRemoveReference={vi.fn()}
            onRemoveImage={vi.fn()}
            onRemoveFile={vi.fn()}
          />
        </ThemeProvider>,
      )

    // 执行中：终止按钮在（唯一执行态 running 派生）
    const running = renderBar('running')
    expect(running.container.querySelector('.interrupt')).toBeTruthy()
    running.unmount()

    // 空闲：无终止按钮
    const idle = renderBar('idle')
    expect(idle.container.querySelector('.interrupt')).toBeNull()
    idle.unmount()

    // 收尾期：主循环已退出、收尾不可中断 → 不显示终止（提交会被拒收退回输入框）
    const finalizing = renderBar('finalizing')
    expect(finalizing.container.querySelector('.interrupt')).toBeNull()
    finalizing.unmount()
  })

  it('③ 常驻自愈：后端回到 idle 时残留执行态被复位', async () => {
    vi.mocked(api.getExecutionState).mockResolvedValue(snapshot('running') as never)
    const { result } = renderHook(() => useExecutionState())
    await act(async () => {
      await Promise.resolve()
    })
    await waitFor(() => expect(result.current.stage).toBe('running'))

    // 后端已空闲（例如收尾结束、或界面刷新后残留）→ 轮询自愈回 idle
    vi.mocked(api.getExecutionState).mockResolvedValue(snapshot('idle') as never)
    await act(async () => {
      await result.current.refresh()
    })
    expect(result.current.stage).toBe('idle')
    expect(result.current.busy).toBe(false)
  })

  it('③b 保护：后端尚未受理（idle）但本端已有流式目标时，不自愈成空闲', async () => {
    vi.mocked(api.getExecutionState).mockResolvedValue(snapshot('idle') as never)
    const { result } = renderHook(() => useExecutionState({ hasStreamingTarget: () => true }))
    await act(async () => {
      result.current.setStage('running')
    })
    await act(async () => {
      const next = await result.current.refresh()
      expect(next).toBe('running')
    })
    expect(result.current.stage).toBe('running')
  })
})
