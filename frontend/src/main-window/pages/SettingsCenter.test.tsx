import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SettingsCenter } from './SettingsCenter'
import type { WorkflowItem } from '../../core/types'
import zh from '../../locales/zh'
import en from '../../locales/en'

// 真实子页全部替换为轻量桩：
// ① 本测试只验证「设置中心外壳」——导航分组、分区切换、面板不被关闭、回调透传；
// ② 真实子页会把 IPC / @xyflow/react / motion 等依赖拖进 jsdom。
// 注：画布 / 模型两个分区不在此渲染（宿主分流到 App 层全屏宿主），故无对应桩；
//     二者收在导航最上方的「快捷入口」组，点击即关闭面板走整页宿主。
vi.mock('../memories/MemoriesPage', () => ({
  MemoriesPage: () => <div data-testid="page-memories" />,
}))
vi.mock('../workflow/WorkflowPage', () => ({
  WorkflowPage: ({
    onRunClick,
    onCanvasClick,
    scheduleDialogLayer,
  }: {
    onRunClick: (wf: WorkflowItem) => void
    onCanvasClick: (wf: WorkflowItem) => void
    scheduleDialogLayer?: 'default' | 'settings'
  }) => (
    <div data-testid="page-workflows" data-schedule-dialog-layer={scheduleDialogLayer}>
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
vi.mock('./GithubPage', () => ({
  GithubPage: () => <div data-testid="page-github" />,
}))
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
    onOpenCanvas: vi.fn(),
    onOpenModels: vi.fn(),
  }
  render(<SettingsCenter {...props} />)
  return props
}

/** 导航 Aria 与面板标题、聊天头部齿轮共用 app.settings → 措辞统一为「控制面板」 */
const nav = () => screen.getByRole('navigation', { name: '控制面板' })
const navItem = (label: string) => within(nav()).getByRole('button', { name: label })

/** 导航分组标题（按 DOM 顺序） */
const navGroupTitles = () =>
  Array.from(nav().querySelectorAll('.settings-center-nav-group-title')).map(el => el.textContent)

/** 指定分组内的导航项文案（按 DOM 顺序） */
const navGroupLabels = (title: string) => {
  const group = Array.from(nav().querySelectorAll('.settings-center-nav-group')).find(
    el => el.querySelector('.settings-center-nav-group-title')?.textContent === title,
  )
  if (!group) throw new Error(`导航分组不存在：${title}`)
  return Array.from(group.querySelectorAll('.settings-center-nav-label')).map(el => el.textContent)
}

describe('SettingsCenter 设置中心外壳', () => {
  it('左导航分「快捷入口 / 浏览 / 设置 / 管理」四组共 17 项，默认落在「记忆」', async () => {
    renderCenter()

    const items = within(nav()).getAllByRole('button')
    expect(items).toHaveLength(17)

    // 分组顺序：快捷入口（最上）→ 浏览 → 设置 → 管理
    expect(navGroupTitles()).toEqual(['快捷入口', '浏览', '设置', '管理'])
    // 2 + 6 + 6 + 3 = 17（浏览组新增定时任务中心）
    expect(navGroupLabels('快捷入口')).toHaveLength(2)
    expect(navGroupLabels('浏览')).toHaveLength(6)
    expect(navGroupLabels('设置')).toHaveLength(6)
    expect(navGroupLabels('管理')).toHaveLength(3)

    // 默认分区仍是「记忆」，右内容不变
    expect(navItem('记忆')).toHaveAttribute('aria-current', 'page')
    expect(await screen.findByTestId('page-memories')).toBeInTheDocument()
  })

  it('「快捷入口」在 DOM 中先于「浏览」，组内顺序为「模型 → 画布」', () => {
    renderCenter()

    const titles = navGroupTitles()
    expect(titles.indexOf('快捷入口')).toBe(0)
    expect(titles.indexOf('快捷入口')).toBeLessThan(titles.indexOf('浏览'))
    expect(navGroupLabels('快捷入口')).toEqual(['模型', '画布'])
  })

  it('会话工作台分区：点击导航切换右侧内容（项目文件夹折叠上限设置页）', async () => {
    renderCenter()
    await screen.findByTestId('page-memories')

    fireEvent.click(navItem('会话工作台'))
    expect(await screen.findByTestId('page-session-groups')).toBeInTheDocument()
    expect(navItem('会话工作台')).toHaveAttribute('aria-current', 'page')
  })

  it('定时任务位于工作流之后，其余分组顺序保持不变', async () => {
    const props = renderCenter()

    expect(navGroupLabels('浏览')).toEqual(['记忆', '工作流', '定时任务', '技能', '知识库', 'MCP'])
    expect(navGroupLabels('设置')).toEqual([
      '灵魂',
      '移动端',
      '浏览器',
      '主题与语言',
      '会话工作台',
      '外部 Agent',
    ])
    // 管理组：GitHub 与「版本与更新」相邻（权限与安全 → GitHub → 版本与更新）
    expect(navGroupLabels('管理')).toEqual(['权限与安全', 'GitHub', '版本与更新'])

    // 旧名「插件」不再出现在导航里；点击 GitHub 仍落在原 'plugins' 分区（渲染新页面）
    expect(within(nav()).queryByRole('button', { name: '插件' })).toBeNull()
    fireEvent.click(navItem('GitHub'))
    expect(await screen.findByTestId('page-github')).toBeInTheDocument()
    expect(navItem('GitHub')).toHaveAttribute('aria-current', 'page')
    expect(props.onClose).not.toHaveBeenCalled()
  })

  it('「模型」「画布」带外链标识与「在整页打开」提示，其余项无', () => {
    renderCenter()

    for (const label of ['模型', '画布']) {
      const item = navItem(label)
      expect(item).toHaveClass('settings-center-nav-item-hosted')
      expect(item).toHaveAttribute('title', '在整页打开')
      expect(item.querySelector('.settings-center-nav-external')).not.toBeNull()
    }

    // 面板内直接打开的项：不带宿主标识，也不带「在整页打开」提示
    for (const label of ['记忆', '灵魂', '版本与更新']) {
      const item = navItem(label)
      expect(item).not.toHaveClass('settings-center-nav-item-hosted')
      expect(item).not.toHaveAttribute('title')
      expect(item.querySelector('.settings-center-nav-external')).toBeNull()
    }
  })

  it('入口文案与面板/导航一致：「控制面板」（EN: Control Panel），无「设置」残留', () => {
    // 聊天头部齿轮 ChatPanel 的 aria-label / title 与面板标题（.settings-center-title）、
    // dialog aria-label、nav aria-label 共用 app.settings 这一个 key：
    // 键值即四处措辞（齿轮无轻量挂载路径，故此处断言键值 + 面板侧渲染结果）
    expect(zh['app.settings']).toBe('控制面板')
    expect(en['app.settings']).toBe('Control Panel')

    renderCenter()
    const dialog = screen.getByRole('dialog', { name: '控制面板' })
    expect(dialog.querySelector('.settings-center-title')).toHaveTextContent('控制面板')
    expect(nav()).toHaveAccessibleName('控制面板')
    // 旧措辞不得残留为导航项文案（分组标题「设置」是 cmd.category.settings，属正当分组名）
    expect(within(nav()).queryByRole('button', { name: '设置' })).toBeNull()
  })

  it('点击左侧任一项：右侧切换内容且面板保持打开', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-memories')

    fireEvent.click(navItem('灵魂'))

    expect(await screen.findByTestId('page-soul')).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByTestId('page-memories')).not.toBeInTheDocument())
    // 外壳未被关闭：导航 + 右上角关闭按钮仍在，关闭回调未被触发
    expect(nav()).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '关闭' })).toBeInTheDocument()
    expect(props.onClose).not.toHaveBeenCalled()
    expect(navItem('灵魂')).toHaveAttribute('aria-current', 'page')
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

  it('工作流分区：定时设置弹窗使用高于控制面板的层级', async () => {
    renderCenter()
    fireEvent.click(navItem('工作流'))

    expect(await screen.findByTestId('page-workflows')).toHaveAttribute(
      'data-schedule-dialog-layer',
      'settings',
    )
  })

  it('工作流分区：行内「画布」委托宿主全屏打开，并带上目标工作流', async () => {
    const props = renderCenter()
    fireEvent.click(navItem('工作流'))
    fireEvent.click(
      await within(await screen.findByTestId('page-workflows')).findByText('stub-canvas'),
    )

    expect(props.onOpenCanvas).toHaveBeenCalledWith('wf-canvas')
    // 弹窗内不渲染画布，分区也不切换（关闭面板与打开全屏宿主由宿主决定）
    expect(screen.queryByTestId('page-canvas')).toBeNull()
    expect(navItem('工作流')).toHaveAttribute('aria-current', 'page')
  })

  it('宿主分流：点「画布」「模型」交给 App 层全屏宿主，弹窗内容区不动', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-memories')

    fireEvent.click(navItem('画布'))
    expect(props.onOpenCanvas).toHaveBeenCalledWith(null)
    expect(props.onOpenModels).not.toHaveBeenCalled()

    fireEvent.click(navItem('模型'))
    expect(props.onOpenModels).toHaveBeenCalledTimes(1)

    // 两项均不落在弹窗内容区：既不渲染对应子页，也不改变当前分区与外壳
    expect(screen.queryByTestId('page-canvas')).toBeNull()
    expect(screen.queryByTestId('page-models')).toBeNull()
    expect(navItem('记忆')).toHaveAttribute('aria-current', 'page')
    expect(props.onClose).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('焦点陷阱：打开后焦点在面板内，Tab / Shift+Tab 在面板内循环', async () => {
    renderCenter()
    const panel = screen.getByRole('dialog')
    await waitFor(() => expect(panel.contains(document.activeElement)).toBe(true))

    const close = screen.getByRole('button', { name: '关闭' })
    const buttons = within(panel).getAllByRole('button')
    const last = buttons[buttons.length - 1]

    // 焦点在面板本体（初始态）→ Tab 落到首个可聚焦元素
    fireEvent.keyDown(panel, { key: 'Tab' })
    expect(document.activeElement).toBe(close)

    // 末项 Tab → 回绕到首项
    last.focus()
    fireEvent.keyDown(panel, { key: 'Tab' })
    expect(document.activeElement).toBe(close)

    // 首项 Shift+Tab → 回绕到末项
    close.focus()
    fireEvent.keyDown(panel, { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(last)
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
