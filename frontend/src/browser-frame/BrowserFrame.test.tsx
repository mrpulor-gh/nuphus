/**
 * BrowserFrame —— 独立浏览器窗口的壳页面（44px 控制器）。
 *
 * 覆盖契约（jsdom 不做布局，断言 DOM 结构与调用；几何/真机 IPC 交真机）：
 *   · 自身 label：getCurrentWindow().label（tauri 注入的 currentWindow metadata），
 *     非 Tauri 环境回落 ?window= 查询参数
 *   · 地址栏：回车 / Go → browser_navigate(label, 归一化 URL)；编辑中不被轮询回写
 *   · 前进/后退/刷新：disabled 态由 canGoBack/canGoForward/loading 决定
 *   · 录制开关：clear → record_action(toggle) → get_recording 计数；命令失败回滚开关态
 *   · 标注开关：过 window.__nuphusBrowser 缝（本期 no-op + 日志）
 *   · tab 列表：list_windows 渲染、× → browser_close、点非本窗口窗口降级为刷新列表
 *
 * 轮询用 vi.useFakeTimers 驱动：壳页面拿不到事件（plugin:event|listen 对 browser-*
 * 窗口被 ACL 拒绝，见 BrowserFrame.tsx 头注释），只能轮询。
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BrowserFrame, normalizeUrl, resolveWindowLabel } from './BrowserFrame'

// ── api wrapper 全量 stub（返回 undefined = bridge 的「非 Tauri 环境」语义）──
const apiMocks = vi.hoisted(() => ({
  browserGetState: vi.fn(async () => ({
    url: 'https://example.com',
    title: 'Example Domain',
    loading: false,
    canGoBack: true,
    canGoForward: false,
  })),
  browserNavigate: vi.fn(async () => undefined),
  browserReload: vi.fn(async () => undefined),
  browserClose: vi.fn(async () => undefined),
  browserGoBack: vi.fn(async () => undefined),
  browserGoForward: vi.fn(async () => undefined),
  browserListWindows: vi.fn(async () => [
    { label: 'browser-1', title: 'Example Domain', url: 'https://example.com' },
    { label: 'browser-2', title: 'Rust 官网', url: 'https://www.rust-lang.org' },
  ]),
  browserRecordAction: vi.fn(async () => undefined),
  browserGetRecording: vi.fn(async () => [
    { type: 'record-toggle', on: true },
    { type: 'click', selector: '#go' },
  ]),
  browserClearRecording: vi.fn(async () => undefined),
}))

vi.mock('../main-window/lib/api', () => ({
  browserGetState: apiMocks.browserGetState,
  browserNavigate: apiMocks.browserNavigate,
  browserReload: apiMocks.browserReload,
  browserClose: apiMocks.browserClose,
  browserGoBack: apiMocks.browserGoBack,
  browserGoForward: apiMocks.browserGoForward,
  browserListWindows: apiMocks.browserListWindows,
  browserRecordAction: apiMocks.browserRecordAction,
  browserGetRecording: apiMocks.browserGetRecording,
  browserClearRecording: apiMocks.browserClearRecording,
}))

// 壳页面自身 label 的来源（见组件头注释）：Tauri 模式读 getCurrentWindow().label
const currentWindowLabel = vi.hoisted(() => ({ value: 'browser-1' }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: currentWindowLabel.value }),
}))

/** 渲染壳页面（label 已在 mock 里固定为 browser-1） */
function renderFrame() {
  return render(<BrowserFrame />)
}

const addressInput = () => document.querySelector('input.bf-address') as HTMLInputElement
const backButton = () => screen.getByRole('button', { name: '后退' })
const forwardButton = () => screen.getByRole('button', { name: '前进' })
const reloadButton = () => screen.getByRole('button', { name: '刷新' })
const recordButton = () => screen.getByRole('button', { name: '录制' })
const annotateButton = () => screen.getByRole('button', { name: '标注' })

beforeEach(() => {
  // shouldAdvanceTime：假时钟按真实时间自动前进——waitFor 的内部轮询才不会和
  // 假定时器互掐（否则 waitFor 永远等不到 microtask 落地，全用例超时）。
  // 需要精确推进轮询的用例仍可主动 advanceTimersByTimeAsync。
  vi.useFakeTimers({ shouldAdvanceTime: true })
  vi.clearAllMocks()
  currentWindowLabel.value = 'browser-1'
  // 默认查询串干净：resolveWindowLabel 的回落分支由专门用例覆盖
  window.history.replaceState({}, '', '/browser-frame.html')
})

afterEach(() => {
  vi.useRealTimers()
})

describe('resolveWindowLabel：壳页面自身 label', () => {
  it('Tauri 环境取宿主窗口 label（getCurrentWindow 内部读 tauri 注入的 metadata）', () => {
    expect(resolveWindowLabel()).toBe('browser-1')
  })

  it('非 Tauri 环境回落 ?window= 查询参数（dev 直开 / 单测）', () => {
    currentWindowLabel.value = '' // 模拟 getCurrentWindow().label 为空
    window.history.replaceState({}, '', '/browser-frame.html?window=browser-9')
    expect(resolveWindowLabel()).toBe('browser-9')
  })
})

