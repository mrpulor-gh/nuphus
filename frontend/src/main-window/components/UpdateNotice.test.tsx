import { StrictMode, useState } from 'react'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { wfGateStatus } from '../lib/api'
import { useAppUpdate } from '../hooks/useAppUpdate'
import { UpdatePage } from '../pages/UpdatePage'
import { UpdateNotice } from './UpdateNotice'

vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '1.0.0') }))
vi.mock('../lib/api', () => ({
  getChangelog: vi.fn(async () => ''),
  wfGateStatus: vi.fn(),
}))

/** 与 App 相同的生命周期：控制器常驻，手动页按弹窗开关挂载。 */
function Harness({
  ready = true,
  stage = 'idle',
}: {
  ready?: boolean
  stage?: 'idle' | 'running' | 'finalizing'
}) {
  const updater = useAppUpdate(ready, stage !== 'idle')
  const [open, setOpen] = useState(false)
  return (
    <>
      <UpdateNotice updater={updater} onOpen={() => setOpen(true)} />
      <button onClick={() => setOpen(true)}>打开更新页</button>
      <button onClick={() => void updater.install()}>直接请求安装</button>
      {open && (
        <div role="dialog">
          <button onClick={() => setOpen(false)}>关闭更新页</button>
          <UpdatePage updater={updater} />
        </div>
      )}
    </>
  )
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function release(version = '1.1.0', body = '新版本说明') {
  const update = {
    version,
    body,
    download: vi.fn(async (_onEvent?: (event: DownloadEvent) => void) => {}),
    install: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  }
  return { ...update, resource: update as unknown as Update }
}

async function advance(ms = 5_000) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms)
  })
}

