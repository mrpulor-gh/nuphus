import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'

/**
 * 临时伙伴（Side Chat）面板契约。
 *
 * 分两块：
 * ① 渲染行为（jsdom 真渲染）：打开 / 发送（含发送中防并发）/ 清空 / 关闭 /
 *    项目选择（内置 Explore 项 + 书签表）。
 * ② 接线契约（源码断言）：入口按钮在输入栏、面板挂在 .chat-panel、后端三命令已注册。
 *    之所以按源码断言：ChatPanel 有上百个 props，整根渲染要把整套 bridge / locale /
 *    gate 替身一遍，而这里要钉的是结构事实，不是渲染结果。
 */

const {
  sideChatSend,
  sideChatAppend,
  sideChatState,
  sideChatPrepare,
  sideChatCancel,
  sideChatReset,
  sideChatClose,
  sideChatSave,
  sideChatHistoryList,
  sideChatHistoryDelete,
  sideChatRemoveAppendItem,
  listModels,
  getProjectBookmarks,
  getProjectDir,
  hudUpdate,
  listenMock,
  unlistenMock,
  invokeMock,
} = vi.hoisted(() => ({
  sideChatSend: vi.fn(),
  sideChatAppend: vi.fn(),
  sideChatState: vi.fn(),
  // 面板挂载必调 prepare（打开即建壳）：默认 resolved，面板里的 .catch 才可用。
  sideChatPrepare: vi.fn(() => Promise.resolve({ created: true })),
  sideChatCancel: vi.fn(),
  sideChatReset: vi.fn(),
  sideChatClose: vi.fn(),
  sideChatSave: vi.fn(),
  sideChatHistoryList: vi.fn(),
  sideChatHistoryDelete: vi.fn(),
  sideChatRemoveAppendItem: vi.fn(),
  listModels: vi.fn(),
  getProjectBookmarks: vi.fn(),
  getProjectDir: vi.fn(),
  hudUpdate: vi.fn(),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
  invokeMock: vi.fn(),
}))
vi.mock('../lib/api', () => ({
  sideChatSend,
  sideChatAppend,
  sideChatState,
  sideChatPrepare,
  sideChatCancel,
  sideChatReset,
  sideChatClose,
  sideChatSave,
  sideChatHistoryList,
  sideChatHistoryDelete,
  sideChatRemoveAppendItem,
  listModels,
  getProjectBookmarks,
  getProjectDir,
  hudUpdate,
}))

// 实时过程走 Tauri 事件频道：listen 的 handler 由本测试捕获后手动推事件
// （退订函数可断言，组件卸载必须退订，否则监听泄漏）
vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}))
// skill 选择经动态 import('@tauri-apps/api/core') 的 invoke('skill_list')
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

import { EXPLORE_PROJECT, SideChatPanel } from './SideChatPanel'

/** 组件经 listen 注册的 handler（每个挂载周期攒一份，afterEach 清空） */
type SideChatHandler = (payload: unknown) => void
let liveHandlers: SideChatHandler[] = []

/** 推一条 'side-chat-event'（Tauri Event 形状：数据在 .payload） */
function pushLiveEvent(sessionKey: string, event: Record<string, unknown>) {
  liveHandlers.forEach(h => h({ payload: { session_key: sessionKey, event } }))
}

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}
const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')
const inputBarSrc = read('./ChatInputBar.tsx')
const chatPanelSrc = read('./ChatPanel.tsx')
const mainRs = read('../../../../src-tauri/src/main.rs')
const css = read('./side-chat.css')

const SESSION_KEY = 'test-side-chat-key'

function renderPanel(
  overrides: {
    modelName?: string
    onClose?: () => void
    onMinimize?: () => void
    onSendToMain?: (text: string) => void
    onSubmitToMain?: (text: string) => void
    minimized?: boolean
    mainExecTitle?: string | null
  } = {},
) {
  const onClose = overrides.onClose ?? vi.fn()
  const onMinimize = overrides.onMinimize ?? vi.fn()
  const onSendToMain = overrides.onSendToMain ?? vi.fn()
  const onSubmitToMain = overrides.onSubmitToMain ?? vi.fn()
  const result = render(
    <SideChatPanel
      sessionKey={SESSION_KEY}
      modelName={overrides.modelName ?? 'qwen3-max'}
      onClose={onClose}
      onMinimize={onMinimize}
      minimized={overrides.minimized}
      onSendToMain={onSendToMain}
      onSubmitToMain={onSubmitToMain}
      mainExecTitle={overrides.mainExecTitle}
    />,
  )
  return { ...result, onClose, onMinimize, onSendToMain, onSubmitToMain }
}

/** 打开「历史」浮层（保存 / 清空 / 归档列表三合一，hover 触发） */
function openHistoryMenu() {
  fireEvent.mouseEnter(document.querySelector('.scp-history-wrap') as Element)
}

