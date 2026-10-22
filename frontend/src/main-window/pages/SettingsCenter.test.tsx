import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SettingsCenter } from './SettingsCenter'
import { useAppUpdate } from '../hooks/useAppUpdate'
import type { WorkflowItem } from '../../core/types'
import zh from '../../locales/zh'
import en from '../../locales/en'

// 真实子页全部替换为轻量桩：
// ① 本测试只验证「设置中心外壳」——导航分组、分区切换、面板不被关闭、回调透传；
// ② 真实子页会把 IPC / @xyflow/react / motion 等依赖拖进 jsdom。
// 注：画布 / 模型两个分区不在此渲染（宿主分流到 App 层全屏宿主），故无对应桩；
//     二者收在导航最上方的「快捷入口」组，点击即关闭面板走整页宿主。
// 注：主题不是设置中心的分区，而是聊天 header 的非模态外观浮窗
//     （layout/AppearancePanel.tsx）；外观浮窗原来的语言占位 2026-09-28 改成了
//     真正的「语言」分区（系统组，LanguagePage）。头像同年同日并入灵魂页
//     （第二个 Section），不再是独立导航项。
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
/* 灵魂：单模块（显示开关 + 用户 / Nuphus 两条身份行）—— 壳测试只钉「分区切换」，
   模块内容由 SoulPage 自己的单测覆盖 */
vi.mock('./SoulPage', () => ({ SoulPage: () => <div data-testid="page-soul" /> }))
vi.mock('./MobilePage', () => ({ MobilePage: () => <div data-testid="page-mobile" /> }))
vi.mock('./BrowserPage', () => ({ BrowserPage: () => <div data-testid="page-browser" /> }))
/* 主题不在本面板：它走聊天 header 的外观浮窗（layout/AppearancePanel）；
   语言分区在本面板（系统组 → LanguagePage），链路由 LanguagePage 自己的
   测试覆盖，这里只验证分区切换。帮助分区复用 App 层同一 HelpPage（无 props）。 */
vi.mock('./HelpPage', () => ({ HelpPage: () => <div data-testid="page-help" /> }))
vi.mock('./LanguagePage', () => ({ LanguagePage: () => <div data-testid="page-language" /> }))
vi.mock('./ExternalAgentsPage', () => ({
  ExternalAgentsPage: () => <div data-testid="page-external-agents" />,
}))
vi.mock('./SecurityPage', () => ({ SecurityPage: () => <div data-testid="page-security" /> }))
vi.mock('./DataDirsPage', () => ({ DataDirsPage: () => <div data-testid="page-data-dirs" /> }))
vi.mock('./UpdatePage', () => ({ UpdatePage: () => <div data-testid="page-update" /> }))

