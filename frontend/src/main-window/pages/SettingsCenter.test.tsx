import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SettingsCenter } from './SettingsCenter'
import type { WorkflowItem } from '../../core/types'

// 15 个真实子页全部替换为轻量桩：
// ① 本测试只验证「设置中心外壳」——导航分组、分区切换、面板不被关闭、回调透传；
// ② 真实子页会把 IPC / @xyflow/react / motion 等依赖拖进 jsdom。
vi.mock('../memories/MemoriesPage', () => ({
  MemoriesPage: () => <div data-testid="page-memories" />,
}))
vi.mock('../workflow/CanvasWorkbenchPage', () => ({
  // 透出 workflowId：用于断言「工作流列表带过来的目标工作流交给了内嵌工作台」
  CanvasWorkbenchPage: ({ workflowId }: { workflowId?: string | null }) => (
    <div data-testid="page-canvas">{workflowId ?? ''}</div>
  ),
}))
vi.mock('../workflow/WorkflowPage', () => ({
  WorkflowPage: ({
    onRunClick,
    onCanvasClick,
  }: {
    onRunClick: (wf: WorkflowItem) => void
    onCanvasClick: (wf: WorkflowItem) => void
  }) => (
    <div data-testid="page-workflows">
      <button type="button" onClick={() => onRunClick({ id: 'wf-1' } as WorkflowItem)}>
        stub-run
      </button>
      <button type="button" onClick={() => onCanvasClick({ id: 'wf-canvas' } as WorkflowItem)}>
        stub-canvas
      </button>
    </div>
  ),
}))
vi.mock('../knowledge/KnowledgePage', () => ({
  KnowledgePage: () => <div data-testid="page-knowledge" />,
}))
vi.mock('./SkillsPage', () => ({ SkillsPage: () => <div data-testid="page-skills" /> }))
vi.mock('./McpPage', () => ({ McpPage: () => <div data-testid="page-mcp" /> }))
vi.mock('./PluginComingSoon', () => ({
  PluginComingSoon: () => <div data-testid="page-plugins" />,
}))
vi.mock('./ModelsPage', () => ({ ModelsPage: () => <div data-testid="page-models" /> }))
vi.mock('./SoulPage', () => ({ SoulPage: () => <div data-testid="page-soul" /> }))
vi.mock('./MobilePage', () => ({ MobilePage: () => <div data-testid="page-mobile" /> }))
vi.mock('./BrowserPage', () => ({ BrowserPage: () => <div data-testid="page-browser" /> }))
vi.mock('./ThemesPage', () => ({
  ThemesPage: ({ showToast }: { showToast: (msg: string) => void }) => (
    <button type="button" onClick={() => showToast('通知')}>
      stub-toast
    </button>
  ),
}))
vi.mock('./ExternalAgentsPage', () => ({
  ExternalAgentsPage: () => <div data-testid="page-external-agents" />,
}))
vi.mock('./SecurityPage', () => ({ SecurityPage: () => <div data-testid="page-security" /> }))
vi.mock('./UpdatePage', () => ({ UpdatePage: () => <div data-testid="page-update" /> }))

function renderCenter() {
  const props = {
    onClose: vi.fn(),
    showToast: vi.fn(),
    onRunWorkflow: vi.fn(),
    onModelChanged: vi.fn(),
  }
  render(<SettingsCenter {...props} />)
  return props
}

const nav = () => screen.getByRole('navigation', { name: '设置' })
const navItem = (label: string) => within(nav()).getByRole('button', { name: label })

describe('SettingsCenter 设置中心外壳', () => {
  it('左导航分「浏览 / 设置 / 管理」三组共 15 项，默认落在「记忆」', async () => {
    renderCenter()

    const items = within(nav()).getAllByRole('button')
    expect(items).toHaveLength(15)

    expect(within(nav()).getByText('浏览')).toBeInTheDocument()
    expect(within(nav()).getByText('设置')).toBeInTheDocument()
    expect(within(nav()).getByText('管理')).toBeInTheDocument()

    expect(navItem('记忆')).toHaveAttribute('aria-current', 'page')
    expect(await screen.findByTestId('page-memories')).toBeInTheDocument()
  })

  it('点击左侧任一项：右侧切换内容且面板保持打开', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-memories')

    fireEvent.click(navItem('模型'))

    expect(await screen.findByTestId('page-models')).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByTestId('page-memories')).not.toBeInTheDocument())
    // 外壳未被关闭：导航 + 右上角关闭按钮仍在，关闭回调未被触发
    expect(nav()).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '关闭' })).toBeInTheDocument()
    expect(props.onClose).not.toHaveBeenCalled()
    expect(navItem('模型')).toHaveAttribute('aria-current', 'page')
    expect(navItem('记忆')).not.toHaveAttribute('aria-current')
  })

  it('右上角关闭按钮 → 返回聊天（onClose）', () => {
    const props = renderCenter()
    fireEvent.click(screen.getByRole('button', { name: '关闭' }))
    expect(props.onClose).toHaveBeenCalledTimes(1)
  })

  it('工作流分区：行内「运行」委托给宿主 onRunWorkflow（弹窗与退出由宿主决定）', async () => {
    const props = renderCenter()
    fireEvent.click(navItem('工作流'))
    fireEvent.click(
      await within(await screen.findByTestId('page-workflows')).findByText('stub-run'),
    )

    expect(props.onRunWorkflow).toHaveBeenCalledTimes(1)
    expect(props.onRunWorkflow).toHaveBeenCalledWith({ id: 'wf-1' })
  })

  it('工作流分区：行内「画布」切到画布分区，并带上目标工作流', async () => {
    renderCenter()
    fireEvent.click(navItem('工作流'))
    fireEvent.click(
      await within(await screen.findByTestId('page-workflows')).findByText('stub-canvas'),
    )

    expect(await screen.findByTestId('page-canvas')).toHaveTextContent('wf-canvas')
    expect(navItem('画布')).toHaveAttribute('aria-current', 'page')
  })

  it('主题分区：showToast 通道透传给子页', async () => {
    const props = renderCenter()
    fireEvent.click(navItem('主题与语言'))
    fireEvent.click(await screen.findByText('stub-toast'))

    expect(props.showToast).toHaveBeenCalledWith('通知')
  })

  it('不含「新建会话 / 强制重置 / 贪吃蛇」等非设置项', () => {
    renderCenter()
    for (const label of ['新建会话', '强制重置', '贪吃蛇']) {
      expect(within(nav()).queryByText(label)).toBeNull()
    }
  })
})