describe('SideChatPanel 渲染行为', () => {
  beforeEach(() => {
    // 模型列表默认空（下拉只剩「跟随主会话」），单测按需覆盖
    listModels.mockResolvedValue([])
    // 项目选择器：默认无已设项目、主会话也未设目录（选择器退化为仅「跟随」）
    getProjectBookmarks.mockResolvedValue([])
    getProjectDir.mockResolvedValue({ path: '', name: '', tag: 'default' })
    // 历史归档默认空（开户不炸）；个别用例按需覆盖
    sideChatHistoryList.mockResolvedValue([])
    // 'side-chat-event' 监听：捕获 handler，退订函数可断言
    // 状态查询默认：无壳 / 无在途 / 无完成轮次（各用例按需覆盖）
    sideChatState.mockResolvedValue({ exists: false, running: false, last_turn: null })
    liveHandlers = []
    listenMock.mockReset()
    unlistenMock.mockReset()
    listenMock.mockImplementation((_event: string, handler: SideChatHandler) => {
      liveHandlers.push(handler)
      return Promise.resolve(unlistenMock)
    })
    invokeMock.mockReset()
  })

  afterEach(() => {
    // 刷新续命写入 localStorage：必须逐用例清，否则消息跨用例串流
    localStorage.clear()
    cleanup()
    liveHandlers = []
    sideChatSend.mockReset()
    sideChatAppend.mockReset()
    sideChatRemoveAppendItem.mockReset()
    sideChatState.mockReset()
    sideChatCancel.mockReset()
    sideChatReset.mockReset()
    sideChatClose.mockReset()
    sideChatSave.mockReset()
    listModels.mockReset()
    getProjectBookmarks.mockReset()
    getProjectDir.mockReset()
    hudUpdate.mockReset()
    listenMock.mockReset()
    unlistenMock.mockReset()
    invokeMock.mockReset()
  })

  it('打开即渲染：标题 / 输入框 / 空态结构化说明（设置入口已移除）', () => {
    renderPanel()
    // 顶栏 aria-label + 空态标题都可能是「临时伙伴」，只断言至少一处
    expect(screen.getAllByText('临时伙伴').length).toBeGreaterThan(0)
    expect(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…')).toBeTruthy()
    // 空态分层：一句定位 + 三条要点（不再是单行灰字）
    expect(screen.getByText('创意方案 · 探索调研 · 临时任务 · 核对审计')).toBeTruthy()
    expect(screen.getByText('系统操作不可用；关闭或切换会话即清空')).toBeTruthy()
    expect(screen.getByText('不写记忆、不进上下文、不参与派发')).toBeTruthy()
    // 权限 / 系统类设置整块移除：面板不再提供工具权限开关。
    // （唯一保留的开关是「结论自动交付」——并行协作契约，且它不是 role=switch）
    expect(screen.queryByRole('button', { name: '临时伙伴设置' })).toBeNull()
    expect(screen.queryAllByRole('switch')).toHaveLength(0)
  })

  it('发送：带当前权限与模型调后端，用户/助手气泡都落盘到本地', async () => {
    sideChatSend.mockResolvedValue({ reply: '这是临时回复' })
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '帮我看看这个' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))

    await waitFor(() => expect(screen.getByText('这是临时回复')).toBeTruthy())
    expect(sideChatSend).toHaveBeenCalledWith(
      SESSION_KEY,
      '帮我看看这个',
      true,
      true,
      'qwen3-max',
      undefined,
      null,
      // provider：跟随主会话（model 为空）时不传
      undefined,
      // projectDir：未选项目 → 跟随主会话
      null,
      // turnId：前端生成，形态不约束（只要求存在，用于刷新判重）
      expect.any(String),
    )
    expect(screen.getByText('帮我看看这个')).toBeTruthy()
    // 输入框清空、恢复可发送
    expect(
      (screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…') as HTMLTextAreaElement)
        .value,
    ).toBe('')
    expect((screen.getByRole('button', { name: '发送' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('执行中追加：入队本轮而非新起一轮，收到 drain 回执后转正式', async () => {
    let finishSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise<{ reply: string }>(resolve => {
        finishSend = resolve
      }),
    )
    sideChatAppend.mockResolvedValue({ queued: true, duplicate: false, pending: 1 })
    renderPanel()
    const box = screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…')
    fireEvent.change(box, { target: { value: '先看看目录' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))

    // 在途：发送键让位，输入第二条后出现「追加到本轮」
    await waitFor(() => expect(screen.queryByRole('button', { name: '发送' })).toBeNull())
    fireEvent.change(box, { target: { value: '顺便看下配置' } })
    fireEvent.click(await screen.findByRole('button', { name: '追加到本轮' }))

    await waitFor(() =>
      expect(sideChatAppend).toHaveBeenCalledWith(SESSION_KEY, '顺便看下配置', null),
    )
    // 关键不变量：没有新起一轮 send
    expect(sideChatSend).toHaveBeenCalledTimes(1)
    // 入队回执：渲染在输入框上方的固定区（流式期间也可见），不进消息列表——
    // 列表跟着流式内容长，气泡会被顶出视口（用户实测「根本看不到」）
    expect(screen.getByText('顺便看下配置')).toBeTruthy()
    expect(screen.getByText('已入队 · 本步结束后生效')).toBeTruthy()
    expect(document.querySelector('.scp-append-queue')).toBeTruthy()
    expect(document.querySelector('.scp-messages')?.textContent).not.toContain('顺便看下配置')

    // 迭代边界 drain → 后端 emit append_queue_updated → 固定区消失，消息转正式
    act(() => pushLiveEvent(SESSION_KEY, { type: 'append_queue_updated', messages: [] }))
    await waitFor(() => expect(document.querySelector('.scp-append-queue')).toBeNull())
    expect(document.querySelector('.scp-messages')?.textContent).toContain('顺便看下配置')

    act(() => finishSend({ reply: '看完了' }))
    await waitFor(() => expect(screen.getByText('看完了')).toBeTruthy())
  })

  it('取消追加：后端移除成功 → 固定区条目消失，不进历史', async () => {
    let finishSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise<{ reply: string }>(resolve => {
        finishSend = resolve
      }),
    )
    sideChatAppend.mockResolvedValue({ queued: true, duplicate: false, pending: 1 })
    sideChatRemoveAppendItem.mockResolvedValue({ removed: true, pending: [] })
    renderPanel()
    const box = screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…')
    fireEvent.change(box, { target: { value: '先看看目录' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: '发送' })).toBeNull())
    fireEvent.change(box, { target: { value: '顺便看下配置' } })
    fireEvent.click(await screen.findByRole('button', { name: '追加到本轮' }))
    await waitFor(() => expect(document.querySelector('.scp-append-queue')).toBeTruthy())

    // 取消按内容匹配（不是下标）：刷新后 UI 队列与后端队列会漂移，下标会删错人
    fireEvent.click(screen.getByRole('button', { name: '取消追加' }))
    await waitFor(() =>
      expect(sideChatRemoveAppendItem).toHaveBeenCalledWith(SESSION_KEY, '顺便看下配置'),
    )
    await waitFor(() => expect(document.querySelector('.scp-append-queue')).toBeNull())
    // 被取消的追加不进历史：后端没消费它，UI 也不能留痕
    expect(screen.queryByText('顺便看下配置')).toBeNull()

    act(() => finishSend({ reply: '看完了' }))
    await waitFor(() => expect(screen.getByText('看完了')).toBeTruthy())
  })

  it('取消追加：已被迭代边界消费 → 明示已生效，消息转正式留痕', async () => {
    let finishSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise<{ reply: string }>(resolve => {
        finishSend = resolve
      }),
    )
    sideChatAppend.mockResolvedValue({ queued: true, duplicate: false, pending: 1 })
    // 竞态：drain 抢先一步，后端队列里已经没有它（removed=false 不是错误）
    sideChatRemoveAppendItem.mockResolvedValue({ removed: false, pending: [] })
    renderPanel()
    const box = screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…')
    fireEvent.change(box, { target: { value: '先看看目录' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: '发送' })).toBeNull())
    fireEvent.change(box, { target: { value: '顺便看下配置' } })
    fireEvent.click(await screen.findByRole('button', { name: '追加到本轮' }))
    await waitFor(() => expect(document.querySelector('.scp-append-queue')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '取消追加' }))
    // 不重试、不消失：按「已生效」收尾；它进过上下文，历史必须留痕
    await waitFor(() => expect(hudUpdate).toHaveBeenCalledWith('该追加已生效，无法取消', 'warning'))
    expect(document.querySelector('.scp-append-queue')).toBeNull()
    expect(document.querySelector('.scp-messages')?.textContent).toContain('顺便看下配置')

    act(() => finishSend({ reply: '看完了' }))
    await waitFor(() => expect(screen.getByText('看完了')).toBeTruthy())
  })

  it('轮次结束：未消费的追加被后端丢弃，UI 同步清掉（角标不永驻）', async () => {
    let finishSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise<{ reply: string }>(resolve => {
        finishSend = resolve
      }),
    )
    sideChatAppend.mockResolvedValue({ queued: true, duplicate: false, pending: 1 })
    renderPanel()
    const box = screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…')
    fireEvent.change(box, { target: { value: '先看看目录' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: '发送' })).toBeNull())
    // 最后一条迭代边界之后入队：后端 ReturnGuard::drop 会清掉它
    fireEvent.change(box, { target: { value: '最后补一句' } })
    fireEvent.click(await screen.findByRole('button', { name: '追加到本轮' }))
    await waitFor(() => expect(document.querySelector('.scp-append-queue')).toBeTruthy())

    act(() => finishSend({ reply: '看完了' }))
    await waitFor(() => expect(screen.getByText('看完了')).toBeTruthy())
    expect(document.querySelector('.scp-append-queue')).toBeNull()
    expect(screen.queryByText('最后补一句')).toBeNull()
  })

  it('刷新恢复：后端仍在执行 → 恢复「执行中」态（停止键出现）', async () => {
    sideChatState.mockResolvedValue({ exists: true, running: true, last_turn: null })
    renderPanel()
    await waitFor(() => expect(screen.getByRole('button', { name: '停止' })).toBeTruthy())
  })

  it('刷新恢复：已完成但 invoke 丢失的回复按 turn_id 回填，完成事件不重复插入', async () => {
    // 模拟刷新：本地只剩用户消息，助手回复随 Promise 一起丢了
    localStorage.setItem(
      'nuphus.sidechat.msgs.' + SESSION_KEY,
      JSON.stringify([{ role: 'user', text: '刷新前的问题' }]),
    )
    sideChatState.mockResolvedValue({
      exists: true,
      running: false,
      last_turn: { turn_id: 'turn-x', reply: '刷新后才补上的回复', steps: [], ctx_used: 1200 },
    })
    renderPanel()
    await waitFor(() => expect(screen.getByText('刷新后才补上的回复')).toBeTruthy())

    // 同一轮再补一条完成事件：判重，不得落成第二个气泡
    act(() =>
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_turn_finished',
        turn_id: 'turn-x',
        reply: '刷新后才补上的回复',
        steps: [],
        ctx_used: 1200,
      }),
    )
    expect(screen.getAllByText('刷新后才补上的回复').length).toBe(1)
  })

  it('派发轮：dispatch_question 起点落带来源的 user 气泡；finish 不再补 user', async () => {
    // 新语义（2026-11-04）：user 消息只来自「派发起点 dispatch_question」与
    // 「用户手动发送」；turn_finished 只落回复——后端已不带 question，前端
    // 也没有补落逻辑。历史遗留字段带不带 question 都不产生 user 气泡。
    renderPanel()
    act(() =>
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_dispatch_question',
        turn_id: 'dispatch-turn-1',
        question: '核对 side_chat.rs 的门禁语义',
      }),
    )
    await waitFor(() => expect(screen.getByText('核对 side_chat.rs 的门禁语义')).toBeTruthy())
    // 来源徽标：用户一眼区分 Leader 派发
    expect(screen.getByText('Leader 派发')).toBeTruthy()
    act(() =>
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_turn_finished',
        turn_id: 'dispatch-turn-1',
        question: '历史遗留字段：应被忽略',
        reply: '派发结论：门禁已升级',
        steps: [],
        ctx_used: 800,
      }),
    )
    await waitFor(() => expect(screen.getByText('派发结论：门禁已升级')).toBeTruthy())
    // finish 不补 user：遗留 question 不落气泡
    expect(screen.queryByText('历史遗留字段：应被忽略')).toBeNull()
    // 任务（user）与结论（assistant）都落，且任务气泡在前
    const bubbles = document.querySelectorAll('.scp-bubble')
    expect(bubbles.length).toBe(2)
    expect(bubbles[0].className).toContain('scp-bubble--user')
    expect(bubbles[1].className).toContain('scp-bubble--assistant')
  })

  it('摘要栏：派发轮 + 工具事件流 → 完成消息只留静态摘要行（展开已移除）', async () => {
    // 2026-11-04 所有者定：展开内容全是工具名与路径、无信息量——完成消息的
    // 过程条降为静态一行「已用 N 个工具」；实时过程区（执行中）不受影响。
    renderPanel()
    act(() =>
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_dispatch_question',
        turn_id: 'dt-repro',
        question: '核对门禁语义',
      }),
    )
    await waitFor(() => expect(screen.getByText('核对门禁语义')).toBeTruthy())
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        call_id: 'c1',
        tool_name: 'Grep',
        params: { pattern: 'side_chat' },
        from_task: true,
      })
    })
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_end',
        call_id: 'c1',
        success: true,
        duration_ms: 42,
      })
    })
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '结论如下',
        is_thinking: false,
        from_task: true,
      })
    })
    act(() =>
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_turn_finished',
        turn_id: 'dt-repro',
        reply: '结论如下',
        steps: [],
        ctx_used: 100,
      }),
    )
    await waitFor(() => expect(screen.getByText('结论如下')).toBeTruthy())

    // 静态摘要行：一行计数，非按钮、无展开交互
    expect(document.querySelector('.scp-process-row.is-static')).toBeTruthy()
    expect(screen.getByText('已用 1 个工具')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /已用 1 个工具/ })).toBeNull()
    // 明细（工具名/路径）不进完成消息的 DOM
    expect(screen.queryByText('Grep')).toBeNull()
    expect(document.querySelector('.scp-timeline')).toBeNull()
    // 回复正文完整渲染
    expect(screen.getByText('结论如下')).toBeTruthy()
  })

  it('实时过程：收起只剩摘要行 + 条上展开钮，点开内容回来（回看不中断）', async () => {
    // 覆盖「执行中」的收起/展开回路：liveCollapsed 门控 .scp-live-content，
    // 收起后只剩静态摘要行与 live bar 上的展开钮（Running pill 同在条上）。
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '干活' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        call_id: 'c1',
        tool_name: 'Grep',
        params: { pattern: 'x' },
        from_task: true,
      })
    })
    // 展开态：实时内容在 DOM 中（'Grep' 同时出现在摘要行与时间线，用 getAllByText）
    expect(document.querySelector('.scp-live-content')).toBeTruthy()
    expect(screen.getAllByText('Grep').length).toBeGreaterThan(0)
    expect(document.querySelectorAll('.scp-tl-tool-name').length).toBeGreaterThan(0)
    // 收起：内容卸载，只剩摘要行；条上出现「展开」钮
    fireEvent.click(screen.getByRole('button', { name: '收起' }))
    expect(document.querySelector('.scp-live-content')).toBeNull()
    expect(document.querySelector('.scp-timeline')).toBeNull()
    // 展开：内容回来（同一份轨迹，不丢）
    fireEvent.click(screen.getByRole('button', { name: '展开' }))
    expect(document.querySelector('.scp-live-content')).toBeTruthy()
    expect(screen.getAllByText('Grep').length).toBeGreaterThan(0)
    expect(document.querySelectorAll('.scp-tl-tool-name').length).toBeGreaterThan(0)
  })

  it('项目选择：选项来自项目书签，选中后随消息下发 projectDir（不碰全局目录）', async () => {
    getProjectBookmarks.mockResolvedValue([
      { name: 'A 项目', path: 'E:/work/A' },
      { name: '已归档项目', path: 'E:/work/old', archived: true },
    ])
    getProjectDir.mockResolvedValue({ path: 'E:/main', name: '主项目', tag: 'default' })
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    renderPanel()

    // 打开选择器：已设项目列出，已归档的不出现
    fireEvent.click(await screen.findByRole('button', { name: '项目' }))
    expect(await screen.findByText('A 项目')).toBeTruthy()
    expect(screen.queryByText('已归档项目')).toBeNull()
    // 「跟随主会话」项展示主会话目录，用户知道不选是什么后果
    expect(screen.getByText('E:/main')).toBeTruthy()

    fireEvent.click(screen.getByText('A 项目'))
    expect(screen.getByRole('button', { name: '项目' }).textContent).toContain('A 项目')

    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '看下这个项目' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() =>
      expect(sideChatSend).toHaveBeenCalledWith(
        SESSION_KEY,
        '看下这个项目',
        true,
        true,
        'qwen3-max',
        undefined,
        null,
        undefined,
        'E:/work/A',
        expect.any(String),
      ),
    )
    // 只随消息下发：不调用 set_project_dir（渲染路径里没有这个调用点）
    expect(screen.getByText('看下这个项目')).toBeTruthy()
  })

  it('项目选择：Explore 是内置默认项，不属于任何项目路径（发送带哨兵而非路径）', async () => {
    getProjectBookmarks.mockResolvedValue([{ name: 'A 项目', path: 'E:/work/A' }])
    getProjectDir.mockResolvedValue({ path: 'E:/main', name: '主项目', tag: 'default' })
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    renderPanel()

    // 打开选择器：Explore 与「跟随主会话」都是内置项，不来自书签表
    fireEvent.click(await screen.findByRole('button', { name: '项目' }))
    const explore = await screen.findByText('Explore')
    expect(screen.getByText('不绑定任何项目目录')).toBeTruthy()

    fireEvent.click(explore)
    // 触发器显示 Explore，且不当成具体路径：提示语进自定义 tip 气泡
    // （原生 title 已退场——它约 1s 才响应，且样式不可控）
    const trigger = screen.getByRole('button', { name: '项目' })
    expect(trigger.textContent).toContain('Explore')
    expect(trigger.parentElement?.querySelector('.scp-tip-bubble')?.textContent).toBe(
      '不绑定任何项目目录',
    )

    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '纯调研一句' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sideChatSend).toHaveBeenCalled())
    // 第 9 参 = projectDir：哨兵值，不是任何项目路径
    expect((sideChatSend.mock.calls[0] as unknown[])[8]).toBe(EXPLORE_PROJECT)

    // 偏好落盘：用户级选择，刷新（重挂）后不该被打回「跟随主会话」
    const prefs = JSON.parse(localStorage.getItem('nuphus.sidechat.prefs') as string)
    expect(prefs.projectPath).toBe(EXPLORE_PROJECT)
  })

  it('项目选择：跟随主会话与 Explore 是两条路——前者下发 null，后者下发哨兵', async () => {
    getProjectDir.mockResolvedValue({ path: 'E:/main', name: '主项目', tag: 'default' })
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    renderPanel()

    // 默认（跟随主会话）：projectDir = null，后端不注入工作目录段
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '第一句' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sideChatSend).toHaveBeenCalled())
    expect((sideChatSend.mock.calls[0] as unknown[])[8]).toBeNull()

    // 切到 Explore：同一会话后续消息带哨兵（后端据此生成「无项目」段）
    fireEvent.click(screen.getByRole('button', { name: '项目' }))
    fireEvent.click(await screen.findByText('Explore'))
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '第二句' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sideChatSend).toHaveBeenCalledTimes(2))
    expect((sideChatSend.mock.calls[1] as unknown[])[8]).toBe(EXPLORE_PROJECT)
  })

  it('执行过程：有步骤才出现静态摘要行（展开已移除，2026-11-04 所有者定）', async () => {
    sideChatSend.mockResolvedValue({
      reply: '查完了',
      steps: [
        { tool: 'Grep', ok: true, summary: 'side_chat.rs' },
        { tool: 'Read', ok: true, summary: '' },
        { tool: 'web_search', ok: false, summary: 'nuphus 临时对话' },
      ],
    })
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '查一下' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('查完了')).toBeTruthy())

    // 一行静态摘要：「已用 3 个工具」——非按钮、无 aria-expanded、无明细
    expect(document.querySelector('.scp-process-row.is-static')).toBeTruthy()
    expect(screen.getByText('已用 3 个工具')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /已用 3 个工具/ })).toBeNull()
    // 展开内容（工具名/路径/成败样式）不再进完成消息的 DOM
    expect(screen.queryByText('Grep')).toBeNull()
    expect(screen.queryByText('side_chat.rs')).toBeNull()
    expect(document.querySelector('.scp-steps-list')).toBeNull()
    expect(document.querySelector('.scp-timeline')).toBeNull()
  })

  it('无步骤：不渲染执行过程条', async () => {
    sideChatSend.mockResolvedValue({ reply: '直接答' })
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '你好' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('直接答')).toBeTruthy())
    // 无工具步骤 → 不渲染摘要行（2026-11-04 起摘要行只由 steps 驱动）
    expect(document.querySelector('.scp-process')).toBeNull()
  })

  it('模型选择：弹窗先出「默认」与提供商行，展开后才出现具体模型', async () => {
    const base = {
      alias: [] as string[],
      supports_streaming: true,
      supports_vision: false,
      supports_audio: false,
      supports_image_generation: false,
      reasoning_efforts: [] as string[],
    }
    listModels.mockResolvedValue([
      {
        ...base,
        id: 'deepseek-chat',
        provider: 'deepseek',
        context_window: 131072,
        supports_vision: false,
      },
      { ...base, id: 'image-01', provider: 'minimax', supports_image_generation: true },
      {
        ...base,
        id: 'qwen3-max',
        provider: 'qwen',
        context_window: 1048576,
        supports_vision: true,
      },
    ])
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    // 主会话模型名故意取不冲突值：触发器会显示它，别和待选模型重名
    renderPanel({ modelName: 'main-model-x' })
    // 模型列表挂载即拉一次（effort 能力判定也要它）；只拉一次，不随弹窗重复
    expect(listModels).toHaveBeenCalledTimes(1)

    fireEvent.click(screen.getByRole('button', { name: '模型' }))
    // 第一项是「默认（跟随主会话）」
    expect(screen.getByText('默认')).toBeTruthy()
    expect(screen.getAllByText('跟随主会话').length).toBeGreaterThan(0)
    // 提供商成行出现；图片生成模型被过滤，minimax 整行不存在
    await waitFor(() => {
      expect(screen.getByText('deepseek')).toBeTruthy()
      expect(screen.getByText('qwen')).toBeTruthy()
    })
    expect(screen.queryByText('minimax')).toBeNull()
    // 未展开：具体模型不进 DOM（懒加载）
    expect(screen.queryByText('deepseek-chat')).toBeNull()

    // 展开提供商 → 具体模型出现，并带能力标注
    fireEvent.click(screen.getByText('deepseek'))
    expect(screen.getByText('deepseek-chat')).toBeTruthy()
    // 上下文用紧凑读数（131072 → 131.07k），不支持视觉则不出视觉标
    expect(screen.getByText('131.07k')).toBeTruthy()
    expect(screen.queryByText('视觉')).toBeNull()
    // 手风琴：展开 qwen 后 deepseek 收起
    fireEvent.click(screen.getByText('qwen'))
    expect(screen.queryByText('deepseek-chat')).toBeNull()
    expect(screen.getByText('qwen3-max')).toBeTruthy()
    // 1048576 → 1.05M，且多模态模型带「视觉」标
    expect(screen.getByText('1.05M')).toBeTruthy()
    expect(screen.getByText('视觉')).toBeTruthy()

    // 选中后随发送传给后端
    fireEvent.click(screen.getByText('qwen3-max'))
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: 'hi' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() =>
      expect(sideChatSend).toHaveBeenCalledWith(
        SESSION_KEY,
        'hi',
        true,
        true,
        'qwen3-max',
        undefined,
        null,
        // 从提供商分组里选中的模型：provider 必须成对传（否则后端按段序猜）
        'qwen',
        // projectDir：未选项目 → 跟随主会话
        null,
        expect.any(String),
      ),
    )
  })

  it('推理强度：模型未声明 reasoning_efforts 时不渲染入口（能力契约）', async () => {
    const base = {
      alias: [] as string[],
      supports_streaming: true,
      supports_vision: false,
      supports_audio: false,
      supports_image_generation: false,
      reasoning_efforts: [] as string[],
    }
    listModels.mockResolvedValue([{ ...base, id: 'qwen3-max', provider: 'qwen' }])
    renderPanel()
    // 打开模型弹窗触发懒加载后，effort 入口仍不出现（模型没声明档位）
    fireEvent.click(screen.getByRole('button', { name: '模型' }))
    await waitFor(() => expect(listModels).toHaveBeenCalled())
    expect(screen.queryByRole('button', { name: '推理强度' })).toBeNull()
  })

  it('推理强度：声明了档位才出旋钮，选中档位随发送传给后端', async () => {
    const base = {
      alias: [] as string[],
      supports_streaming: true,
      supports_vision: false,
      supports_audio: false,
      supports_image_generation: false,
    }
    listModels.mockResolvedValue([
      {
        ...base,
        id: 'qwen3-max',
        provider: 'qwen',
        reasoning_efforts: ['low', 'high', 'max'],
        default_effort: 'high',
      },
    ])
    sideChatSend.mockResolvedValue({ reply: '深度思考完了' })
    renderPanel()

    // 入口出现（aria-label = 推理强度；等挂载懒加载的模型列表到位）；
    // 未选择时触发器显示「默认」
    await waitFor(() => expect(screen.getByRole('button', { name: '推理强度' })).toBeTruthy())
    const trigger = screen.getByRole('button', { name: '推理强度' })
    expect(screen.getByText('默认')).toBeTruthy()

    fireEvent.click(trigger)
    const menu = document.querySelector('.scp-effort-menu')
    expect(menu).toBeTruthy()
    const optTexts = Array.from(menu?.querySelectorAll('button') ?? []).map(b => b.textContent)
    // 首选项 = 提供商默认，其后是模型声明的三档（high 带默认标注）
    expect(optTexts[0]).toBe('提供商默认')
    expect(optTexts).toContain('low')
    expect(optTexts).toContain('high (默认)')
    expect(optTexts).toContain('max')

    // 选 high：菜单收起、触发器切到档位值
    const highBtn = Array.from(menu?.querySelectorAll('button') ?? []).find(b =>
      b.textContent?.startsWith('high'),
    )
    fireEvent.click(highBtn as Element)
    expect(document.querySelector('.scp-effort-menu')).toBeNull()
    expect(screen.getByText('high')).toBeTruthy()

    // 发送：effort 作为第 7 参传给后端
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '仔细分析' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() =>
      expect(sideChatSend).toHaveBeenCalledWith(
        SESSION_KEY,
        '仔细分析',
        true,
        true,
        'qwen3-max',
        undefined,
        'high',
        // 该用例走「跟随主会话」：model 为空 → provider 也不传
        undefined,
        null,
        expect.any(String),
      ),
    )
  })

  it('推理强度：切换模型后档位重置为默认（档位是模型能力）', async () => {
    const base = {
      alias: [] as string[],
      supports_streaming: true,
      supports_vision: false,
      supports_audio: false,
      supports_image_generation: false,
    }
    listModels.mockResolvedValue([
      { ...base, id: 'qwen3-max', provider: 'qwen', reasoning_efforts: ['low', 'high'] },
      { ...base, id: 'main-model-x', provider: 'qwen', reasoning_efforts: ['low', 'high'] },
    ])
    renderPanel({ modelName: 'main-model-x' })
    await waitFor(() => expect(screen.getByRole('button', { name: '推理强度' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: '推理强度' }))
    const menu = document.querySelector('.scp-effort-menu')
    const highBtn = Array.from(menu?.querySelectorAll('button') ?? []).find(b =>
      b.textContent?.startsWith('high'),
    )
    fireEvent.click(highBtn as Element)
    expect(screen.getByText('high')).toBeTruthy()

    // 经模型弹窗切到另一模型 → 档位回「默认」
    fireEvent.click(screen.getByRole('button', { name: '模型' }))
    await waitFor(() => expect(screen.getByText('qwen')).toBeTruthy())
    fireEvent.click(screen.getByText('qwen'))
    fireEvent.click(screen.getByText('qwen3-max'))
    expect(screen.getByText('默认')).toBeTruthy()
    expect(screen.queryByText('high')).toBeNull()
  })

  it('发送中：发送钮让位给停止钮，重复点击不会第二次调用后端（防并发）', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '第一条' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))

    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())
    // 发送中：发送钮整体让位给停止钮（不是禁用残留）——没有可重复点击的发送入口
    expect(screen.queryByRole('button', { name: '发送' })).toBeNull()
    expect(screen.getByRole('button', { name: '停止' })).toBeTruthy()
    expect(sideChatSend).toHaveBeenCalledTimes(1)

    // 结算后恢复
    await act(async () => {
      resolveSend({ reply: '完成' })
    })
    await waitFor(() => expect(screen.getByText('完成')).toBeTruthy())
    expect(sideChatSend).toHaveBeenCalledTimes(1)
  })

  it('停止：只置位后端中断，不销毁会话不清消息；中断结果是普通气泡', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    sideChatCancel.mockResolvedValue(undefined)
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '在跑了吗' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '停止' }))
    expect(sideChatCancel).toHaveBeenCalledWith(SESSION_KEY)
    // 停止 ≠ 关闭 / 重置：会话与消息必须保留（隔离边界，停止不是销毁）
    expect(sideChatReset).not.toHaveBeenCalled()
    expect(sideChatClose).not.toHaveBeenCalled()

    // 后端封口返回「任务已被用户中断」：普通助手气泡（非 error 态），发送钮复位
    await act(async () => {
      resolveSend({ reply: '任务已被用户中断' })
    })
    await waitFor(() => expect(screen.getByText('任务已被用户中断')).toBeTruthy())
    expect(document.querySelector('.scp-bubble.is-error')).toBeNull()
    expect(screen.getByRole('button', { name: '发送' })).toBeTruthy()
  })

  // ── 刷新续命 / Markdown / 底部操作行 / 分段排序 ──

  it('刷新续命：消息从 localStorage 恢复进气泡（webview 刷新不丢对话）', () => {
    localStorage.setItem(
      `nuphus.sidechat.msgs.${SESSION_KEY}`,
      JSON.stringify([{ role: 'assistant', text: '刷新前的结论' }]),
    )
    renderPanel()
    expect(screen.getByText('刷新前的结论')).toBeTruthy()
    // 空态不出现：有恢复消息就不是空会话
    expect(screen.queryByText('系统操作不可用；关闭或切换会话即清空')).toBeNull()
  })

  it('Markdown 渲染：助手回复的标题/列表成为真元素，不再是一坨纯文本', async () => {
    sideChatSend.mockResolvedValue({ reply: '# 结论\n- 要点甲\n- 要点乙' })
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '问' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('结论')).toBeTruthy())
    // 标题与列表项由 MarkdownContent 渲染为带 class 的真元素
    expect(document.querySelector('.scp-bubble .markdown-h')).toBeTruthy()
    expect(document.querySelectorAll('.scp-bubble .markdown-li').length).toBe(2)
  })

  it('底部单行操作：只挂在最后一条助手回复下，成行不悬浮', async () => {
    sideChatSend
      .mockResolvedValueOnce({ reply: '第一答' })
      .mockResolvedValueOnce({ reply: '第二答' })
    renderPanel()
    const input = screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…')
    fireEvent.change(input, { target: { value: '一问' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('第一答')).toBeTruthy())
    fireEvent.change(input, { target: { value: '二问' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('第二答')).toBeTruthy())

    // 全局只有一条操作行，且紧跟最后一条助手气泡（不是每条都有、不是悬浮角标）
    expect(document.querySelectorAll('.scp-msg-actions').length).toBe(1)
    const bubbles = document.querySelectorAll('.scp-bubble--assistant')
    expect(bubbles.length).toBe(2)
    expect(bubbles[bubbles.length - 1].nextElementSibling?.className).toContain('scp-msg-actions')
  })

  it('实时过程：思考/正文按到达序分段，迭代间不合并成一坨', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '查一下' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    act(() => {
      // 第一轮迭代：思考两段（合并）+ 正文一段
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '想',
        is_thinking: true,
        from_task: true,
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '一下',
        is_thinking: true,
        from_task: true,
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '答',
        is_thinking: false,
        from_task: true,
      })
      // 第二轮迭代：思考与正文各自重新开段，保持真实到达顺序
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '再想',
        is_thinking: true,
        from_task: true,
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '再审',
        is_thinking: false,
        from_task: true,
      })
    })
    const segs = document.querySelectorAll('.scp-live-text')
    expect(segs.length).toBe(4)
    expect(segs[0].textContent).toBe('想一下')
    expect(segs[0].className).toContain('is-thinking')
    expect(segs[1].textContent).toBe('答')
    expect(segs[1].className).not.toContain('is-thinking')
    expect(segs[2].textContent).toBe('再想')
    expect(segs[3].textContent).toBe('再审')

    await act(async () => {
      resolveSend({ reply: '完' })
    })
  })

  it('思考沉淀：执行中的 thinking 不污染最终回复（展开交互已移除）', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '请深入思考' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '这是深度思考过程分析',
        is_thinking: true,
        from_task: true,
      })
    })

    await act(async () => {
      resolveSend({ reply: '这是最终结论' })
    })
    await waitFor(() => expect(screen.getByText('这是最终结论')).toBeTruthy())

    // 无工具步骤 → 不渲染摘要行；thinking 不再作为可折叠内容呈现
    // （2026-11-04 所有者定：完成消息只留静态摘要行，展开内容无信息量）
    expect(document.querySelector('.scp-process')).toBeNull()
    expect(screen.queryByRole('button', { name: '思考过程' })).toBeNull()
    expect(screen.queryByText('这是深度思考过程分析')).toBeNull()
    // 回复本身完整渲染
    expect(screen.getByText('这是最终结论')).toBeTruthy()
  })

  it('发送失败：错误以助手气泡呈现，不抛到界面外', async () => {
    sideChatSend.mockRejectedValue(new Error('网络断了'))
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '在吗' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText(/临时伙伴执行失败: 网络断了/)).toBeTruthy())
    expect(sideChatSend).toHaveBeenCalledTimes(1)
  })

  it('权限固定开启：面板无任何开关，发送恒带 fileAccess/webSearch=true', async () => {
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    renderPanel()
    // 权限设置入口整块移除：role=switch 一个都不该有（否则用户能把自己关到失去全部功能）。
    // 「结论自动交付」是 aria-pressed 按钮而非 role=switch，不在此列。
    expect(screen.queryAllByRole('switch')).toHaveLength(0)

    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '读一下文件' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sideChatSend).toHaveBeenCalled())
    // 第 3、4 参即 fileAccess / webSearch：恒 true
    expect(sideChatSend).toHaveBeenCalledWith(
      SESSION_KEY,
      '读一下文件',
      true,
      true,
      'qwen3-max',
      undefined,
      null,
      undefined,
      null,
      expect.any(String),
    )
  })

  it('清空对话：本地消息立即清空 + 后端 reset 丢弃执行体', async () => {
    sideChatSend.mockResolvedValue({ reply: '第一轮回复' })
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '第一轮' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('第一轮回复')).toBeTruthy())

    // 清空已收进「历史」浮层（保存 / 清空 / 归档三合一）：hover 展开后点 menuitem
    openHistoryMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '清空对话' }))
    await waitFor(() => expect(sideChatReset).toHaveBeenCalledWith(SESSION_KEY))
    expect(screen.queryByText('第一轮')).toBeNull()
    expect(screen.queryByText('第一轮回复')).toBeNull()
    expect(screen.getByText('创意方案 · 探索调研 · 临时任务 · 核对审计')).toBeTruthy()
  })

  it('关闭：点 X 只回调父组件（后端销毁由父组件负责）', () => {
    const { onClose } = renderPanel()
    fireEvent.click(screen.getByRole('button', { name: '关闭临时伙伴' }))
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(sideChatClose).not.toHaveBeenCalled()
  })

  it('收起：只回调父组件，不销毁会话、不清消息', () => {
    const onMinimize = vi.fn()
    renderPanel({ onMinimize })
    fireEvent.click(screen.getByRole('button', { name: '收起（保留对话）' }))
    expect(onMinimize).toHaveBeenCalledTimes(1)
    // 收起 ≠ 关闭：不能碰销毁链路
    expect(sideChatClose).not.toHaveBeenCalled()
    expect(sideChatReset).not.toHaveBeenCalled()
  })

  it('拖拽手柄：按下后跟手位移，松手定格', () => {
    renderPanel()
    const panel = document.querySelector('.scp-panel') as HTMLElement
    const handle = screen.getByLabelText('拖拽移动面板')
    // 初始无位移
    expect(panel.style.transform).toBe('')

    fireEvent.mouseDown(handle, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(window, { clientX: 160, clientY: 130 })
    expect(panel.style.transform).toBe('translate(60px, 30px)')
    expect(panel.className).toContain('is-dragging')

    // 松手后保持位置，退出拖动态，并打上 is-moved 类名防最小化再打开动画抖动
    fireEvent.mouseUp(window, { clientX: 160, clientY: 130 })
    expect(panel.style.transform).toBe('translate(60px, 30px)')
    expect(panel.className).not.toContain('is-dragging')
    expect(panel.className).toContain('is-moved')
  })

  it('最小化态：面板整体收起，消息与输入区留在 DOM 但不渲染', () => {
    const { container } = render(
      <SideChatPanel
        sessionKey={SESSION_KEY}
        modelName="qwen3-max"
        onClose={vi.fn()}
        minimized
        onSendToMain={vi.fn()}
        onSubmitToMain={vi.fn()}
      />,
    )
    // 面板打上最小化类，由 CSS 整体隐藏（不是只藏消息区、留一条 header）
    expect(container.querySelector('.scp-panel')?.className).toContain('is-minimized')
    // 消息区与输入区仍在 DOM 里（state 保留），恢复时原样回来
    expect(container.querySelector('.scp-messages')).toBeTruthy()
    expect(container.querySelector('.scp-composer')).toBeTruthy()
  })

  // ── 实时执行过程（'side-chat-event'，按 session_key 过滤）──

  it('实时过程：tool_call_start 渲染工具名+摘要，非本会话事件被忽略', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '查一下' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    // 无任何实时事件前：仍是「思考中」三点
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    // 别的 session_key 事件直接丢弃：不进本面板过程区
    act(() => {
      pushLiveEvent('other-session-key', {
        type: 'tool_call_start',
        call_id: 'x1',
        tool_name: 'Grep',
        params: { pattern: 'secret' },
        iteration: 1,
        from_task: false,
      })
    })
    expect(screen.queryByText('Grep')).toBeNull()

    // 本会话事件：过程区出现工具名 + 摘要（params.path 优先于 query）
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        call_id: 'c1',
        tool_name: 'Read',
        params: { path: 'src/main.rs', query: 'fn main' },
        iteration: 1,
        from_task: false,
      })
    })
    expect(screen.getAllByText('Read').length).toBeGreaterThan(0)
    expect(screen.getAllByText('src/main.rs').length).toBeGreaterThan(0)
    // 有实时数据即接管「思考中」气泡
    expect(screen.queryByText('思考中…')).toBeNull()

    // tool_call_end：移出在途集合，无其他实时数据时回落「思考中」
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_end',
        call_id: 'c1',
        tool_name: 'Read',
        success: true,
        duration_ms: 12,
      })
    })
    // tool_call_end：在途集合清空，但轨迹保留（状态行继续显示最后一个工具 + 成败与耗时，
    // 不再回落到「思考中」——同步工具的在途窗口可能被合批吞掉，痕迹是唯一证据）
    await waitFor(() =>
      expect(document.querySelector('.scp-live-tool-state')?.textContent).toBe('成功'),
    )
    // 耗时取事件里的 duration_ms（12ms < 1s → 显示 ms）
    expect(document.querySelector('.scp-live-tool-duration')?.textContent).toBe('12ms')

    // 结算：实时态整体清空（迟到的过程事件不再渲染）
    await act(async () => {
      resolveSend({ reply: '完事' })
    })
    await waitFor(() => expect(screen.getByText('完事')).toBeTruthy())
    expect(document.querySelector('.scp-live')).toBeNull()
    expect(screen.queryByText('执行中')).toBeNull()
  })

  it('实时过程：llm_text_delta 按 is_thinking 分段累积（思考斜体段 / 正文段）', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '在吗' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '先想',
        is_thinking: true,
        from_task: true,
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '一下',
        is_thinking: true,
        from_task: true,
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '答案',
        is_thinking: false,
        from_task: true,
      })
    })
    // 思考段累积成一段（is-thinking），正文段单独一段
    const thinking = document.querySelector('.scp-live-text.is-thinking')
    expect(thinking?.textContent).toBe('先想一下')
    const body = document.querySelectorAll('.scp-live-text:not(.is-thinking)')
    expect(body.length).toBe(1)
    expect(body[0].textContent).toBe('答案')

    await act(async () => {
      resolveSend({ reply: '最终回复' })
    })
    await waitFor(() => expect(screen.getByText('最终回复')).toBeTruthy())
    // 流式文本随发送结束清空：最终回复是独立气泡，过程区不留残影
    expect(document.querySelector('.scp-live')).toBeNull()
  })

  // ── 流式累积的幂等性 ──
  //
  // 侧频道没有 seq / 去重（SideChatEmitter 只做裸 emit），重复投递无法在
  // 传输层排除；累积逻辑必须自己识别「无新内容」。曾经的实现把
  // text === last.text 当成增量而整段再拼一遍，且末段被污染后 startsWith
  // 再也不命中，误差沿整条流复利放大——思考文本出现交错重复。

  it('流式累积：重复 / 累积快照 / 重叠重发都不放大，落盘 thinking 与实时一致', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '在吗' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    const thinkingText = () => document.querySelector('.scp-live-text.is-thinking')?.textContent

    // ① 同一 chunk 二次到达 = 重复投递（幽灵监听器 / 上游重发）→ 不重复
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: 'system_info/',
        is_thinking: true,
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: 'system_info/',
        is_thinking: true,
      })
    })
    expect(thinkingText()).toBe('system_info/')

    // ② 累积式快照（provider 整段重发到当前位置）→ 替换而非叠加
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: 'system_info/system_sleep/',
        is_thinking: true,
      })
    })
    expect(thinkingText()).toBe('system_info/system_sleep/')

    // ③ 重叠重发（text 是末段的尾巴）→ 无新内容
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: 'system_sleep/',
        is_thinking: true,
      })
    })
    expect(thinkingText()).toBe('system_info/system_sleep/')

    // ④ 正常增量仍然拼接（回归保护：幂等不能把真增量也吞了）
    act(() => {
      pushLiveEvent(SESSION_KEY, { type: 'llm_text_delta', text: '300s stall.', is_thinking: true })
    })
    expect(thinkingText()).toBe('system_info/system_sleep/300s stall.')

    // ⑤ 收尾：回复落 DOM、实时过程区随 sending=false 卸载。
    //   （「落盘 thinking 与实时一致」的验证随展开交互移除而取消——
    //    2026-11-04 所有者定：完成消息只留静态摘要行。）
    await act(async () => {
      resolveSend({ reply: '完' })
    })
    await waitFor(() => expect(screen.getByText('完')).toBeTruthy())
    expect(document.querySelector('.scp-live-content')).toBeNull()
    expect(screen.queryByRole('button', { name: '思考过程' })).toBeNull()
  })

  // ── 工具调用的可见性 ──
  //
  // 曾经的实现只维护「在途集合」：start 入 Map、end 就删。Read/Grep/Edit 这类
  // 同步工具毫秒级返回，start 与 end 被 React 18 合批成同一次渲染，Map「加了
  // 又删」净值为空——工具行永远不显示（用户看到「只有 thinking/text」）。
  // 主通道的 timeline 用「追加 + 原地改状态」避开这点，这里对齐同一做法。

  it('工具调用：start+end 被合批吞掉在途窗口时，工具名仍然显示（带 ✓）', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '查一下' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    // 同一个 act() 里推 start + end：模拟毫秒级同步工具被自动合批
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        call_id: 'c1',
        tool_name: 'Read',
        params: { path: 'src/main.rs' },
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_end',
        call_id: 'c1',
        tool_name: 'Read',
        success: true,
        duration_ms: 8,
      })
    })
    // 在途集合已空，但轨迹还在：工具名 + 摘要 + 完成标记 + 耗时都必须可见
    expect(screen.getAllByText('Read').length).toBeGreaterThan(0)
    expect(screen.getAllByText('src/main.rs').length).toBeGreaterThan(0)
    expect(document.querySelector('.scp-live-tool-state')?.textContent).toBe('成功')
    expect(document.querySelector('.scp-live-tool-duration')?.textContent).toBe('8ms')
    expect(screen.queryByText('思考中…')).toBeNull()

    await act(async () => {
      resolveSend({ reply: '完' })
    })
  })

  it('工具调用：在途中显示工具名与并行计数，end 后保留痕迹', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(
      new Promise(resolve => {
        resolveSend = resolve
      }),
    )
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '查一下' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    // 三个在途（两个同名）：行内显示 `Grep ×2`
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        call_id: 'g1',
        tool_name: 'Grep',
        params: { pattern: 'foo' },
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        call_id: 'g2',
        tool_name: 'Grep',
        params: { pattern: 'bar' },
      })
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        call_id: 'r1',
        tool_name: 'Read',
        params: { path: 'a.rs' },
      })
    })
    expect(screen.getByText('Grep ×2')).toBeTruthy()

    // 全部结束：在途清空，但最后一个工具（Read）仍以痕迹显示
    act(() => {
      pushLiveEvent(SESSION_KEY, { type: 'tool_call_end', call_id: 'g1', success: true })
      pushLiveEvent(SESSION_KEY, { type: 'tool_call_end', call_id: 'g2', success: true })
      pushLiveEvent(SESSION_KEY, { type: 'tool_call_end', call_id: 'r1', success: true })
    })
    expect(screen.getAllByText('Read').length).toBeGreaterThan(0)
    expect(screen.getAllByText('a.rs').length).toBeGreaterThan(0)

    // 之后再来思考文本：状态行让位给流式文本（工具不长期霸占行首）
    act(() => {
      pushLiveEvent(SESSION_KEY, { type: 'llm_text_delta', text: '想', is_thinking: true })
    })
    expect(screen.getByText('思考中…')).toBeTruthy()
    // 状态行让位给流式文本：a.rs 不再出现在 live 工具行里。
    // （归档痕迹仍保留在过程区——让位只针对「在途/最新工具」那一行，
    //  不是把历史擦掉，所以这里按容器判定而不是全局 queryByText。）
    const stillInLiveToolRow = screen
      .queryAllByText('a.rs')
      .some(el => el.closest('[class*="scp-live-tool"]') !== null)
    expect(stillInLiveToolRow).toBe(false)

    await act(async () => {
      resolveSend({ reply: '完' })
    })
  })

  it('工具调用：上游不下发 call_id 也要显示（不再整条丢弃）', async () => {
    sideChatSend.mockReturnValue(new Promise(() => {}))
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '查一下' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'tool_call_start',
        tool_name: 'Glob',
        params: { patterns: ['*.rs'] },
      })
    })
    // 旧实现在 `if (!callId) return` 处整条丢弃——这一条就是它的回归
    expect(screen.getAllByText('Glob').length).toBeGreaterThan(0)

    // end 同样没有 id：标到最近一条未完成的记录上
    act(() => {
      pushLiveEvent(SESSION_KEY, { type: 'tool_call_end', tool_name: 'Glob', success: true })
    })
    expect(document.querySelector('.scp-live-tool-state')?.textContent).toBe('成功')
  })

  it('流式内容可收起：点「收起」后正文/思考只在过程行体现，可再展开', async () => {
    let resolveSend: (v: { reply: string }) => void = () => {}
    sideChatSend.mockReturnValue(new Promise(r => (resolveSend = r)))
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '跑一下' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('思考中…')).toBeTruthy())

    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'llm_text_delta',
        text: '正文片段',
        is_thinking: false,
        from_task: true,
      })
    })
    expect(document.querySelector('.scp-live-content')).toBeTruthy()

    // 收起：内容区消失，过程行（状态摘要）仍在
    fireEvent.click(screen.getByRole('button', { name: '收起' }))
    expect(document.querySelector('.scp-live-content')).toBeNull()
    expect(document.querySelector('.scp-process-row')).toBeTruthy()

    // 再展开
    fireEvent.click(screen.getByRole('button', { name: '展开' }))
    expect(document.querySelector('.scp-live-content')).toBeTruthy()

    await act(async () => {
      resolveSend({ reply: '完' })
    })
  })

  // ── 历史浮层（保存 / 清空 / 最近归档三合一）──

  it('历史浮层：hover 展开，列出最近 10 条归档并可逐条删除', async () => {
    sideChatHistoryList.mockResolvedValue([
      {
        id: 'h1',
        ts: '2026-11-02 10:00:00',
        model: 'qwen3-max',
        question: '第一条问题',
        reply: 'r1',
        ok: true,
      },
      {
        id: 'h2',
        ts: '2026-11-02 09:00:00',
        model: '',
        question: '第二条问题',
        reply: 'r2',
        ok: false,
      },
    ])
    sideChatHistoryDelete.mockResolvedValue(undefined)
    renderPanel()

    openHistoryMenu()
    await waitFor(() => expect(screen.getByText('第一条问题')).toBeTruthy())
    expect(screen.getByText('第二条问题')).toBeTruthy()
    // 需求：最多 10 条最新
    expect(sideChatHistoryList).toHaveBeenCalledWith(10)

    // 逐条删除：调后端 + 该行消失，其余保留
    fireEvent.click(screen.getAllByRole('button', { name: '删除该记录' })[0])
    await waitFor(() => expect(sideChatHistoryDelete).toHaveBeenCalledWith('h1'))
    await waitFor(() => expect(screen.queryByText('第一条问题')).toBeNull())
    expect(screen.getByText('第二条问题')).toBeTruthy()
  })

  it('历史浮层：穿过图标与浮层之间的间隙不会误关（延迟关闭）', async () => {
    renderPanel()
    const wrap = document.querySelector('.scp-history-wrap') as Element
    fireEvent.mouseEnter(wrap)
    await waitFor(() => expect(document.querySelector('.scp-history-menu')).toBeTruthy())

    // 鼠标路过间隙（离开触发区但还没进浮层）：此刻不许立刻关
    fireEvent.mouseLeave(wrap)
    expect(document.querySelector('.scp-history-menu')).toBeTruthy()

    // 真正离开（超过延迟窗口）才收起
    await waitFor(() => expect(document.querySelector('.scp-history-menu')).toBeNull(), {
      timeout: 1200,
    })
  })

  // ── 偏好持久化（刷新 / 重开面板不打回默认）──

  it('偏好持久化：选定的模型写入 localStorage，重挂后原样恢复', async () => {
    listModels.mockResolvedValue([
      { id: 'pick-me', provider: 'p1', context_window: 1000, supports_vision: false },
    ])
    const first = renderPanel()
    // 选一个非默认模型。模型列表是挂载后异步拉的：必须等提供商行出现再点，
    // 否则点空（用例假红）。
    fireEvent.click(screen.getByRole('button', { name: '模型' }))
    await waitFor(() => expect(screen.getByRole('button', { name: /p1/ })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: /p1/ }))
    await waitFor(() => expect(screen.getByText('pick-me')).toBeTruthy())
    fireEvent.click(screen.getByText('pick-me'))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: '模型' }).textContent).toContain('pick-me'),
    )
    // 权限入口已移除：面板不再提供「联网搜索」开关（全关会让面板失能），
    // webSearch 恒开并随发送下发；此处只验证重挂后模型选择不丢。
    first.unmount()

    // 重挂 = 刷新：偏好必须原样回来（此前 model 是裸 state，必然打回跟随主会话）
    renderPanel()
    expect(screen.getByText('pick-me')).toBeTruthy()
    expect(screen.queryAllByRole('switch')).toHaveLength(0)
  })

  it('模型与提供商成对：选中即带 provider 发送，且两字段一起落盘', async () => {
    listModels.mockResolvedValue([
      { id: 'pick-me', provider: 'p1', context_window: 1000, supports_vision: false },
    ])
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    const first = renderPanel()

    fireEvent.click(screen.getByRole('button', { name: '模型' }))
    await waitFor(() => expect(screen.getByRole('button', { name: /p1/ })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: /p1/ }))
    await waitFor(() => expect(screen.getByText('pick-me')).toBeTruthy())
    fireEvent.click(screen.getByText('pick-me'))

    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '问一句' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sideChatSend).toHaveBeenCalled())

    // 后端拿到的第 5 参 = 模型，第 8 参 = provider（缺它后端只能按段序猜）
    const args = sideChatSend.mock.calls[0] as unknown[]
    expect(args[4]).toBe('pick-me')
    expect(args[7]).toBe('p1')

    // 偏好里两个字段都在（同名模型跨提供商时只存 id 会重蹈 8317 覆辙）
    const prefs = JSON.parse(localStorage.getItem('nuphus.sidechat.prefs') as string)
    expect(prefs.model).toBe('pick-me')
    expect(prefs.provider).toBe('p1')

    first.unmount()
    renderPanel()
    expect(screen.getByRole('button', { name: '模型' }).textContent).toContain('pick-me')
  })

  it('监听退订：listen() 在卸载之后才 resolve 也必须退订（幽灵监听器回归）', async () => {
    // 复现竞态：cleanup 先于 listen() 的 Promise resolve。旧实现在 resolve 时
    // 发现已卸载就直接 return——监听器已经注册却永不退订，此后同一条事件会被
    // 这个 handler 再处理一次，是 dev 下重复投递的根源。
    let resolveListen: (fn: () => void) => void = () => {}
    listenMock.mockImplementation(
      () =>
        new Promise<() => void>(resolve => {
          resolveListen = resolve
        }),
    )
    const { unmount } = renderPanel()
    unmount()
    // 卸载时 promise 未 resolve：unlisteners 还是空的，退订无处可调
    expect(unlistenMock).not.toHaveBeenCalled()

    await act(async () => {
      resolveListen(unlistenMock)
    })
    // 迟到的注册必须立刻退订，否则它会在整个会话期间重复处理每条事件
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  // ── 放入输入栏 / 立即发送（纯图标钮，无障碍名 = tip 文案）──

  it('放入输入栏：按钮回调 onSendToMain，参数是回复全文（不截断）', async () => {
    // 长文本单行（无空白归一化干扰）：证明送出去的是全文而非 chip 截断版
    const full = '结论段。' + '依据内容'.repeat(120)
    sideChatSend.mockResolvedValue({ reply: full })
    const onSendToMain = vi.fn()
    renderPanel({ onSendToMain })
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '问个事' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText(full)).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '放进主会话输入栏，不自动执行' }))
    expect(onSendToMain).toHaveBeenCalledTimes(1)
    expect(onSendToMain).toHaveBeenCalledWith(full)
  })

  it('立即发送：按钮回调 onSubmitToMain，参数是回复全文；无回复时无入口', async () => {
    const full = '结论段。' + '依据内容'.repeat(120)
    sideChatSend.mockResolvedValue({ reply: full })
    const onSubmitToMain = vi.fn()
    renderPanel({ onSubmitToMain })
    // 闸门：没有任何助手回复时面板不提供「立即发送」入口——
    // 临时伙伴的回复没有自动通往主会话的路径，只有用户显式点这一下
    expect(screen.queryByRole('button', { name: '立即提交主会话并执行' })).toBeNull()

    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '问个事' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText(full)).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '立即提交主会话并执行' }))
    expect(onSubmitToMain).toHaveBeenCalledTimes(1)
    expect(onSubmitToMain).toHaveBeenCalledWith(full)
  })

  it('消息操作行：显示本轮 ctx 消耗（used / cap）', async () => {
    listModels.mockResolvedValue([
      {
        alias: [],
        supports_streaming: true,
        supports_vision: false,
        supports_audio: false,
        supports_image_generation: false,
        reasoning_efforts: [],
        id: 'qwen3-max',
        provider: 'qwen',
        context_window: 131072,
      },
    ])
    sideChatSend.mockResolvedValue({ reply: '答完了', ctx_used: 64000 })
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '看看' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('答完了')).toBeTruthy())

    const ctx = document.querySelector('.scp-msg-ctx')
    expect(ctx).toBeTruthy()
    // 64k / 131072：分子分母都按 formatTokens 口径
    expect(ctx?.textContent).toContain('64')
    expect(ctx?.textContent).toContain('131')
  })

  it('面板尺寸：一键放大写下内联尺寸并落盘，还原后回到默认几何', () => {
    renderPanel()
    // 默认几何：aside 上没有 width/height 内联样式（由 CSS 的 380px + top/bottom 推导）
    const panel = document.querySelector('.scp-panel') as HTMLElement
    expect(panel.style.width).toBe('')

    fireEvent.click(screen.getByRole('button', { name: '放大面板' }))
    const zoomed = document.querySelector('.scp-panel') as HTMLElement
    // jsdom 下宿主实测为 0，尺寸被下限兜住；真实浏览器取宿主比例（62%/88%）
    expect(zoomed.style.width).not.toBe('')
    expect(zoomed.style.height).not.toBe('')
    expect(zoomed.style.bottom).toBe('auto')
    expect(JSON.parse(localStorage.getItem('nuphus.sidechat.prefs') as string).size).not.toBeNull()

    // 按钮语义跟着切成「还原」，点回默认（内联样式清空、偏好回落 null）
    fireEvent.click(screen.getByRole('button', { name: '还原面板大小' }))
    const back = document.querySelector('.scp-panel') as HTMLElement
    expect(back.style.width).toBe('')
    expect(back.style.height).toBe('')
    expect(JSON.parse(localStorage.getItem('nuphus.sidechat.prefs') as string).size).toBeNull()
  })

  it('面板尺寸：边缘拉伸把手在 DOM 中且带可访问名（命中热区存在）', () => {
    renderPanel()
    expect(screen.getByRole('button', { name: '拖拽调整面板大小' })).toBeTruthy()
  })

  // ── 输入区：附件 + skill ──

  it('输入区：附件 chip 增删、发送拼 [附件: path]，skill chip 走 references', async () => {
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    invokeMock.mockResolvedValue([
      { name: 'demo-skill', display_name: '演示技能', description: 'd' },
    ])
    renderPanel()

    // 附件：隐藏 file input 选文件（Tauri File.path）→ chip 显示文件名
    const file = new File(['x'], '报告.pdf', { type: 'application/pdf' })
    Object.defineProperty(file, 'path', { value: 'C:/docs/报告.pdf' })
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })
    expect(screen.getByText('报告.pdf')).toBeTruthy()
    // 移除钮可删掉 chip
    fireEvent.click(screen.getByRole('button', { name: '移除附件' }))
    expect(screen.queryByText('报告.pdf')).toBeNull()

    // 重新加回附件（移除后仍可再选）
    const file2 = new File(['x'], '报告.pdf', { type: 'application/pdf' })
    Object.defineProperty(file2, 'path', { value: 'C:/docs/报告.pdf' })
    fireEvent.change(input, { target: { files: [file2] } })
    expect(screen.getByText('报告.pdf')).toBeTruthy()

    // skill：+ 菜单 → 选择技能 → 浮层选中 → chip
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '选择技能' }))
    await waitFor(() => expect(screen.getByRole('option', { name: '演示技能' })).toBeTruthy())
    fireEvent.click(screen.getByRole('option', { name: '演示技能' }))
    expect(screen.getByText('演示技能')).toBeTruthy()
    // skill chip 移除钮
    fireEvent.click(screen.getByRole('button', { name: '移除技能' }))
    expect(screen.queryByText('演示技能')).toBeNull()
    // 重新选定（发送断言要用）
    fireEvent.click(screen.getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('menuitem', { name: '选择技能' }))
    await waitFor(() => expect(screen.getByRole('option', { name: '演示技能' })).toBeTruthy())
    fireEvent.click(screen.getByRole('option', { name: '演示技能' }))

    // 发送：消息尾拼 [附件: path]；skill 走第 6 参 references
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '看看' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(sideChatSend).toHaveBeenCalled())
    expect(sideChatSend).toHaveBeenCalledWith(
      SESSION_KEY,
      '看看\n[附件: C:/docs/报告.pdf]',
      true,
      true,
      'qwen3-max',
      [{ type: 'skill', id: 'demo-skill', label: '演示技能' }],
      null,
      undefined,
      null,
      expect.any(String),
    )
    // 发送成功：chips 清空
    expect(screen.queryByText('报告.pdf')).toBeNull()
    expect(screen.queryByText('演示技能')).toBeNull()
  })

  it('输入区：拿不到 File.path 时回落文件名并 hud 明示（不静默丢失）', () => {
    renderPanel()
    // 不定义 path：模拟浏览器 / 拿不到绝对路径的环境
    const file = new File(['x'], '无路径.txt')
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })
    // chip 仍在（不丢附件），hud 明示「只发了文件名」
    expect(screen.getByText('无路径.txt')).toBeTruthy()
    expect(hudUpdate).toHaveBeenCalledWith('已添加附件', 'info')
    expect(hudUpdate).toHaveBeenCalledWith(expect.stringContaining('仅发送文件名'), 'warning')
  })

  it('输入区：附件可加、路径正常时只出添加提示（面板无权限开关）', () => {
    renderPanel()
    // 权限入口已移除：不提供可关开关（两个开关全关会让面板失去全部功能）
    expect(screen.queryAllByRole('switch')).toHaveLength(0)

    // 路径正常：只应有添加提示，不再有「模型读不到」的关闭态警告
    const file = new File(['x'], '报告.pdf', { type: 'application/pdf' })
    Object.defineProperty(file, 'path', { value: 'C:/docs/报告.pdf' })
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })
    // 能力不剥夺：chip 照常入列，发送入口不受影响
    expect(screen.getByText('报告.pdf')).toBeTruthy()
    expect(hudUpdate).toHaveBeenCalledWith('已添加附件', 'info')
    // 权限恒开：不再有「模型读不到」这类警告
    expect(hudUpdate).not.toHaveBeenCalledWith(expect.stringContaining('模型无法读取'), 'warning')
    // 路径正常时也不该蹭「仅发送文件名」那条（禁止复用糊弄）
    expect(hudUpdate).not.toHaveBeenCalledWith(expect.stringContaining('仅发送文件名'), 'warning')
  })

  it('输入区：拿不到路径时明示仅发送文件名（与权限无关的既有警告）', () => {
    renderPanel()
    // 不定义 path：模拟浏览器 / 拿不到绝对路径的环境
    const file = new File(['x'], '无路径.txt')
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })
    expect(screen.getByText('无路径.txt')).toBeTruthy()
    expect(hudUpdate).toHaveBeenCalledWith('已添加附件', 'info')
    expect(hudUpdate).toHaveBeenCalledWith(expect.stringContaining('仅发送文件名'), 'warning')
  })

  // ── 保存 / 分享 ──

  it('保存：组装会话 md 调 sideChatSave，toast 回显落盘路径', async () => {
    sideChatSend.mockResolvedValue({
      reply: '回复内容',
      steps: [{ tool: 'Read', ok: true, summary: 'a.rs' }],
    })
    sideChatSave.mockResolvedValue('C:/Users/x/.nuphus/side-chat/20261101-120000-abcdef12.md')
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '问题' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('回复内容')).toBeTruthy())

    openHistoryMenu()
    fireEvent.click(screen.getByRole('menuitem', { name: '保存' }))
    await waitFor(() => expect(sideChatSave).toHaveBeenCalled())
    const [key, md] = sideChatSave.mock.calls[0] as [string, string]
    expect(key).toBe(SESSION_KEY)
    // md 结构：标题 + 元信息 + 每轮 大王/临时助手 + 工具步骤 details
    expect(md).toContain('# 临时伙伴存档')
    expect(md).toContain('模型：qwen3-max')
    expect(md).toContain('## 大王')
    expect(md).toContain('问题')
    expect(md).toContain('## 临时助手')
    expect(md).toContain('回复内容')
    expect(md).toContain('<details><summary>工具步骤（1）</summary>')
    // 文字状态替代 ✓/✗：导出物常被贴进文档 / issue，emoji 在别处会变乱码方块
    expect(md).toContain('- Read 成功 a.rs')
    // toast 回显后端返回的路径
    expect(hudUpdate).toHaveBeenCalledWith(
      '已保存到 C:/Users/x/.nuphus/side-chat/20261101-120000-abcdef12.md',
      'done',
    )
  })

  it('分享：同款 md 复制到剪贴板并 toast「已复制」', async () => {
    sideChatSend.mockResolvedValue({ reply: '可分享的回复' })
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    renderPanel()
    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '讲个事' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('可分享的回复')).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: '分享' }))
    await waitFor(() => expect(writeText).toHaveBeenCalled())
    const copied = writeText.mock.calls[0]?.[0] as string
    expect(copied).toContain('# 临时伙伴存档')
    expect(copied).toContain('可分享的回复')
    expect(hudUpdate).toHaveBeenCalledWith('已复制到剪贴板', 'done')
  })
})