/** 后端快照与真实执行态接口一致，允许测试 UI 尚未收到新状态的窗口。 */
function execution(stage: 'idle' | 'running' | 'finalizing' = 'idle') {
  return {
    locked: stage !== 'idle',
    reason: stage === 'idle' ? ('idle' as const) : ('agent' as const),
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.resetAllMocks()
  vi.stubGlobal('__TAURI_INTERNALS__', {})
  vi.mocked(check).mockResolvedValue(null)
  vi.mocked(wfGateStatus).mockResolvedValue(execution())
  localStorage.clear()
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('启动更新提示与手动更新共用流程', () => {
  it('主窗口就绪后只检查一次；StrictMode 不重复检查，未点击不下载', async () => {
    const update = release()
    vi.mocked(check).mockResolvedValue(update.resource)
    const view = render(
      <StrictMode>
        <Harness ready={false} />
      </StrictMode>,
    )
    await advance(10_000)
    expect(check).not.toHaveBeenCalled()
    view.rerender(
      <StrictMode>
        <Harness ready />
      </StrictMode>,
    )
    await advance(4_999)
    expect(check).not.toHaveBeenCalled()
    await advance(1)
    expect(check).toHaveBeenCalledExactlyOnceWith({ timeout: 10_000 })
    expect(screen.getByRole('status')).toHaveTextContent('发现新版本 v1.1.0')
    expect(screen.getByRole('button', { name: '下载并安装' })).toBeEnabled()
    await advance(60_000)
    expect(check).toHaveBeenCalledTimes(1)
    expect(update.download).not.toHaveBeenCalled()
    expect(update.install).not.toHaveBeenCalled()
    expect(relaunch).not.toHaveBeenCalled()
  })

  it('普通浏览器预览不会自动调用桌面 updater', async () => {
    vi.unstubAllGlobals()
    render(<Harness />)
    await advance()
    expect(check).not.toHaveBeenCalled()
  })

  it('没有新版时不显示提示，也不自动打开弹窗', async () => {
    render(<Harness />)
    await advance()
    expect(screen.queryByRole('status')).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('预览更新内容：按分组展示说明，弹窗内可直接发起更新', async () => {
    const update = release(
      '1.2.0',
      '## [1.2.0] - 2026-11-01\n\n### Added\n\n- 更新内容预览入口与正文\n',
    )
    vi.mocked(check).mockResolvedValue(update.resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '更新内容' }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('v1.2.0 更新内容')
    expect(within(dialog).getByText('更新内容预览入口与正文')).toBeInTheDocument()
    fireEvent.click(within(dialog).getByRole('button', { name: '下载并安装' }))
    await advance(0)
    expect(update.download).toHaveBeenCalledTimes(1)
  })

  it('预览兜底：说明不是段落格式时按原文展示', async () => {
    const update = release('1.3.0', '简短说明一行')
    vi.mocked(check).mockResolvedValue(update.resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '更新内容' }))
    expect(within(screen.getByRole('dialog')).getByText('简短说明一行')).toBeInTheDocument()
  })

  it('预览空态：该版本未提供更新说明', async () => {
    const update = release('1.4.0', '')
    vi.mocked(check).mockResolvedValue(update.resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '更新内容' }))
    expect(within(screen.getByRole('dialog')).getByText('该版本未提供更新说明')).toBeInTheDocument()
  })

  it('后台检查失败保持安静，手动打开可看到原因并重试', async () => {
    vi.mocked(check).mockRejectedValueOnce(new Error('network timeout'))
    render(<Harness />)
    await advance()
    expect(screen.queryByRole('status')).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.click(screen.getByText('打开更新页'))
    expect(screen.getByText(/network timeout/)).toBeInTheDocument()
    fireEvent.click(screen.getByText('关闭更新页'))
    fireEvent.click(screen.getByText('打开更新页'))
    await advance(0)
    expect(check).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    await advance(0)
    expect(screen.getByText('当前已是最新版本')).toBeInTheDocument()
  })

  it('手动检查先开始时取消自动重复检查，在途检查不能叠加', async () => {
    const pending = deferred<Update | null>()
    vi.mocked(check).mockReturnValue(pending.promise)
    render(<Harness />)
    fireEvent.click(screen.getByText('打开更新页'))
    fireEvent.click(screen.getByRole('button', { name: '检查更新' }))
    await advance()
    expect(check).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: '检查更新' })).toBeDisabled()
    await act(async () => pending.resolve(null))
  })

  it('自动检查进行中打开手动页，会显示相同的检查状态和结果', async () => {
    const pending = deferred<Update | null>()
    const update = release()
    vi.mocked(check).mockReturnValue(pending.promise)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByText('打开更新页'))
    expect(screen.getByText('正在检查官方版本…')).toBeInTheDocument()
    await act(async () => pending.resolve(update.resource))
    expect(screen.getByRole('dialog')).toHaveTextContent('发现新版本 v1.1.0')
    expect(check).toHaveBeenCalledTimes(1)
  })

  it('稍后仅收起当前版本提示，手动入口保留结果；再次检测到不同版本会提醒', async () => {
    const first = release()
    const second = release('1.2.0')
    vi.mocked(check).mockResolvedValueOnce(first.resource).mockResolvedValueOnce(second.resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '稍后' }))
    expect(screen.queryByRole('status')).toBeNull()
    fireEvent.click(screen.getByText('打开更新页'))
    expect(screen.getByText('发现新版本 v1.1.0')).toBeInTheDocument()
    expect(check).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByRole('button', { name: '检查更新' }))
    await advance(0)
    expect(screen.getByRole('status')).toHaveTextContent('发现新版本 v1.2.0')
    expect(first.close).toHaveBeenCalledTimes(1)
    expect(first.download).not.toHaveBeenCalled()
  })

  it('点击后只下载一次，关闭和重开更新页保持进度，下载完成才安装重启', async () => {
    const update = release()
    const download = deferred<void>()
    let progress!: (event: DownloadEvent) => void
    update.download.mockImplementation(onEvent => {
      progress = onEvent!
      return download.promise
    })
    vi.mocked(check).mockResolvedValue(update.resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    expect(update.download).toHaveBeenCalledExactlyOnceWith(expect.any(Function), {
      timeout: 600_000,
    })
    expect(update.install).not.toHaveBeenCalled()
    act(() => {
      progress({ event: 'Started', data: { contentLength: 100 } })
      progress({ event: 'Progress', data: { chunkLength: 35 } })
    })
    fireEvent.click(screen.getByText('关闭更新页'))
    expect(screen.getByRole('status')).toHaveTextContent('35%')
    fireEvent.click(screen.getByRole('button', { name: '查看详情' }))
    expect(screen.getByRole('progressbar')).toHaveAttribute('value', '35')
    expect(screen.queryByRole('button', { name: '下载并安装' })).toBeNull()
    expect(screen.queryByRole('button', { name: '检查更新' })).toBeNull()
    expect(update.download).toHaveBeenCalledTimes(1)
    expect(check).toHaveBeenCalledTimes(1)
    await act(async () => download.resolve())
    expect(update.install).toHaveBeenCalledExactlyOnceWith({ restartAfterInstall: true })
    expect(relaunch).toHaveBeenCalledTimes(1)
  })

  it('下载停滞提示在进度恢复后消失', async () => {
    const update = release()
    const download = deferred<void>()
    let progress!: (event: DownloadEvent) => void
    update.download.mockImplementation(onEvent => {
      progress = onEvent!
      return download.promise
    })
    vi.mocked(check).mockResolvedValue(update.resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    await advance(10_000)
    expect(screen.getByText(/下载进展缓慢或停滞/)).toBeInTheDocument()
    act(() => progress({ event: 'Progress', data: { chunkLength: 20 } }))
    expect(screen.queryByText(/下载进展缓慢或停滞/)).toBeNull()
    await act(async () => download.resolve())
  })

  it('签名校验失败时不安装和重启，保留错误与手动重试入口', async () => {
    const update = release()
    update.download.mockRejectedValue(new Error('minisign: signature verification failed'))
    vi.mocked(check).mockResolvedValue(update.resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    expect(screen.getByText(/安装包签名校验未通过/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重试' })).toBeEnabled()
    expect(update.install).not.toHaveBeenCalled()
    expect(relaunch).not.toHaveBeenCalled()
  })

  it('主窗口卸载时取消未开始的启动检查', async () => {
    const view = render(<Harness />)
    view.unmount()
    await advance()
    expect(check).not.toHaveBeenCalled()
  })

  it('主窗口卸载后检查才返回时，释放更新资源', async () => {
    const pending = deferred<Update | null>()
    const update = release()
    vi.mocked(check).mockReturnValue(pending.promise)
    const view = render(<Harness />)
    await advance()
    view.unmount()
    await act(async () => pending.resolve(update.resource))
    expect(update.close).toHaveBeenCalledTimes(1)
  })

  it('主窗口在下载时卸载，结束后释放资源而不继续安装', async () => {
    const update = release()
    const download = deferred<void>()
    update.download.mockReturnValue(download.promise)
    vi.mocked(check).mockResolvedValue(update.resource)
    const view = render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    view.unmount()
    expect(update.close).not.toHaveBeenCalled()
    await act(async () => download.resolve())
    expect(update.close).toHaveBeenCalledTimes(1)
    expect(update.install).not.toHaveBeenCalled()
  })

  it.each(['running', 'finalizing'] as const)('运行和收尾期间禁止所有入口安装：%s', async stage => {
    const update = release()
    vi.mocked(check).mockResolvedValue(update.resource)
    const view = render(<Harness stage={stage} />)
    await advance()
    expect(screen.getByRole('button', { name: '下载并安装' })).toBeDisabled()
    fireEvent.click(screen.getByText('打开更新页'))
    for (const button of screen.getAllByRole('button', { name: '下载并安装' })) {
      expect(button).toBeDisabled()
    }
    fireEvent.click(screen.getByText('直接请求安装'))
    await advance(0)
    expect(update.download).not.toHaveBeenCalled()
    expect(update.install).not.toHaveBeenCalled()
    expect(relaunch).not.toHaveBeenCalled()
    view.rerender(<Harness />)
    fireEvent.click(screen.getAllByRole('button', { name: '下载并安装' })[0])
    await advance(0)
    expect(update.install).toHaveBeenCalledTimes(1)
  })

  it('下载期间启动任务时保留安装包，任务结束后必须再次点击才安装', async () => {
    const update = release()
    const download = deferred<void>()
    update.download.mockReturnValue(download.promise)
    vi.mocked(check).mockResolvedValue(update.resource)
    const view = render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    view.rerender(<Harness stage="running" />)
    await act(async () => download.resolve())
    expect(update.install).not.toHaveBeenCalled()
    expect(relaunch).not.toHaveBeenCalled()
    for (const button of screen.getAllByRole('button', { name: '安装并重启' })) {
      expect(button).toBeDisabled()
    }
    view.rerender(<Harness />)
    await advance(0)
    expect(update.install).not.toHaveBeenCalled()
    fireEvent.click(screen.getAllByRole('button', { name: '安装并重启' })[0])
    await advance(0)
    expect(update.download).toHaveBeenCalledTimes(1)
    expect(update.install).toHaveBeenCalledTimes(1)
    expect(relaunch).toHaveBeenCalledTimes(1)
  })

  it('界面尚未同步但后端仍忙时，在下载前阻止更新', async () => {
    const update = release()
    vi.mocked(check).mockResolvedValue(update.resource)
    vi.mocked(wfGateStatus).mockResolvedValue(execution('finalizing'))
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    expect(update.download).not.toHaveBeenCalled()
    expect(update.install).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toHaveTextContent('请等待任务执行和保存完成后再安装')
  })

  it('独立工作流运行中也禁止下载和安装', async () => {
    const update = release()
    vi.mocked(check).mockResolvedValue(update.resource)
    vi.mocked(wfGateStatus).mockResolvedValue({ locked: true, reason: 'workflow' })
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    expect(update.download).not.toHaveBeenCalled()
    expect(update.install).not.toHaveBeenCalled()
    expect(relaunch).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toHaveTextContent('请等待任务执行和保存完成后再安装')
  })

  it('下载结束复查发现后端仍忙时，不安装或自动重启', async () => {
    const update = release()
    vi.mocked(check).mockResolvedValue(update.resource)
    vi.mocked(wfGateStatus)
      .mockResolvedValueOnce(execution())
      .mockResolvedValueOnce(execution('running'))
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
    await advance(0)
    expect(update.download).toHaveBeenCalledTimes(1)
    expect(update.install).not.toHaveBeenCalled()
    expect(relaunch).not.toHaveBeenCalled()
    expect(screen.getAllByRole('button', { name: '安装并重启' })).toHaveLength(2)
    fireEvent.click(screen.getByText('关闭更新页'))
    fireEvent.click(screen.getByText('打开更新页'))
    fireEvent.click(screen.getAllByRole('button', { name: '安装并重启' })[1])
    await advance(0)
    expect(update.download).toHaveBeenCalledTimes(1)
    expect(update.install).toHaveBeenCalledTimes(1)
  })

  it.each(['before', 'after'] as const)(
    '无法确认后端状态时安全停止，可重新点击安装：%s',
    async phase => {
      const update = release()
      vi.mocked(check).mockResolvedValue(update.resource)
      if (phase === 'after') vi.mocked(wfGateStatus).mockResolvedValueOnce(execution())
      vi.mocked(wfGateStatus).mockRejectedValueOnce(new Error('IPC unavailable'))
      render(<Harness />)
      await advance()
      fireEvent.click(screen.getByRole('button', { name: '下载并安装' }))
      await advance(0)
      expect(update.download).toHaveBeenCalledTimes(phase === 'before' ? 0 : 1)
      expect(update.install).not.toHaveBeenCalled()
      expect(relaunch).not.toHaveBeenCalled()
      expect(screen.getByRole('dialog')).toHaveTextContent('无法确认任务状态')
      fireEvent.click(screen.getAllByRole('button', { name: /^(下载并安装|安装并重启)$/ })[1])
      await advance(0)
      expect(update.download).toHaveBeenCalledTimes(1)
      expect(update.install).toHaveBeenCalledTimes(1)
    },
  )

  it('稍后的选择跨重启保留，手动页仍可更新，下一个版本重新提示', async () => {
    const update = release()
    vi.mocked(check).mockResolvedValue(update.resource)
    const first = render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '稍后' }))
    first.unmount()
    const reopened = render(<Harness />)
    await advance()
    expect(screen.queryByRole('status')).toBeNull()
    fireEvent.click(screen.getByText('打开更新页'))
    expect(screen.getByRole('button', { name: '下载并安装' })).toBeEnabled()
    reopened.unmount()
    vi.mocked(check).mockResolvedValue(release('1.2.0').resource)
    render(<Harness />)
    await advance()
    expect(screen.getByRole('status')).toHaveTextContent('发现新版本 v1.2.0')
  })

  it('存储不可用时仍能检查和稍后，本次窗口内记住选择', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('storage unavailable')
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('storage unavailable')
    })
    vi.mocked(check).mockResolvedValue(release().resource)
    render(<Harness />)
    await advance()
    fireEvent.click(screen.getByRole('button', { name: '稍后' }))
    expect(screen.queryByRole('status')).toBeNull()
    fireEvent.click(screen.getByText('打开更新页'))
    expect(screen.getByRole('button', { name: '下载并安装' })).toBeEnabled()
  })
})
