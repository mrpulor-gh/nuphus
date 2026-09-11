import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { CanvasWorkbenchPage } from './CanvasWorkbenchPage'

// 三个 tab 页面均改为 lazy 拆包：测试中用轻量桩替换真实模块，
// 避免把工作流画布 / UI 原型画布的依赖（@xyflow/react、motion 等）拖进 jsdom。
vi.mock('../lib/api', () => ({
  listWorkflows: vi.fn(async () => [{ id: 'wf-1', status: 'draft', updated_at: 1 }]),
  wfSave: vi.fn(async () => ({ saved: true })),
}))
vi.mock('../workflow-canvas/CanvasPage', () => ({
  // 透出 workflowId：用于断言「列表选中哪个，画布就打开哪个」
  CanvasPage: ({ workflowId }: { workflowId: string }) => (
    <div data-testid="canvas-page">{workflowId}</div>
  ),
}))
vi.mock('../tools/ToolsPage', () => ({
  ToolsPage: () => <div data-testid="tools-page" />,
}))
vi.mock('../canvases/ui-prototype/UiPrototypeCanvas', () => ({
  UiPrototypeCanvas: () => <div data-testid="prototype-canvas" />,
}))

describe('CanvasWorkbenchPage 首帧反馈与 tab 拆包', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('外壳与加载态在首帧同步可见，不等待 IPC / 子页面 chunk', async () => {
    render(<CanvasWorkbenchPage onClose={() => {}} />)

    // 首帧断言：无 Provider 时 useLanguage 回退到 zh，'common.loading' = '加载中...'
    expect(screen.getByRole('button', { name: 'UI 原型' })).toBeInTheDocument()
    expect(screen.getByText('加载中...')).toBeInTheDocument()

    // 收尾：等在 act 内落地的异步状态，避免 act 警告
    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
  })

  it('工作流画布 chunk 就绪后接管加载态', async () => {
    render(<CanvasWorkbenchPage onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
  })

  it('切换 tab 只挂载对应页面，不残留其它 tab 内容', async () => {
    render(<CanvasWorkbenchPage onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
    expect(screen.queryByTestId('prototype-canvas')).not.toBeInTheDocument()
    expect(screen.queryByTestId('tools-page')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'UI 原型' }))
    await waitFor(() => expect(screen.getByTestId('prototype-canvas')).toBeInTheDocument())
    expect(screen.queryByTestId('canvas-page')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '工具' }))
    await waitFor(() => expect(screen.getByTestId('tools-page')).toBeInTheDocument())
    expect(screen.queryByTestId('prototype-canvas')).not.toBeInTheDocument()
  })

  it('传入 workflowId 时直接编辑该工作流，跳过自选草稿', async () => {
    render(<CanvasWorkbenchPage workflowId="wf-explicit" onClose={() => {}} />)

    // 指定了目标：不应出现自选阶段的加载态
    expect(screen.queryByText('加载中...')).not.toBeInTheDocument()
    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
    expect(screen.getByTestId('canvas-page').textContent).toBe('wf-explicit')

    // 也不应再去查询草稿列表
    const { listWorkflows } = await import('../lib/api')
    expect(listWorkflows).not.toHaveBeenCalled()
  })

  it('未传 workflowId 时回退为自选最近更新的草稿', async () => {
    render(<CanvasWorkbenchPage onClose={() => {}} />)

    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
    expect(screen.getByTestId('canvas-page').textContent).toBe('wf-1')
  })
})