/** 并行协作契约（批2）：侧边双可见 + 结论自动交付的**派发门禁**。
 *  门禁由后端按派发态自动判定（仅主会话正在派发执行任务时才自动交付），
 *  前端**没有开关**——闲聊结论推进主会话就是无关信息涌入，违背隔离职责。
 *  这里锁两个可见性行为：主会话执行中细提示显隐、待交付计数显隐，
 *  外加「轮次结束刷一次计数」的接线用例。 */
describe('SideChatPanel 并行协作契约', () => {
  beforeEach(() => {
    listModels.mockResolvedValue([])
    getProjectBookmarks.mockResolvedValue([])
    getProjectDir.mockResolvedValue({ path: '', name: '', tag: 'default' })
    sideChatHistoryList.mockResolvedValue([])
    sideChatState.mockResolvedValue({ exists: false, running: false, last_turn: null })
    sideChatSend.mockResolvedValue({ reply: 'ok' })
    liveHandlers = []
    // 'side-chat-event' 监听：与首个 describe 同一套捕获手法（缺了它组件挂载即炸）
    listenMock.mockImplementation((_event: string, handler: SideChatHandler) => {
      liveHandlers.push(handler)
      return Promise.resolve(unlistenMock)
    })
    invokeMock.mockReset()
  })

  afterEach(() => {
    // 与首个 describe 同一套清理：本块用例各自 render，不清则面板 / localStorage /
    // mock 调用记录跨用例串流（实测：上一个用例的 'ok' 气泡会让 getByText 撞多重匹配）
    localStorage.clear()
    cleanup()
    liveHandlers = []
    sideChatSend.mockReset()
    sideChatState.mockReset()
    listenMock.mockReset()
    unlistenMock.mockReset()
    invokeMock.mockReset()
  })

  it('主会话执行中细提示：传入 Exec 标题即渲染，null / 空不渲染', () => {
    const { unmount } = renderPanel({ mainExecTitle: '跑一遍构建验证' })
    expect(screen.getByText('主会话正在执行：跑一遍构建验证')).toBeTruthy()
    unmount()

    renderPanel({ mainExecTitle: null })
    expect(screen.queryByText(/主会话正在执行：/)).toBeNull()
  })

  it('待交付计数：pending_deliveries>0 渲染提示，0 / 缺省不渲染', async () => {
    sideChatState.mockResolvedValue({
      exists: true,
      running: false,
      last_turn: null,
      pending_deliveries: 2,
    })
    renderPanel()
    expect(await screen.findByText('2 条结论待交付（主会话收尾中，下轮自动补投）')).toBeTruthy()
  })

  it('待交付计数：后端未下发该字段（旧版本）按 0 处理，不渲染提示', async () => {
    sideChatState.mockResolvedValue({ exists: true, running: false, last_turn: null })
    renderPanel()
    // 等挂载 effect 的 state 查询落定：查询成功但没有 pending 字段
    await waitFor(() => expect(sideChatState).toHaveBeenCalled())
    expect(screen.queryByText(/条结论待交付/)).toBeNull()
  })

  it('轮次结束刷一次计数：本轮结论可能被拒收落箱，finally 再查 sideChatState', async () => {
    sideChatState.mockResolvedValue({
      exists: true,
      running: false,
      last_turn: null,
      pending_deliveries: 1,
    })
    renderPanel()
    // 挂载时已查一次（此时计数就会显示）
    expect(await screen.findByText('1 条结论待交付（主会话收尾中，下轮自动补投）')).toBeTruthy()

    fireEvent.change(screen.getByLabelText('创意方案 · 探索调研 · 临时任务 · 核对审计…'), {
      target: { value: '再查一句' },
    })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    await waitFor(() => expect(screen.getByText('ok')).toBeTruthy())
    // 发送结束后又查一次（挂载 1 次 + finally 1 次）
    await waitFor(() => expect(sideChatState.mock.calls.length).toBeGreaterThanOrEqual(2))
  })
})