describe('normalizeUrl（从已删除的 BrowserPanel 迁移覆盖）', () => {
  it('裸域名补 https://，其余原样透传由后端裁决', () => {
    expect(normalizeUrl('example.com')).toBe('https://example.com')
    expect(normalizeUrl('  example.com/path  ')).toBe('https://example.com/path')
    expect(normalizeUrl('http://a.test')).toBe('http://a.test')
    expect(normalizeUrl('https://a.test/x?y=1')).toBe('https://a.test/x?y=1')
    // 安全判定不在前端：危险 scheme 原样交后端拒绝
    expect(normalizeUrl('file:///C:/Windows')).toBe('file:///C:/Windows')
    expect(normalizeUrl('   ')).toBe('')
  })
})

describe('BrowserFrame 壳页面', () => {
  it('挂载即拉自身状态 + tab 列表，按钮态来自 canGoBack/canGoForward', async () => {
    renderFrame()

    await waitFor(() => {
      expect(apiMocks.browserGetState).toHaveBeenCalledWith('browser-1')
      expect(apiMocks.browserListWindows).toHaveBeenCalled()
    })
    // canGoBack=true / canGoForward=false（mock 快照）→ 后退可用、前进置灰
    expect(backButton()).toBeEnabled()
    expect(forwardButton()).toBeDisabled()
    // 地址栏回填权威 URL
    await waitFor(() => expect(addressInput().value).toBe('https://example.com'))
    // tab 列表渲染两个窗口（按标题文本查——非本窗口的 title 属性是 focusPending 提示）
    expect(screen.getByText('Example Domain')).toBeTruthy()
    expect(screen.getByText('Rust 官网')).toBeTruthy()
    expect(document.querySelectorAll('.bf-tab')).toHaveLength(2)
    expect(document.querySelector('.bf-tab.is-own')).not.toBeNull()
  })

  it('地址栏回车 → browser_navigate(label, 归一化 URL)，并乐观回填', async () => {
    renderFrame()
    await waitFor(() => expect(apiMocks.browserGetState).toHaveBeenCalled())

    fireEvent.change(addressInput(), { target: { value: 'rust-lang.org' } })
    fireEvent.keyDown(addressInput(), { key: 'Enter' })

    await waitFor(() =>
      expect(apiMocks.browserNavigate).toHaveBeenCalledWith('browser-1', 'https://rust-lang.org'),
    )
    // 乐观回填：用户立刻看到目标地址（失败时下一轮轮询拉回权威 URL）
    expect(addressInput().value).toBe('https://rust-lang.org')
  })

  it('空输入回车 → 提示「请先输入网址」，不发命令', async () => {
    renderFrame()
    await waitFor(() => expect(addressInput().value).toBe('https://example.com'))

    fireEvent.change(addressInput(), { target: { value: '   ' } })
    fireEvent.keyDown(addressInput(), { key: 'Enter' })

    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('请先输入网址'))
    expect(apiMocks.browserNavigate).not.toHaveBeenCalled()
  })

  it('编辑中不被轮询回写（地址栏草稿与权威 URL 解耦）', async () => {
    // 首帧快照后，后端 URL 变了（模拟页面内跳转）：非编辑态应回写
    const stateMock = apiMocks.browserGetState
    stateMock.mockResolvedValueOnce({
      url: 'https://example.com',
      title: 'Example Domain',
      loading: false,
      canGoBack: true,
      canGoForward: false,
    })
    stateMock.mockResolvedValue({
      url: 'https://example.com/next',
      title: 'Next',
      loading: false,
      canGoBack: true,
      canGoForward: false,
    })

    renderFrame()
    await waitFor(() => expect(addressInput().value).toBe('https://example.com'))

    // 推进一轮 2s 状态轮询 → 权威 URL 变化，非编辑态应回写
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2200)
    })
    await waitFor(() => expect(addressInput().value).toBe('https://example.com/next'))

    // 用户开始编辑 → 再推进一轮轮询也不能覆盖输入
    fireEvent.change(addressInput(), { target: { value: 'https://user-typing.test' } })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2200)
    })
    expect(addressInput().value).toBe('https://user-typing.test')
  })

  it('前进/后退/刷新分别调对应命令', async () => {
    renderFrame()
    await waitFor(() => expect(backButton()).toBeEnabled())

    fireEvent.click(backButton())
    await waitFor(() => expect(apiMocks.browserGoBack).toHaveBeenCalledWith('browser-1'))

    fireEvent.click(reloadButton())
    await waitFor(() => expect(apiMocks.browserReload).toHaveBeenCalledWith('browser-1'))

    // canGoForward=false → 前进按钮置灰，点击不产生调用
    fireEvent.click(forwardButton())
    expect(apiMocks.browserGoForward).not.toHaveBeenCalled()
  })

  it('录制开关：clear → 记 toggle 动作 → 停录取回计数', async () => {
    renderFrame()
    const record = recordButton()

    // 开录
    fireEvent.click(record)
    await waitFor(() => {
      expect(apiMocks.browserClearRecording).toHaveBeenCalledWith('browser-1')
      expect(apiMocks.browserRecordAction).toHaveBeenCalledWith('browser-1', {
        type: 'record-toggle',
        on: true,
        at: expect.any(Number),
      })
    })
    expect(record).toHaveAttribute('aria-pressed', 'true')

    // 停录 → 取回录制流（mock 2 条）并展示在 title
    fireEvent.click(record)
    await waitFor(() => {
      expect(apiMocks.browserGetRecording).toHaveBeenCalledWith('browser-1')
      expect(apiMocks.browserRecordAction).toHaveBeenLastCalledWith('browser-1', {
        type: 'record-toggle',
        on: false,
        at: expect.any(Number),
      })
    })
    expect(record).toHaveAttribute('aria-pressed', 'false')
    expect(record.getAttribute('title')).toContain('已录制 2 条')
  })

  it('录制命令失败 → 开关态回滚 + 错误可见', async () => {
    apiMocks.browserRecordAction.mockRejectedValueOnce(new Error('窗口已关闭'))
    renderFrame()
    const record = recordButton()

    fireEvent.click(record)

    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('窗口已关闭'))
    // 回滚：不能留一个「假装在录」的 UI
    expect(record).toHaveAttribute('aria-pressed', 'false')
  })

  it('标注开关：翻 UI 态 + 过 __nuphusBrowser 缝（本期 no-op）', async () => {
    const infoSpy = vi.spyOn(console, 'info').mockImplementation(() => {})
    renderFrame()
    const annotate = annotateButton()

    fireEvent.click(annotate)
    expect(annotate).toHaveAttribute('aria-pressed', 'true')
    // 缝存在且被调用（no-op 实现会打日志）
    await waitFor(() =>
      expect(infoSpy).toHaveBeenCalledWith(expect.stringContaining('setAnnotate(true)')),
    )
    expect(typeof window.__nuphusBrowser?.setAnnotate).toBe('function')

    fireEvent.click(annotate)
    expect(annotate).toHaveAttribute('aria-pressed', 'false')
    infoSpy.mockRestore()
  })

  it('tab × → browser_close(目标 label) 并刷新列表', async () => {
    renderFrame()
    await waitFor(() => expect(document.querySelectorAll('.bf-tab')).toHaveLength(2))

    // 同步捕获基数：fireEvent 返回时 handler 刚过第一个 await（close 已发、
    // refreshTabs 未发），此时取计数才不会把刷新那次算进基数
    fireEvent.click(screen.getByRole('button', { name: /关闭该标签页：Rust 官网/ }))
    const callsBefore = apiMocks.browserListWindows.mock.calls.length

    await waitFor(() => expect(apiMocks.browserClose).toHaveBeenCalledWith('browser-2'))
    await waitFor(() =>
      expect(apiMocks.browserListWindows.mock.calls.length).toBeGreaterThan(callsBefore),
    )
  })

  it('点非本窗口 tab：降级为刷新列表（聚焦需 Rust 支持，不假装切换）', async () => {
    const infoSpy = vi.spyOn(console, 'info').mockImplementation(() => {})
    renderFrame()
    await waitFor(() => expect(document.querySelectorAll('.bf-tab')).toHaveLength(2))

    // 非本窗口 tab 的 title 是「聚焦待接入」提示，点标题文本即可（事件冒泡到按钮）
    fireEvent.click(screen.getByText('Rust 官网'))

    // 不调用任何窗口命令，只刷新列表 + 明确日志
    await waitFor(() => expect(infoSpy).toHaveBeenCalledWith(expect.stringContaining('browser-2')))
    expect(apiMocks.browserNavigate).not.toHaveBeenCalled()
    // 只有自身窗口被查询（没有去 get_state 别人的窗口）
    expect(apiMocks.browserGetState).toHaveBeenCalledTimes(1)
    infoSpy.mockRestore()
  })

  it('轮询驱动：状态 2s / tab 3s（壳拿不到事件，ACL 拒绝 listen）', async () => {
    renderFrame()
    await waitFor(() => expect(apiMocks.browserGetState).toHaveBeenCalledTimes(1))

    await act(async () => {
      await vi.advanceTimersByTimeAsync(6500)
    })
    // 6.5s 内：state 约 3 次（初始 + 2s + 4s + 6s），tab 约 3 次（初始 + 3s + 6s）
    expect(apiMocks.browserGetState.mock.calls.length).toBeGreaterThanOrEqual(4)
    expect(apiMocks.browserListWindows.mock.calls.length).toBeGreaterThanOrEqual(3)
  })
})