function renderCenter() {
  const props = {
    onClose: vi.fn(),
    onRunWorkflow: vi.fn(),
    onOpenCanvas: vi.fn(),
    onOpenModels: vi.fn(),
  }
  function Center() {
    return <SettingsCenter {...props} updater={useAppUpdate(false)} />
  }
  render(<Center />)
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
  it('左导航分「快捷入口 / AI 能力 / 连接 / 工作台 / 系统」五组共 19 项，默认落在「灵魂」', async () => {
    renderCenter()

    const items = within(nav()).getAllByRole('button')
    expect(items).toHaveLength(19)

    // 分组顺序：快捷入口（最上）→ AI 能力 → 连接 → 工作台 → 系统
    expect(navGroupTitles()).toEqual(['快捷入口', 'AI 能力', '连接', '工作台', '系统'])
    // 2 + 4 + 3 + 6 + 4 = 19（原 avatars 并入灵魂，help 为系统组新增项）
    expect(navGroupLabels('快捷入口')).toHaveLength(2)
    expect(navGroupLabels('AI 能力')).toHaveLength(4)
    expect(navGroupLabels('连接')).toHaveLength(3)
    expect(navGroupLabels('工作台')).toHaveLength(6)
    expect(navGroupLabels('系统')).toHaveLength(4)

    // 默认分区 = 灵魂（AI 能力组首项，每轮对话都生效的身份设置），右内容随之
    expect(navItem('灵魂')).toHaveAttribute('aria-current', 'page')
    expect(await screen.findByTestId('page-soul')).toBeInTheDocument()
  })

  it('「快捷入口」在 DOM 中先于其余四组，组内顺序为「模型 → 画布」', () => {
    renderCenter()

    const titles = navGroupTitles()
    expect(titles.indexOf('快捷入口')).toBe(0)
    expect(titles.indexOf('快捷入口')).toBeLessThan(titles.indexOf('AI 能力'))
    expect(titles.indexOf('AI 能力')).toBeLessThan(titles.indexOf('连接'))
    expect(titles.indexOf('连接')).toBeLessThan(titles.indexOf('工作台'))
    expect(titles.indexOf('工作台')).toBeLessThan(titles.indexOf('系统'))
    expect(navGroupLabels('快捷入口')).toEqual(['模型', '画布'])
  })

  it('会话分区：点击导航切换右侧内容（项目文件夹折叠上限设置页）', async () => {
    renderCenter()
    await screen.findByTestId('page-soul')

    fireEvent.click(navItem('会话'))
    expect(await screen.findByTestId('page-session-groups')).toBeInTheDocument()
    expect(navItem('会话')).toHaveAttribute('aria-current', 'page')
  })

  it('五组组内顺序逐字钉住（含帮助在语言之后、GitHub 居系统组末位）', async () => {
    const props = renderCenter()

    expect(navGroupLabels('快捷入口')).toEqual(['模型', '画布'])
    // AI 能力组：灵魂居首（默认分区）；头像不再是独立项，已在灵魂页内
    expect(navGroupLabels('AI 能力')).toEqual(['灵魂', '记忆', '技能', '知识库'])
    expect(navGroupLabels('连接')).toEqual(['移动端', '浏览器', 'MCP'])
    expect(navGroupLabels('工作台')).toEqual([
      '会话',
      '工作流',
      '定时任务',
      '外部 Agent',
      '权限与安全',
      '数据目录',
    ])
    // 系统组：语言 → 帮助（新增）→ 版本与更新 → GitHub
    expect(navGroupLabels('系统')).toEqual(['语言', '帮助', '版本与更新', 'GitHub'])

    // 旧名「插件」不再出现在导航里；点击 GitHub 仍落在原 'plugins' 分区（渲染新页面）
    expect(within(nav()).queryByRole('button', { name: '插件' })).toBeNull()
    fireEvent.click(navItem('GitHub'))
    expect(await screen.findByTestId('page-github')).toBeInTheDocument()
    expect(navItem('GitHub')).toHaveAttribute('aria-current', 'page')
    expect(props.onClose).not.toHaveBeenCalled()
  })

  it('帮助分区：面板内嵌渲染 HelpPage，面板保持打开（不调 onClose）', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-soul')

    fireEvent.click(navItem('帮助'))

    expect(await screen.findByTestId('page-help')).toBeInTheDocument()
    expect(navItem('帮助')).toHaveAttribute('aria-current', 'page')
    // 外壳未被关闭：导航与关闭回调都保持原状
    expect(nav()).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '关闭' })).toBeInTheDocument()
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
    // 旧措辞不得残留为导航项文案（「设置」分组标题已随五组重组一并删除）
    expect(within(nav()).queryByRole('button', { name: '设置' })).toBeNull()
  })

  it('点击左侧任一项：右侧切换内容且面板保持打开', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-soul')

    // 默认落在灵魂；切到记忆再切回灵魂，验证切换链路与两块结构
    fireEvent.click(navItem('记忆'))
    expect(await screen.findByTestId('page-memories')).toBeInTheDocument()

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

  it('数据目录分区：弹窗内容区渲染页面，外壳保持打开', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-soul')

    fireEvent.click(navItem('数据目录'))
    expect(await screen.findByTestId('page-data-dirs')).toBeInTheDocument()
    expect(navItem('数据目录')).toHaveAttribute('aria-current', 'page')
    expect(props.onClose).not.toHaveBeenCalled()
  })

  it('宿主分流：点「画布」「模型」交给 App 层全屏宿主，弹窗内容区不动', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-soul')

    fireEvent.click(navItem('画布'))
    expect(props.onOpenCanvas).toHaveBeenCalledWith(null)
    expect(props.onOpenModels).not.toHaveBeenCalled()

    fireEvent.click(navItem('模型'))
    expect(props.onOpenModels).toHaveBeenCalledTimes(1)

    // 两项均不落在弹窗内容区：既不渲染对应子页，也不改变当前分区与外壳
    expect(screen.queryByTestId('page-canvas')).toBeNull()
    expect(screen.queryByTestId('page-models')).toBeNull()
    expect(navItem('灵魂')).toHaveAttribute('aria-current', 'page')
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

  it('主题分区已迁出：导航无「主题与语言」，头像也不再是独立项（已并入灵魂页）', async () => {
    renderCenter()

    // 反断言：themes 分区与它的子页必须都不在（防死链回流）
    expect(within(nav()).queryByRole('button', { name: '主题与语言' })).toBeNull()
    expect(screen.queryByTestId('page-themes')).toBeNull()
    // 头像 2026-09-28 并入灵魂页第二个 Section，不再是独立导航项 / 独立分区
    expect(within(nav()).queryByRole('button', { name: '头像设置' })).toBeNull()
    expect(screen.queryByTestId('page-avatars')).toBeNull()

    // 正断言：灵魂分区承载身份单模块（开关 + 两条身份行）
    fireEvent.click(navItem('灵魂'))
    expect(await screen.findByTestId('page-soul')).toBeInTheDocument()
    expect(navItem('灵魂')).toHaveAttribute('aria-current', 'page')
  })

  it('语言分区：点导航切换右侧 LanguagePage，aria-current 随动', async () => {
    const props = renderCenter()
    await screen.findByTestId('page-soul')

    fireEvent.click(navItem('语言'))
    expect(await screen.findByTestId('page-language')).toBeInTheDocument()
    expect(navItem('语言')).toHaveAttribute('aria-current', 'page')
    expect(navItem('灵魂')).not.toHaveAttribute('aria-current')
    // 切换分区不关闭外壳
    expect(nav()).toBeInTheDocument()
    expect(props.onClose).not.toHaveBeenCalled()
    // 语言在系统组，不在 AI 能力组（防回流到外观浮窗时代的占位）
    expect(navGroupLabels('系统')).toContain('语言')
    expect(navGroupLabels('AI 能力')).not.toContain('语言')
  })

  it('不含「新建会话 / 强制重置 / 贪吃蛇」等非设置项', () => {
    renderCenter()
    for (const label of ['新建会话', '强制重置', '贪吃蛇']) {
      expect(within(nav()).queryByText(label)).toBeNull()
    }
  })
})