describe('SideChatPanel 接线契约（源码断言）', () => {
  // 本 describe 以源码断言为主，但「在途态入口」三条回归要真渲染：这里补一套
  // 与上一个 describe 同构的 mock 基线（上一个 describe 的 afterEach 做过
  // mockReset，不补的话 sideChatState 返回 undefined，面板挂载即炸）。
  beforeEach(() => {
    listModels.mockResolvedValue([])
    getProjectBookmarks.mockResolvedValue([])
    getProjectDir.mockResolvedValue({ path: '', name: '', tag: 'default' })
    sideChatHistoryList.mockResolvedValue([])
    sideChatState.mockResolvedValue({ exists: false, running: false, last_turn: null })
    sideChatPrepare.mockResolvedValue({ created: true })
    sideChatCancel.mockResolvedValue(undefined)
    sideChatReset.mockResolvedValue(undefined)
    liveHandlers = []
    listenMock.mockImplementation((_event: string, handler: SideChatHandler) => {
      liveHandlers.push(handler)
      return Promise.resolve(unlistenMock)
    })
    invokeMock.mockReset()
  })

  afterEach(() => {
    localStorage.clear()
    cleanup()
    liveHandlers = []
    sideChatSend.mockReset()
    sideChatAppend.mockReset()
    sideChatState.mockReset()
    listenMock.mockReset()
    unlistenMock.mockReset()
    invokeMock.mockReset()
  })

  it('入口按钮是 ChatPanel 左下角浮动，已移出输入栏操作组', () => {
    // 输入栏里不再有临时伙伴按钮
    expect(inputBarSrc).not.toContain('input-side-chat-btn')
    expect(inputBarSrc).not.toContain('onToggleSideChat')
    expect(inputBarSrc).not.toContain('IconMessageCircle')
    // 入口改为对话窗口左下角的浮动按钮，由 ChatPanel 直接挂（图标 = Bot）
    expect(chatPanelSrc).toContain('chat-side-chat-fab')
    expect(chatPanelSrc).toContain('IconBot')
    expect(chatPanelSrc).toContain('onClick={toggleSideChat}')
    expect(chatPanelSrc).toContain("t('sideChat.title')")
  })

  it('面板挂在 .chat-panel 内，随 sideChatKey 条件渲染', () => {
    expect(chatPanelSrc).toContain('<div className="chat-panel">')
    expect(chatPanelSrc).toContain('<SideChatPanel')
    expect(chatPanelSrc).toContain('sessionKey={sideChatKey}')
    // 主会话切换 → 关闭面板（onSessionChanged 走包装后的 handleChatReplaced）
    expect(chatPanelSrc).toContain('onSessionChanged={handleChatReplaced}')
    expect(chatPanelSrc).toContain('const handleChatReplaced')
  })

  it('送入主对话接线：ChatPanel 传 onSendToMain，handler 构造 sidechat 引用（全文）', () => {
    // 挂载处传 prop
    expect(chatPanelSrc).toContain('onSendToMain={handleSideChatToMain}')
    expect(chatPanelSrc).toContain('const handleSideChatToMain')
    // handler 复用既有引用通道：sidechat 类型 + content 全文（前端不截断）
    expect(chatPanelSrc).toContain("type: 'sidechat'")
    expect(chatPanelSrc).toContain('content: trimmed')
    expect(chatPanelSrc).toContain('addQuoteReference(')
  })

  // ── 在途态的三个入口（大王 2026-11-04 报障的回归）──
  // 症候：派发轮里用户在伙伴窗口发指令「卡住不动、等主会话跑完才好」，
  // 追加路径与停止按钮都找不到。根因是 composer 的在途态只由前端 sending
  // 投影驱动，而派发这一路没人给它置位；后端 BUSY 又是权威在途信号却不被采纳。
  it('派发起点事件即置在途态：composer 从「发送」切到「追加 + 停止」', async () => {
    renderPanel()
    // 空闲态：只有发送钮，没有停止钮
    expect(screen.queryByRole('button', { name: '停止' })).toBeNull()
    // Leader 派发：后端在 await send_impl **之前** emit 起点事件
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_dispatch_question',
        turn_id: 'turn-dispatch-1',
        question: '核对一遍构建',
      })
    })
    // 在途态：停止钮立刻可用（追加钮要有输入才出现）
    expect(screen.getByRole('button', { name: '停止' })).toBeTruthy()
  })

  it('派发轮在途追加（queued）不重复置位，仅落队气泡', async () => {
    renderPanel()
    // 先来一发非 queued 的派发起点：把 composer 置在途态（queued 气泡只在
    // 在途态的「已入队」固定区渲染，这正是它在途才可见的契约）
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_dispatch_question',
        turn_id: 'turn-dispatch-2',
        question: '第一批任务',
      })
    })
    // 派发轮在途时 Leader 再派一件 → queued 气泡（不重复置位、不新开轮次）
    act(() => {
      pushLiveEvent(SESSION_KEY, {
        type: 'side_chat_dispatch_question',
        turn_id: '',
        question: '第二批任务',
        queued: true,
      })
    })
    expect(await screen.findByText('第二批任务')).toBeTruthy()
    // 两条都在：起点一条 + 入队一条
    expect(screen.getAllByText(/任务$/).length).toBeGreaterThanOrEqual(2)
  })

  it('后端 BUSY = 在途权威：发送降级为追加，不出错误气泡', async () => {
    // 前端 sending 投影落后于事实时的兜底（派发起点事件未到 / 监听漏事件）
    sideChatSend.mockRejectedValue(new Error('该临时会话正在执行上一条消息，请稍候'))
    sideChatAppend.mockResolvedValue({ queued: true, duplicate: false, pending: 1 })
    renderPanel()
    fireEvent.change(screen.getByLabelText(/探索调研/), { target: { value: '补一句' } })
    fireEvent.click(screen.getByRole('button', { name: '发送' }))
    // 降级为追加：进后端队列（队列插入检查在后端），落 queued 气泡
    await waitFor(() => expect(sideChatAppend).toHaveBeenCalledWith(SESSION_KEY, '补一句', null))
    expect(await screen.findByText('补一句')).toBeTruthy()
    // 不当失败呈现：错误气泡会带上后端原文，这里必须不出现
    expect(screen.queryByText(/该临时会话正在执行上一条消息/)).toBeNull()
    // 轮次仍在途：composer 留在在途态（停止钮在，用户停得下来）
    expect(screen.getByRole('button', { name: '停止' })).toBeTruthy()
  })

  it('BUSY 文案镜像：前端常量与后端 BUSY_MESSAGE 同文（改一侧必须双改）', () => {
    const sideChatRs = read('../../../../src-tauri/src/commands/side_chat.rs')
    const backend = sideChatRs.match(/const BUSY_MESSAGE: &str = "([^"]+)"/)?.[1]
    expect(backend).toBeTruthy()
    expect(read('./SideChatPanel.tsx')).toContain(`const BACKEND_BUSY_MESSAGE = '${backend}'`)
  })

  it('面板 memo 隔断主会话重渲染风暴（父级每 token 重渲染不再波及伙伴窗口）', () => {
    const src = read('./SideChatPanel.tsx')
    expect(src).toContain('export const SideChatPanel = memo(SideChatPanelImpl)')
  })

  it('实时过程契约：面板监听独立频道 side-chat-event 并按 sessionKey 过滤', () => {
    // 事件名与主会话 nuphus-event 严格分离（前端源码断言）
    expect(chatPanelSrc).not.toContain("'side-chat-event'")
    // 面板侧：listen 独立频道 + session_key 过滤 + 白名单四类事件
    const src = read('./SideChatPanel.tsx')
    expect(src).toContain("'side-chat-event'")
    expect(src).toContain('data.session_key !== sessionKey')
    for (const type of ['tool_call_start', 'tool_call_end', 'llm_text_delta', 'tool_output_line']) {
      expect(src).toContain(type)
    }
    // 过程区 DOM 契约：工具行 + thinking/正文两段
    expect(css).toContain('.scp-live-tool')
    expect(css).toContain('.scp-live-text.is-thinking')
  })

  it('流式滚动不抢用户操作：复用 useStickyScroll，禁止裸写 scrollTop', () => {
    const src = read('./SideChatPanel.tsx')
    // 走与主对话 / 执行追踪面板同一贴底实现（手势才是跟随态唯一入口）
    expect(src).toContain("from '../../hooks/useStickyScroll'")
    expect(src).toContain('useStickyScroll(stickKey')
    expect(src).toContain('ref={scrollRef}')
    expect(src).toContain('onScroll={onScroll}')
    // 反模式：内容一变更就裸改 scrollTop（会把正在上翻阅读的用户拽回底部）
    expect(src).not.toContain('el.scrollTop = el.scrollHeight')
    // 关掉浏览器滚动锚定（否则流式增长会反向改 scrollTop 与跟随争抢）
    expect(css).toMatch(/\.scp-messages\s*\{[^}]*overflow-anchor:\s*none/)
  })

  it('发送 / 追加都强制回底：followReset 从 hook 解构并在两处调用', () => {
    const src = read('./SideChatPanel.tsx')
    // 必须从 hook 拿到 followReset（漏解构 = 调一个不存在的变量，编译红）
    expect(src).toMatch(/useStickyScroll\(stickKey[\s\S]{0,200}followReset/)
    // 新轮次（handleSend）与执行中追加（handleAppend）都要落底
    expect(src.match(/followReset\(\)/g)?.length).toBe(2)
    // 走 followReset 而非裸 jumpToBottom：新轮次要确定性瞬移，不要平滑动画
    expect(src).not.toMatch(/setMessages\(prev => \[\.\.\.prev[\s\S]{0,120}jumpToBottom\(\)/)
  })

  it('中断按钮复用主输入框终止语义键（不另造配色）+ 思考块无左侧边框', () => {
    // 终止色走 --send-interrupt-*（简白=墨黑反色 / dark·fresh=红），与主输入框一致
    expect(css).toMatch(/\.scp-send\.is-stop\s*\{[^}]*var\(--send-interrupt-fill\)/)
    expect(css).toMatch(/\.scp-send\.is-stop\s*\{[^}]*var\(--send-interrupt-fg\)/)
    expect(css).not.toMatch(/\.scp-send\.is-stop\s*\{[^}]*background:\s*var\(--error\)/)
    // 思考内容不再带左侧竖线边框（视觉噪声）
    expect(css).not.toMatch(/\.scp-live-text\.is-thinking\s*\{[^}]*border-left/)
    expect(css).not.toMatch(/\.scp-thinking-text\s*\{[^}]*border-left/)
  })

  it('历史浮层交接不躲猫猫：CSS 带透明桥接条（间隙并入命中区）', () => {
    expect(css).toContain('.scp-history-menu::before')
    // 桥接条必须覆盖「图标底边 → 浮层顶边」整段 10px（header 下 padding 8
    // + 视觉间隙 2）：短了会出现 hover 死区，长了会抢右侧按钮底缘的 hover
    expect(css).toMatch(/\.scp-history-menu::before\s*\{[^}]*top:\s*-10px/)
    expect(css).toMatch(/\.scp-history-menu::before\s*\{[^}]*height:\s*10px/)
    // 浮层贴 header 底沿（+2px，不蹭 1px 下边框）
    expect(css).toMatch(/\.scp-history-menu\s*\{[^}]*top:\s*calc\(100% \+ 2px\)/)
  })

  it('历史浮层不越面板边界：通栏包含块 + 右贴边 + 窄面板宽度兜底 + 纵向实测上限', () => {
    // 横向：包含块必须是 .scp-header（通栏）。挂在 28px 图标 wrap 上时，
    // 260px 浮层左缘探出面板约 26px，被 .scp-panel 的 overflow:hidden 裁掉
    expect(css).toMatch(/\.scp-header\s*\{[^}]*position:\s*relative/)
    // wrap 必须让出包含块：它若留着 position:relative，下面 max-width 的 100%
    // 会解析成 28px（calc 后仅 4px，浮层被压成一条），right 也会贴错边
    expect(css).not.toMatch(/\.scp-history-wrap\s*\{[^}]*position:\s*relative/)
    expect(css).toMatch(/\.scp-history-menu\s*\{[^}]*right:\s*12px/)
    // 面板可拉窄（最小 320px）/ max-width:82%：max-width 兜底不越左边界
    expect(css).toMatch(/\.scp-history-menu\s*\{[^}]*max-width:\s*calc\(100% - 24px\)/)
    // 纵向：条目过多时列表内部滚，浮层不伸出面板底边
    expect(css).toMatch(/\.scp-history-list\s*\{[^}]*overflow-y:\s*auto/)
    const src = read('./SideChatPanel.tsx')
    // 上限按面板剩余空间实测，window resize 时重算、卸载时摘除
    expect(src).toMatch(/addEventListener\('resize',\s*fitHistoryMenu\)/)
    expect(src).toMatch(/removeEventListener\('resize',\s*fitHistoryMenu\)/)
  })

  it('追加消息固定区在输入框上方：CSS 存在、queued 不进列表（保下标）', () => {
    expect(css).toMatch(/\.scp-append-queue\s*\{/)
    const src = read('./SideChatPanel.tsx')
    // 渲染位置：composer 内部首块（消息列表之外，不随滚动走）
    expect(src).toMatch(/scp-composer[\s\S]{0,400}scp-append-queue/)
    // queued 消息在列表里返回 null 而不是 filter：下标 i 是 openSteps 与
    // lastAssistantIdx 的键，过滤会让折叠态 / 操作条整体错位
    expect(src).toMatch(/if \(msg\.queued\) return null/)
  })

  it('失败态工具名与状态点同色（--error）：扫一眼能定位是哪个工具挂了', () => {
    expect(css).toMatch(
      /\.scp-tl-tool\.is-failed \.scp-tl-tool-name\s*\{[^}]*color:\s*var\(--error\)/,
    )
  })

  it('工具调用块：命令完整换行不截断 + 深色字体 + 尾部 meta 承载状态与动态计时', () => {
    // 完整显示：summary 不再单行 ellipsis——路径/命令被截断会埋掉定位信息
    expect(css).toMatch(/\.scp-tl-tool-summary\s*\{[^}]*white-space:\s*pre-wrap/)
    expect(css).toMatch(/\.scp-tl-tool-summary\s*\{[^}]*word-break:\s*break-all/)
    // 深色字体：与思考的弱化色拉开层级（--fg-1 主文本色），名与摘要同色
    expect(css).toMatch(/\.scp-tl-tool-summary\s*\{[^}]*color:\s*var\(--fg-1\)/)
    expect(css).toMatch(/\.scp-tl-tool-name\s*\{[^}]*color:\s*var\(--fg-1\)/)
    expect(css).toMatch(/\.scp-live-tool-summary\s*\{[^}]*white-space:\s*pre-wrap/)
    expect(css).toMatch(/\.scp-live-tool-name\s*\{[^}]*color:\s*var\(--fg-1\)/)
    // 尾部 meta：状态与计时成组跟随块尾；等宽数字防计时跳动
    expect(css).toMatch(/\.scp-tl-meta\s*\{[^}]*flex:\s*1 1 auto/)
    expect(css).toMatch(/\.scp-tl-meta\s*\{[^}]*justify-content:\s*flex-end/)
    expect(css).toMatch(/\.scp-tl-duration\s*\{[^}]*font-variant-numeric:\s*tabular-nums/)
    // 旧的行尾 auto 从 state 上移交 meta：单行布局已退场
    expect(css).not.toMatch(/\.scp-tl-state\s*\{[^}]*margin-left:\s*auto/)
    const src = read('./SideChatPanel.tsx')
    // DOM：meta 包裹 state + duration；在途走 elapsedMs 动态计时
    expect(src).toMatch(/scp-tl-meta/)
    expect(src).toMatch(/elapsedMs\(b\.startedAt, nowMs\)/)
    // 200ms tick 时钟（sending 期间走动，空闲停表）
    expect(src).toMatch(/setInterval\(\(\) => setNowTick/)
  })

  it('流式收起钮进固定高度 sticky 条 + 头部三图标走 MorphIcon 形变包', () => {
    // 条子 sticky + 固定高度：钮相对视口静止，正文在它上方滚（旧病根是钮
    // absolute 在内容区里，内容每长一截钮就往下漂、追随跟不上就滑出视口）
    expect(css).toMatch(/\.scp-live-bar\s*\{[^}]*position:\s*sticky/)
    expect(css).toMatch(/\.scp-live-bar\s*\{[^}]*height:\s*20px/)
    expect(css).not.toMatch(/\.scp-live-collapse\s*\{[^}]*position:\s*absolute/)
    // 内容区底部只留小缝：条子压下时最后一行不贴条边，且不堆叠式浪费留白
    expect(css).toMatch(/\.scp-live-content\s*\{[^}]*padding-bottom:\s*4px/)
    // 回底钮在条子可见时上让，避免右下角两钮重叠
    expect(css).toMatch(/\.scp-jump-bottom\.is-raised\s*\{[^}]*bottom:\s*32px/)
    const src = read('./SideChatPanel.tsx')
    expect(src).toMatch(/is-raised/)
    // 历史 / 分享 / 放大还原：已集成的 morphicons 平滑形变包（放大↔还原是真形变）
    expect(src).toMatch(/import \{ MorphIcon \} from 'morphicons\/react'/)
    expect(src).toMatch(/MorphIcon icon=\{ArchiveIcon\}/)
    expect(src).toMatch(/MorphIcon icon=\{ShareIcon\}/)
    expect(src).toMatch(/MorphIcon\s+icon=\{panelZoomed \? MinimizeIcon : MaximizeIcon\}/)
  })

  it('header 与消息操作一律 IconTip（无原生 title）+ 操作钮纯图标', () => {
    const src = read('./SideChatPanel.tsx')
    // IconTip 组件落地；5 个 header 图标钮显式关掉原生 title——不关就是
    // 「原生灰框 + 自定义气泡」双提示，且原生有约 1s 响应延迟
    expect(src).toMatch(/function IconTip\(/)
    expect(src.match(/nativeTitle=\{false\}/g)?.length).toBe(5)
    // 旧原生 title 全部退场（历史 / 分享 / 收起 / 关闭 / 拖拽）
    for (const gone of [
      "title={t('sideChat.history')}",
      "title={t('sideChat.share')}",
      "title={t('sideChat.minimize')}",
      "title={t('sideChat.close')}",
      "title={t('sideChat.drag')}",
    ]) {
      expect(src).not.toContain(gone)
    }
    // 消息操作钮：纯图标，可访问名 = tip 文案
    expect(src).toMatch(
      /className="scp-msg-action-btn"\s+aria-label=\{t\('sideChat\.sendToMainHint'\)\}/,
    )
    // tips 防裁切：方向按图标位置分（header 在面板顶部→向下；操作条在列表
    // 尾部→向上），宽度内容定宽不贴 28px 图标，左组左对齐
    expect(css).toMatch(/\.scp-tip-bubble\s*\{[^}]*width:\s*max-content/)
    expect(css).toMatch(
      /\.scp-tip\.place-below \.scp-tip-bubble\s*\{[^}]*top:\s*calc\(100% \+ 6px\)/,
    )
    expect(css).toMatch(
      /\.scp-tip\.place-above \.scp-tip-bubble\s*\{[^}]*bottom:\s*calc\(100% \+ 6px\)/,
    )
    expect(css).toMatch(/\.scp-tip\.align-left \.scp-tip-bubble\s*\{[^}]*left:\s*0/)
    // 消息操作条三处（ctx + 两个动作）都是 place="above"；浮层打开时 tip 让位
    expect(src.match(/place="above"/g)?.length).toBe(3)
    expect(src).toContain("text={historyOpen ? '' : t('sideChat.historyTip')}")
    // 操作钮纯图标方形（不再是胶囊文字钮）
    expect(css).toMatch(/\.scp-msg-action-btn\s*\{[^}]*width:\s*24px/)
    expect(css).toMatch(/\.scp-msg-action-btn\s*\{[^}]*border-radius:\s*6px/)
  })

  it('后端四个命令已在 Tauri 注册表登记', () => {
    for (const cmd of [
      'commands::side_chat_send',
      'commands::side_chat_close',
      'commands::side_chat_reset',
      'commands::side_chat_save',
      'commands::side_chat_history_list',
      'commands::side_chat_history_delete',
      'commands::side_chat_remove_append_item',
    ]) {
      expect(mainRs).toContain(cmd)
    }
  })

  it('面板绝对定位浮层：不挤压主对话布局', () => {
    expect(css).toContain('position: absolute')
    expect(css).toContain('.scp-panel')
    // 覆盖在消息区之上，用层级而非改布局
    expect(css).toMatch(/z-index:\s*60/)
    // 移动过后面板禁用进场动画，防最小化恢复时从默认原点滑动的视觉抖动
    expect(css).toMatch(/\.scp-panel\.is-moved\s*\{[^}]*animation:\s*none/)
  })

  it('最小化是整体收起：display none，不是留一条 header', () => {
    expect(css).toMatch(/\.scp-panel\.is-minimized\s*\{[^}]*display:\s*none/)
    // 「只留顶栏」的高度 / 定位特例已移除
    expect(css).not.toMatch(/\.scp-panel\.is-minimized\s*\{[^}]*bottom:\s*auto/)
    // 恢复入口：最小化时左下角浮动按钮带状态角标（is-busy 脉冲 / is-idle 灰点）
    expect(chatPanelSrc).toContain('chat-side-chat-badge')
    expect(chatPanelSrc).toContain('is-busy')
    expect(chatPanelSrc).toContain('is-idle')
  })

  it('空态只有一种对齐方式（全左对齐）+ 标签列固定宽度', () => {
    // 空态容器左对齐，且内部不再有任何居中元素
    expect(css).toMatch(/\.scp-empty\s*\{[^}]*text-align:\s*left/)
    expect(css).not.toMatch(/\.scp-empty-icon\s*\{[^}]*margin:\s*0 auto/)
    expect(css).not.toMatch(/\.scp-empty-title\s*\{[^}]*text-align:\s*center/)
    // 键值对：标签列固定宽度，三条内容共用同一左边界
    expect(css).toMatch(/\.scp-empty-k\s*\{[^}]*flex:\s*0 0 3em/)
    expect(css).toMatch(/\.scp-empty-points li\s*\{[^}]*display:\s*flex/)
  })
})
