/**
 * 外观浮窗（AppearancePanel）契约测试。
 *
 * 这个浮窗的形态约束有两条，测试逐条钉住：
 * ① **不挡主界面**：非模态浮层（无全屏遮罩 / 无 aria-modal），Esc 与点面板外都能收起，
 *    点面板内不误收；
 * ② **开合不丢未保存调整**：收起是 `hidden`（保活不卸载），不是条件渲染 ——
 *    关闭再打开，本地草稿（行内改名输入等）还在。这条正是旧 ThemesPage 被
 *    CompactModal 关一次即卸载所丢的东西，是本任务核心收益的回归钉。
 *
 * ③ **一页流，不翻页**（2026-09-28 重做）：面板内不得再出现 Tab 并列、
 *    手风琴折叠（details/summary）、按并列条件渲染的操作条，也不得出现
 *    「选择页 / 自定义页」两页互切 —— 全部内容按优先级排在同一条纵向流里，
 *    管理型低频动作收进「⋯ 更多」浮层。「放弃修改」是唯一被允许的条件按钮
 *    （仅有无保存调整时出现，规格明文）。
 *
 * ④ **说人话**：渲染文本里禁止出现 `--` 开头的 token 名（反断言钉死）。
 */
import { act, fireEvent, render, screen, waitFor, type RenderResult } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AppearancePanel } from './AppearancePanel'
import { ThemeProvider } from '../../hooks/useTheme'
import { markOpacityIntent, OPACITY_COLOR_KEYS } from '../../hooks/customTheme'
import { __setThumbEncoder } from '../../ui/assetUrl'

// 轻提示通道：jsdom 无 Tauri IPC，桩掉（HUD/island 分流不在本测试范围）
vi.mock('../../ui/islandChannel', () => ({ showAppFeedback: vi.fn() }))

// api wrapper 全量 stub：函数一律替换为返回 undefined 的 vi.fn（getLanguage 等）
vi.mock('../lib/api', async importOriginal => {
  const actual = await importOriginal<Record<string, unknown>>()
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(actual)) {
    out[key] = typeof value === 'function' ? vi.fn(async () => undefined) : value
  }
  return out
})
vi.mock('../lib/localImage', () => ({ pickAndImportImage: vi.fn(async () => null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
// jsdom 无 Tauri 运行时：convertFileSrc 原样返回路径（asset:// 通道判失败），
// invoke('read_image_base64') 走 assetUrl 的 blob 兜底 —— 原图字节因此能取到
// （缩略图链路的解码输入），与生产链路同一实现、同一判定。
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: { imagePath?: string }) =>
    cmd === 'read_image_base64' && args?.imagePath
      ? { base64: 'QUJD', mime: 'image/png' }
      : undefined,
  ),
  convertFileSrc: (p: string) => p,
}))

/** 浮层容器与分区标题（查询器集中一处，改结构只改这里） */
const panel = (r: RenderResult) => r.container.querySelector('.apnp-panel') as HTMLElement
const zoneTitles = (r: RenderResult) =>
  Array.from(r.container.querySelectorAll('.apnp-zone-title')).map(el => el.textContent?.trim())
const presetCards = (r: RenderResult) =>
  Array.from(r.container.querySelectorAll('.apnp-preset-grid .apnp-preset-card'))
/** 主题卡（不含首张固定「＋ 新建」卡） */
const themeCards = (r: RenderResult) =>
  Array.from(
    r.container.querySelectorAll('.apnp-theme-grid .apnp-theme-card:not(.apnp-theme-card-new)'),
  )
const newCard = (r: RenderResult) =>
  r.container.querySelector('.apnp-theme-card-new') as HTMLElement
const moreMenu = () => document.querySelector('.apnp-more-menu')

/** 预置一个「我的主题」条目（useTheme 的存储契约：themes 列表 + 激活 id） */
function seedCustomTheme(name = '深夜蓝调') {
  const theme = {
    name,
    base: 'dark',
    overrides: { '--accent': '#3b82f6', '--surface-0': '#0d1b2a' },
  }
  localStorage.setItem('nuphus_custom_themes', JSON.stringify([{ ...theme, id: 'ct-seed-1' }]))
  localStorage.setItem('nuphus_custom_active', 'ct-seed-1')
}

/** 读回持久化的主题列表 */
function storedThemes(): Array<{
  id?: string
  name: string
  base: string
  overrides: Record<string, string>
  skin?: string
}> {
  return JSON.parse(localStorage.getItem('nuphus_custom_themes') || '[]')
}

/** 已广播的皮肤变更序列（null = 清除）；jsdom 里路径经 blob 兜底解析，故有图时为 blob: */
let skinBroadcasts: Array<string | null> = []
function onSkinChange(e: Event) {
  skinBroadcasts.push((e as CustomEvent<{ url: string | null }>).detail?.url ?? null)
}

async function renderPanel(open = true) {
  const onClose = vi.fn()
  // act 内 await：挂载期有异步落实（皮肤等），等它落账再断言，
  // 否则 React 会报 "update not wrapped in act"
  let view!: RenderResult
  await act(async () => {
    view = render(
      <ThemeProvider>
        <AppearancePanel open={open} onClose={onClose} />
      </ThemeProvider>,
    )
  })
  return { ...view, onClose }
}

describe('AppearancePanel 外观浮窗', () => {
  beforeEach(() => {
    localStorage.clear()
    skinBroadcasts = []
    window.addEventListener('nuphus-skin-changed', onSkinChange)
  })
  afterEach(() => {
    window.removeEventListener('nuphus-skin-changed', onSkinChange)
    __setThumbEncoder(null)
    vi.clearAllMocks()
  })

  it('渲染为非模态浮层：标题「外观」、无 aria-modal、无遮罩', async () => {
    const view = await renderPanel()

    const dialog = screen.getByRole('dialog', { name: '外观' })
    expect(dialog).toBe(panel(view))
    // 关键：不是模态 —— 没有 aria-modal，也没有全屏遮罩
    expect(dialog).not.toHaveAttribute('aria-modal')
    expect(view.container.querySelector('.apnp-backdrop')).toBeNull()
    expect(panel(view)).not.toHaveAttribute('hidden')
  })

  it('一页流分区自上而下：系统主题 → 我的主题 → 界面不透明度 → 颜色微调', async () => {
    const view = await renderPanel()

    expect(zoneTitles(view)).toEqual(['系统主题', '我的主题', '界面不透明度', '颜色微调'])
  })

  it('皮肤背景栏已移除：换背景只走「我的主题」卡面的上传钮', async () => {
    const view = await renderPanel()

    // 反断言：独立的皮肤背景区（预览条 + 更换/清除按钮）不得再出现
    expect(zoneTitles(view)).not.toContain('皮肤背景')
    expect(view.container.querySelector('.skin-preview')).toBeNull()
    expect(panel(view).textContent).not.toContain('选择背景图片')
    expect(panel(view).textContent).not.toContain('更换背景')
    // 正断言：每张主题卡的媒体区都有上传钮（hover 浮出）
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([{ id: 'ct-a', name: '海蓝', base: 'dark', overrides: {} }]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const seeded = await renderPanel()
    expect(themeCards(seeded)[0].querySelector('.apnp-theme-upload')?.textContent).toBe('上传')
  })

  it('废形态钉死：无 Tab 并列 / 无手风琴折叠 / 无条件操作条 / 无两页互切', async () => {
    const view = await renderPanel()

    // 内容区里不得出现 role=tablist（Tab 并列的官方语义）
    expect(view.container.querySelectorAll('[role="tablist"]')).toHaveLength(0)
    // 不得出现折叠件（手风琴形态）
    expect(view.container.querySelector('details')).toBeNull()
    expect(view.container.querySelector('summary')).toBeNull()
    // 不得出现「按条件渲染的底部操作条」（保存按钮常驻，不靠条件浮出）
    expect(
      view.container.querySelector('.apnp-dock, .apnp-action-bar, [class*="action-bar"]'),
    ).toBeNull()
    // 全部内容同一棵树：不存在「切换后才出现」的第二页
    expect(view.container.querySelectorAll('.apnp-preset-card').length).toBeGreaterThan(0)
    expect(view.container.querySelectorAll('.color-token-picker').length).toBeGreaterThan(0)
    expect(view.container.querySelectorAll('.opacity-range').length).toBeGreaterThan(0)
  })

  it('渲染文本禁止出现 -- token 名（反断言）', async () => {
    const view = await renderPanel()

    expect(panel(view).textContent).not.toMatch(/--[a-z]/)
  })

  it('预设卡三张横排，当前主题卡带 active 态与勾选标（默认 dark）', async () => {
    const view = await renderPanel()

    const cards = presetCards(view)
    expect(cards).toHaveLength(3)
    expect(cards.map(c => c.querySelector('.apnp-preset-name')?.textContent)).toEqual([
      '深邃蓝黑',
      '简白',
      '深渊冷光',
    ])
    // active 唯一且落在当前基底上
    const active = cards.filter(c => c.classList.contains('active'))
    expect(active).toHaveLength(1)
    expect(active[0]).toBe(cards[0])
    expect(active[0].querySelector('.apnp-preset-check')).toBeInTheDocument()
    expect(cards[1].querySelector('.apnp-preset-check')).toBeNull()
    // desc 文案移入 title tooltip（380px 窄窗放不下）
    expect(cards[0]).toHaveAttribute('title', expect.stringContaining('深色主题'))
  })

  it('点击预设卡即时切换基底（active 态与勾选标随动）', async () => {
    const view = await renderPanel()

    const cards = presetCards(view)
    fireEvent.click(cards[2])

    expect(cards[0].classList.contains('active')).toBe(false)
    expect(cards[2].classList.contains('active')).toBe(true)
    expect(cards[2].querySelector('.apnp-preset-check')).toBeInTheDocument()
    // 基底落盘（全局链路不被浮窗化改变）
    expect(localStorage.getItem('nuphus_theme')).toBe('tech')
  })

  it('挂着自定义主题时点预设卡：背景同步离开主题快照（回到系统预设态）', async () => {
    localStorage.setItem('nuphus_skin_bg', 'C:/img/global.png')
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        { id: 'ct-a', name: '海蓝', base: 'dark', overrides: {}, skin: 'C:/img/theme.png' },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()
    skinBroadcasts = []

    await act(async () => {
      fireEvent.click(presetCards(view)[2])
    })

    // 回预设态 → 重新应用 LS_SKIN（解析出可渲染 URL，而不是清空、也不是主题那张）
    expect(skinBroadcasts).toHaveLength(1)
    expect(skinBroadcasts[0]).toMatch(/^blob:/)
  })

  it('「我的主题」空态只显示「＋ 新建」卡（无空列表提示句）', async () => {
    const view = await renderPanel()

    expect(themeCards(view)).toHaveLength(0)
    expect(newCard(view)).toBeInTheDocument()
    expect(panel(view).textContent).not.toContain('保存自定义调整后出现在这里')
  })

  it('「我的主题」是卡片网格：卡面缩略图 + 卡底名称/铅笔 + 删除钮 + 激活高亮', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        { id: 'ct-a', name: '海蓝', base: 'dark', overrides: { '--surface-0': '#0d1b2a' } },
        { id: 'ct-b', name: '暖灰', base: 'light', overrides: {} },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()

    const cards = themeCards(view)
    expect(cards).toHaveLength(2)
    // 展示层最新在前：ct-b（后保存）在前，ct-a 在后
    expect(cards.map(c => c.querySelector('.apnp-theme-name')?.textContent)).toEqual([
      '暖灰',
      '海蓝',
    ])
    // 激活项唯一高亮 + 勾选标（激活的是 ct-a，落在第 2 张）
    const active = cards.filter(c => c.classList.contains('active'))
    expect(active).toHaveLength(1)
    expect(active[0]).toBe(cards[1])
    expect(active[0].querySelector('.apnp-theme-check')).toBeInTheDocument()
    // 每张卡都有：卡面激活钮、hover 上传钮、删除小钮、铅笔
    expect(cards[0].querySelector('.apnp-theme-face')).toBeInTheDocument()
    expect(cards[0].querySelector('.apnp-theme-upload')).toBeInTheDocument()
    expect(view.container.querySelectorAll('.apnp-theme-delete')).toHaveLength(2)
    expect(view.container.querySelectorAll('.apnp-theme-edit')).toHaveLength(2)
    // 无皮肤 → 卡面用该主题色板兜底：ct-b 走内置基底 light，ct-a 用 surface-0 覆盖值
    expect((cards[0].querySelector('.apnp-theme-thumb') as HTMLElement).style.background).toBe(
      'rgb(240, 244, 248)',
    )
    expect((cards[1].querySelector('.apnp-theme-thumb') as HTMLElement).style.background).toBe(
      'rgb(13, 27, 42)',
    )
    // 「＋ 新建」卡固定在网格末位（自定义主题之后）
    const grid = view.container.querySelector('.apnp-theme-grid') as HTMLElement
    expect(grid.children[grid.children.length - 1]).toBe(newCard(view))
  })

  it('点「＋ 新建」：以当前系统主题 + 当前微调预览值建条，新卡进网格首位并行内改名', async () => {
    const view = await renderPanel()

    fireEvent.click(newCard(view))

    const stored = storedThemes()
    expect(stored).toHaveLength(1)
    expect(stored[0].name).toBe('未命名主题')
    expect(stored[0].base).toBe('dark')
    expect(stored[0].overrides).toEqual({})
    // 落盘即激活
    expect(localStorage.getItem('nuphus_custom_active')).toBe(stored[0].id)
    // 新卡出现在网格首位（第一张主题卡）并进入行内改名态
    const cards = themeCards(view)
    expect(cards).toHaveLength(1)
    const renameInput = cards[0].querySelector('.apnp-theme-rename') as HTMLInputElement
    expect(renameInput).toBeInTheDocument()
    expect(renameInput.value).toBe('未命名主题')

    // 改名 → 回车落盘
    fireEvent.change(renameInput, { target: { value: '夜航' } })
    fireEvent.keyDown(renameInput, { key: 'Enter' })
    expect(themeCards(view)[0].querySelector('.apnp-theme-rename')).toBeNull()
    expect(storedThemes()[0].name).toBe('夜航')
  })

  it('保存（未编辑激活主题）= 新建链路：新卡进入行内改名态', async () => {
    const view = await renderPanel()

    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    const stored = storedThemes()
    expect(stored).toHaveLength(1)
    expect(stored[0].name).toBe('未命名主题')
    expect(themeCards(view)[0].querySelector('.apnp-theme-rename')).toBeInTheDocument()
  })

  it('铅笔行内改名：回车 / 失焦都落盘，空输入保留原名', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([{ id: 'ct-a', name: '海蓝', base: 'dark', overrides: {} }]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()

    fireEvent.click(view.container.querySelector('.apnp-theme-edit') as HTMLElement)
    const input = () => themeCards(view)[0].querySelector('.apnp-theme-rename') as HTMLInputElement
    expect(input().value).toBe('海蓝')

    // 回车落盘
    fireEvent.change(input(), { target: { value: '深海' } })
    fireEvent.keyDown(input(), { key: 'Enter' })
    expect(storedThemes()[0].name).toBe('深海')
    expect(themeCards(view)[0].querySelector('.apnp-theme-name')?.textContent).toBe('深海')

    // 再开一次 → 失焦落盘
    fireEvent.click(view.container.querySelector('.apnp-theme-edit') as HTMLElement)
    fireEvent.change(input(), { target: { value: '夜海' } })
    fireEvent.blur(input())
    expect(storedThemes()[0].name).toBe('夜海')

    // 空输入 → 保留原名
    fireEvent.click(view.container.querySelector('.apnp-theme-edit') as HTMLElement)
    fireEvent.change(input(), { target: { value: '' } })
    fireEvent.blur(input())
    expect(storedThemes()[0].name).toBe('夜海')
  })

  it('卡面上传：写入该主题皮肤快照并落盘 + 全局背景切到该图 + 卡面缩略图立即刷新', async () => {
    const { pickAndImportImage } = await import('../lib/localImage')
    vi.mocked(pickAndImportImage).mockResolvedValue('C:/img/skin-a.png')
    // jsdom 无 canvas：注入缩略图编码器替身（生产路径见 assetUrl.encodeThumbWithCanvas）
    __setThumbEncoder(async () => 'blob:thumb-320')
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([{ id: 'ct-a', name: '海蓝', base: 'dark', overrides: {} }]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()
    skinBroadcasts = []
    // 上传前：无皮肤 → 色板兜底，没有缩略图
    expect(
      themeCards(view)[0]
        .querySelector('.apnp-theme-thumb')
        ?.classList.contains('apnp-theme-thumb--img'),
    ).toBe(false)

    await act(async () => {
      fireEvent.click(themeCards(view)[0].querySelector('.apnp-theme-upload') as HTMLElement)
    })

    // 落盘：该条的 skin 快照
    const stored = storedThemes()
    expect(stored).toHaveLength(1)
    expect(stored[0].skin).toBe('C:/img/skin-a.png')
    // 全局背景同步（广播原图的可渲染 URL，非清除）
    expect(skinBroadcasts).toHaveLength(1)
    expect(skinBroadcasts[0]).toMatch(/^blob:/)
    // 卡面缩略图随即刷新成缩小版（不是原图）
    await waitFor(() => {
      const thumb = themeCards(view)[0].querySelector('.apnp-theme-thumb') as HTMLElement
      expect(thumb.classList.contains('apnp-theme-thumb--img')).toBe(true)
      expect(thumb.style.backgroundImage).toContain('blob:thumb-320')
    })
  })

  it('卡面缩略图：渲染的是 canvas 缩小版（编码器产出），原图只作解码输入', async () => {
    __setThumbEncoder(async () => 'blob:thumb-320')
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        { id: 'ct-a', name: '海蓝', base: 'dark', overrides: {}, skin: 'C:/img/skin-a.png' },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()

    await waitFor(() => {
      const thumb = themeCards(view)[0].querySelector('.apnp-theme-thumb') as HTMLElement
      expect(thumb.classList.contains('apnp-theme-thumb--img')).toBe(true)
      expect(thumb.style.backgroundImage).toContain('blob:thumb-320')
    })

    // 原图路径只以「读字节」入参出现（解码输入），渲染出来的是缩略图
    const core = await import('@tauri-apps/api/core')
    const calls = vi.mocked(core.invoke).mock.calls
    expect(calls).toContainEqual(['read_image_base64', { imagePath: 'C:/img/skin-a.png' }])
  })

  it('点卡面激活主题：落盘激活 id + 按该主题皮肤快照恢复全局背景', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        { id: 'ct-a', name: '海蓝', base: 'dark', overrides: {}, skin: 'C:/img/skin-a.png' },
        { id: 'ct-b', name: '暖灰', base: 'light', overrides: {} },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()
    skinBroadcasts = []

    // 展示层 newest-first：ct-b 在前
    await act(async () => {
      fireEvent.click(themeCards(view)[0].querySelector('.apnp-theme-face') as HTMLElement)
    })

    expect(localStorage.getItem('nuphus_custom_active')).toBe('ct-b')
    // 无皮肤主题 = 清全局皮肤（广播 null）
    expect(skinBroadcasts).toEqual([null])

    // 再切回 ct-a（有皮肤）→ 背景回到它的快照（blob URL，非空）
    await act(async () => {
      fireEvent.click(themeCards(view)[1].querySelector('.apnp-theme-face') as HTMLElement)
    })
    expect(localStorage.getItem('nuphus_custom_active')).toBe('ct-a')
    expect(skinBroadcasts).toHaveLength(2)
    expect(skinBroadcasts[1]).toMatch(/^blob:/)
  })

  it('删除主题卡：confirm 后移除并落盘；取消则不动', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        { id: 'ct-a', name: '海蓝', base: 'dark', overrides: {} },
        { id: 'ct-b', name: '暖灰', base: 'light', overrides: {} },
      ]),
    )
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false)
    const view = await renderPanel()

    // 取消 → 不删
    fireEvent.click(view.container.querySelectorAll('.apnp-theme-delete')[0])
    expect(storedThemes()).toHaveLength(2)
    confirmSpy.mockReturnValue(true)

    fireEvent.click(view.container.querySelectorAll('.apnp-theme-delete')[0])
    expect(storedThemes()).toHaveLength(1)
    expect(themeCards(view)).toHaveLength(1)
  })

  it('保存按钮文案「保存」，与放弃修改紧凑同行；放弃修改仅有无保存调整时出现', async () => {
    const view = await renderPanel()

    // 主按钮文案就是「保存」两字
    const save = screen.getByRole('button', { name: '保存' })
    expect(save).toBeInTheDocument()
    // 紧凑同行：保存 + （可能的放弃）+ ⋯更多 在同一行控件里
    const row = view.container.querySelector('.apnp-save-control') as HTMLElement
    expect(row).toContainElement(save)
    expect(row.querySelector('.apnp-more-trigger')).toBeInTheDocument()
    // 「保存」是行内最后一个子元素（贴最右缘，防回归）
    expect(row.lastElementChild).toBe(save)
    // 无未保存调整 → 没有「放弃修改」
    expect(screen.queryByRole('button', { name: '放弃修改' })).toBeNull()

    // 造成未保存调整（改一个核心色 → 进入预览态），「放弃修改」随之出现
    const picker = view.container.querySelectorAll('.color-token-picker')[0]
    fireEvent.change(picker, { target: { value: '#ff0000' } })
    const discard = screen.getByRole('button', { name: '放弃修改' })
    expect(row).toContainElement(discard)

    // 点放弃 → 清预览草稿，按钮随之消失
    fireEvent.click(discard)
    expect(screen.queryByRole('button', { name: '放弃修改' })).toBeNull()
  })

  it('「⋯ 更多」：点开浮现导入/导出/停用（放弃修改不在菜单里）', async () => {
    seedCustomTheme()
    const view = await renderPanel()

    // 默认收起，浮层不在文档里
    expect(moreMenu()).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: /更多/ }))
    expect(moreMenu()).toBeInTheDocument()
    const items = () =>
      Array.from(moreMenu()!.querySelectorAll('.apnp-more-item')).map(el => el.textContent)

    // 造成未保存调整后仍是这三项（放弃修改只在紧凑保存行）
    const bubble = view.container.querySelectorAll('.opacity-range')[0]
    fireEvent.change(bubble, { target: { value: '60' } })
    expect(items()).toEqual(['导入 JSON', '导出 JSON', '停用自定义'])
  })

  it('停用自定义：回纯内置基底，背景回 LS_SKIN', async () => {
    localStorage.setItem('nuphus_skin_bg', 'C:/img/global.png')
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        { id: 'ct-a', name: '海蓝', base: 'dark', overrides: {}, skin: 'C:/img/skin-a.png' },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()
    skinBroadcasts = []

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /更多/ }))
    })
    await act(async () => {
      fireEvent.click(screen.getByRole('menuitem', { name: '停用自定义' }))
    })

    expect(localStorage.getItem('nuphus_custom_active')).toBeNull()
    // 列表保留，卡片仍在
    expect(themeCards(view)).toHaveLength(1)
    // 背景回到系统预设态（LS_SKIN 解析出的 URL，而不是被清空）
    expect(skinBroadcasts).toHaveLength(1)
    expect(skinBroadcasts[0]).toMatch(/^blob:/)
  })

  it('保存（编辑激活主题）：更新该条，名称与皮肤快照原样保留', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        {
          id: 'ct-a',
          name: '海蓝',
          base: 'dark',
          overrides: { '--surface-0': '#0d1b2a' },
          skin: 'C:/img/skin-a.png',
        },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()

    // 改一个核心色 → 进入预览态 → 保存
    const pickers = view.container.querySelectorAll('.color-token-picker')
    fireEvent.change(pickers[0], { target: { value: '#ff0000' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    const stored = storedThemes()
    expect(stored).toHaveLength(1)
    expect(stored[0].id).toBe('ct-a')
    expect(stored[0].name).toBe('海蓝')
    expect(stored[0].skin).toBe('C:/img/skin-a.png')
    expect(stored[0].overrides['--accent']).toBe('#ff0000')
    // 更新而非新建：没有第二条
    expect(storedThemes()[1]).toBeUndefined()
  })

  it('界面不透明度区 5 个滑块，标签即对象名（气泡/输入框/控制面板/弹窗/背景图）', async () => {
    const view = await renderPanel()

    const sliders = Array.from(view.container.querySelectorAll('.opacity-range'))
    expect(sliders).toHaveLength(5)
    expect(sliders.map(s => s.getAttribute('aria-label'))).toEqual([
      '消息气泡',
      '输入框',
      '控制面板',
      '弹窗',
      '背景图',
    ])
    // 皮肤滑块 min=0（0% = 隐藏背景图），其余保持 20 下限
    expect(sliders[4]).toHaveAttribute('min', '0')
    expect(sliders[0]).toHaveAttribute('min', '20')
  })

  it('颜色微调区 5 项全保留，标签是人话（名称 + 括号内效果说明）', async () => {
    const view = await renderPanel()

    expect(view.container.querySelectorAll('.color-token-picker')).toHaveLength(5)
    const labels = Array.from(view.container.querySelectorAll('.color-token-label')).map(
      el => el.textContent,
    )
    expect(labels).toEqual([
      '强调色（按钮·链接·选中态）',
      '窗口底色（全局背景的基础色）',
      '浮层底色（面板·弹窗的底）',
      '主要文字（正文颜色）',
      '次要文字（说明·占位符）',
    ])
    // 面板里不得再有 token 名小注，也没有基底只读行 / 未保存徽标
    expect(view.container.querySelectorAll('.color-token-key')).toHaveLength(0)
    expect(view.container.querySelector('.custom-base-label')).toBeNull()
    expect(view.container.querySelector('.custom-unsaved-badge')).toBeNull()
    // 名称输入框已改为卡片行内改名
    expect(view.container.querySelector('.custom-name-input')).toBeNull()
  })

  it('语言区已从浮窗移除（迁设置中心）', async () => {
    const view = await renderPanel()

    expect(zoneTitles(view)).not.toContain('界面语言')
    expect(view.container.querySelector('.segmented')).toBeNull()
    expect(panel(view).textContent).not.toContain('中文')
  })

  it('Esc 收起浮窗', async () => {
    const view = await renderPanel()

    fireEvent.keyDown(document, { key: 'Escape' })

    expect(view.onClose).toHaveBeenCalledTimes(1)
  })

  it('点面板外收起；点面板内不误收', async () => {
    const view = await renderPanel()

    fireEvent.mouseDown(document.body)
    expect(view.onClose).toHaveBeenCalledTimes(1)

    fireEvent.mouseDown(panel(view))
    expect(view.onClose).toHaveBeenCalledTimes(1)
  })

  it('收起是保活不是卸载：内容仍在 DOM，只摘掉浮层', async () => {
    const view = await renderPanel()

    view.rerender(
      <ThemeProvider>
        <AppearancePanel open={false} onClose={view.onClose} />
      </ThemeProvider>,
    )

    expect(panel(view)).toHaveAttribute('hidden')
    // 内容与本地 state 都没丢：卡片网格、滑块、颜色行都还在树上
    expect(zoneTitles(view)).toHaveLength(4)
    expect(view.container.querySelector('.apnp-theme-grid')).toBeInTheDocument()
    expect(newCard(view)).toBeInTheDocument()
    expect(view.container.querySelectorAll('.opacity-range')).toHaveLength(5)
  })

  it('【核心收益】关闭再打开，未落盘的行内改名草稿不丢', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([{ id: 'ct-a', name: '海蓝', base: 'dark', overrides: {} }]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-a')
    const view = await renderPanel()

    // 点铅笔 → 改名（先不回车，草稿只在本地 state）
    fireEvent.click(view.container.querySelector('.apnp-theme-edit') as HTMLElement)
    const draft = () => themeCards(view)[0].querySelector('.apnp-theme-rename') as HTMLInputElement
    fireEvent.change(draft(), { target: { value: '深夜蓝调' } })

    // 收起 → 重新展开（浮窗常驻，不卸载）
    view.rerender(
      <ThemeProvider>
        <AppearancePanel open={false} onClose={view.onClose} />
      </ThemeProvider>,
    )
    view.rerender(
      <ThemeProvider>
        <AppearancePanel open onClose={view.onClose} />
      </ThemeProvider>,
    )

    expect(panel(view)).not.toHaveAttribute('hidden')
    // 改名输入框与未落盘草稿都还在（旧 ThemesPage 关一次即卸载，丢的就是这个）
    expect(draft().value).toBe('深夜蓝调')
  })

  it('Esc 先收「⋯ 更多」菜单，再收浮窗（不把两者一起关）', async () => {
    const view = await renderPanel()

    fireEvent.click(screen.getByRole('button', { name: /更多/ }))
    expect(moreMenu()).toBeInTheDocument()

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(moreMenu()).toBeNull()
    expect(view.onClose).not.toHaveBeenCalled()

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(view.onClose).toHaveBeenCalledTimes(1)
  })
})

/* ═══════════════════════════════════════════════════════════════
   系统主题切换的实时跟随（大王真机 bug 回归组）

   症状：外观浮窗内切系统主题后，「自定义」区（5 颜色行 + 5 滑块）不刷新 ——
   首切显示新主题值，再切回旧主题却永远停在上一套。
   契约：系统预设态（无自定义激活、无预览草稿）下，微调区显示值必须实时
   等于当前系统主题的实际生效值，切任何系统卡都立即跟随。
   ═══════════════════════════════════════════════════════════════ */

/**
 * 三套系统主题的 token「计算值」夹具。
 *
 * jsdom 不加载 tokens.css：`getComputedStyle().getPropertyValue('--accent')`
 * 恒返回空串，ThemeProvider 写 data-theme 在 jsdom 里不带任何解析结果。
 * 这里按 data-theme 仿真浏览器对 :root / [data-theme] 的解析（内联样式优先，
 * 与真实层级一致）。核心色取 tokens.css 真实值；面板 / 弹窗 / 皮肤的 α 刻意
 * 取三主题互异的值 —— 真实 CSS 里三主题 α 相同（modal .8 / panel .9 / skin .35），
 * 若照抄，「滑块读数停在上一套主题」这类冻结回归将完全测不出来。
 */
const THEME_TOKENS: Record<string, Record<string, string>> = {
  dark: {
    '--accent': '#3b82f6',
    '--surface-0': '#0a0a10',
    '--surface-1': '#12121a',
    '--fg-1': '#f5f5fa',
    '--fg-2': '#d0d0dd',
    '--msg-user-bg': '#152238',
    '--msg-assistant-bg': '#12141e',
    '--input-bg': '#12141e',
    '--panel-bg': 'rgba(14, 14, 22, 0.9)',
    '--modal-bg': 'rgba(14, 14, 22, 0.8)',
    '--skin-bg-opacity': '0.35',
  },
  light: {
    '--accent': '#111111',
    '--surface-0': '#ffffff',
    '--surface-1': '#f7f7f8',
    '--fg-1': '#1a1a1a',
    '--fg-2': '#444444',
    '--msg-user-bg': '#f0f0f2',
    '--msg-assistant-bg': '#ffffff',
    '--input-bg': '#12141e',
    '--panel-bg': 'rgba(251, 251, 252, 0.85)',
    '--modal-bg': 'rgba(251, 251, 252, 0.75)',
    '--skin-bg-opacity': '0.4',
  },
  tech: {
    '--accent': '#7c6ff7',
    '--surface-0': '#020408',
    '--surface-1': '#060810',
    '--fg-1': '#eeeff4',
    '--fg-2': '#c4c8d4',
    '--msg-user-bg': '#0c1020',
    '--msg-assistant-bg': '#060810',
    '--input-bg': '#080a14',
    '--panel-bg': 'rgba(6, 8, 16, 0.95)',
    '--modal-bg': 'rgba(6, 8, 16, 0.85)',
    '--skin-bg-opacity': '0.3',
  },
}

/** 各主题下微调区应有的显示值（= 该主题 token 的实际生效值） */
const EXPECTED: Record<string, { colors: string[]; sliders: number[] }> = {
  dark: {
    colors: ['#3b82f6', '#0a0a10', '#12121a', '#f5f5fa', '#d0d0dd'],
    sliders: [100, 100, 90, 80, 35],
  },
  light: {
    colors: ['#111111', '#ffffff', '#f7f7f8', '#1a1a1a', '#444444'],
    sliders: [100, 100, 85, 75, 40],
  },
  tech: {
    colors: ['#7c6ff7', '#020408', '#060810', '#eeeff4', '#c4c8d4'],
    sliders: [100, 100, 95, 85, 30],
  },
}

/** 5 个核心色 hex 行当前显示值（顺序同 CORE_TOKEN_KEYS） */
const colorTokenValues = (r: RenderResult): string[] =>
  Array.from(r.container.querySelectorAll<HTMLInputElement>('.color-token-hex')).map(el => el.value)

/** 5 个滑块当前读数（顺序：气泡 → 输入框 → 面板 → 弹窗 → 背景图） */
const opacitySliderValues = (r: RenderResult): number[] =>
  Array.from(r.container.querySelectorAll<HTMLInputElement>('.opacity-range')).map(el =>
    Number(el.value),
  )

/** 切到指定系统预设卡（index 0=dark 1=light 2=tech） */
async function switchPreset(r: RenderResult, index: number) {
  await act(async () => {
    fireEvent.click(presetCards(r)[index])
  })
}

/** 把 jsdom 的 getComputedStyle 换成「按 data-theme 解析 token 夹具」的版本 */
function stubThemeComputedStyle() {
  const realGetComputedStyle = window.getComputedStyle.bind(window)
  vi.stubGlobal('getComputedStyle', (el: Element, pseudo?: string | null) => {
    const cs = realGetComputedStyle(el, pseudo)
    const theme = document.documentElement.getAttribute('data-theme') ?? 'dark'
    const tokens = THEME_TOKENS[theme] ?? THEME_TOKENS.dark
    const inline = document.documentElement.style
    // 内联覆盖（useTheme.applyOverrides 写入）优先，回落该主题的 token 计算值
    cs.getPropertyValue = (name: string) =>
      inline.getPropertyValue(name).trim() || tokens[name] || ''
    return cs
  })
}

describe('系统主题切换：自定义区实时跟随当前基底', () => {
  beforeEach(() => {
    localStorage.clear()
    // ThemeProvider 的覆盖应用是全局的：清掉上个用例可能残留的内联 token 与 data-theme
    document.documentElement.style.cssText = ''
    document.documentElement.removeAttribute('data-theme')
    stubThemeComputedStyle()
  })
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('dark → 简白 → dark → tech：颜色区与滑块每步都等于当前主题的生效值', async () => {
    const view = await renderPanel()

    // 初始（dark）
    expect(colorTokenValues(view)).toEqual(EXPECTED.dark.colors)
    expect(opacitySliderValues(view)).toEqual(EXPECTED.dark.sliders)

    await switchPreset(view, 1) // → 简白
    expect(colorTokenValues(view)).toEqual(EXPECTED.light.colors)
    expect(opacitySliderValues(view)).toEqual(EXPECTED.light.sliders)

    await switchPreset(view, 0) // → 回 dark（不得停在上一套）
    expect(colorTokenValues(view)).toEqual(EXPECTED.dark.colors)
    expect(opacitySliderValues(view)).toEqual(EXPECTED.dark.sliders)

    await switchPreset(view, 2) // → tech（第三主题同样立即跟随）
    expect(colorTokenValues(view)).toEqual(EXPECTED.tech.colors)
    expect(opacitySliderValues(view)).toEqual(EXPECTED.tech.sliders)
  })

  it('纯净预设态：切系统主题不产生预览草稿（无「放弃修改」、无内联覆盖残留）', async () => {
    const view = await renderPanel()

    await switchPreset(view, 1)
    await switchPreset(view, 2)

    expect(screen.queryByRole('button', { name: '放弃修改' })).toBeNull()
    for (const key of OPACITY_COLOR_KEYS) {
      expect(document.documentElement.style.getPropertyValue(key)).toBe('')
    }
    expect(document.documentElement.getAttribute('data-theme')).toBe('tech')
  })

  it('拖过气泡滑块后切系统主题：按新基底 computed α 重解析，不停在拖动值', async () => {
    const view = await renderPanel()

    // dark 下把气泡拖到 60% → 生成预览草稿（--msg-*-bg 写 rgba 覆盖）
    fireEvent.change(view.container.querySelectorAll('.opacity-range')[0], {
      target: { value: '60' },
    })
    expect(opacitySliderValues(view)[0]).toBe(60)
    expect(screen.getByRole('button', { name: '放弃修改' })).toBeInTheDocument()

    // 点系统预设卡 = 回纯净预设态：草稿清空，滑块按新基底（简白）computed 重解析
    await switchPreset(view, 1)
    expect(opacitySliderValues(view)).toEqual(EXPECTED.light.sliders)
    expect(screen.queryByRole('button', { name: '放弃修改' })).toBeNull()
    expect(document.documentElement.style.getPropertyValue('--msg-user-bg')).toBe('')

    // 再切 tech 仍跟随（持久 dirty 意图 ≠ 覆盖可跨主题残留）
    await switchPreset(view, 2)
    expect(opacitySliderValues(view)).toEqual(EXPECTED.tech.sliders)
  })

  it('已保存自定义主题激活态：自身 overrides 优先；切系统预设卡后回落基底', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        { id: 'ct-ov', name: '海蓝', base: 'dark', overrides: { '--surface-0': '#0d1b2a' } },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-ov')
    const view = await renderPanel()

    // 自定义激活态：覆盖优先（surface-0 读覆盖值，其余回落 dark 基底）
    expect(colorTokenValues(view)).toEqual(['#3b82f6', '#0d1b2a', '#12121a', '#f5f5fa', '#d0d0dd'])
    expect(opacitySliderValues(view)).toEqual(EXPECTED.dark.sliders)

    // 切到简白 = 回系统预设态：自定义覆盖不跨主题残留，全部回落 light 基底
    await switchPreset(view, 1)
    expect(colorTokenValues(view)).toEqual(EXPECTED.light.colors)
    expect(opacitySliderValues(view)).toEqual(EXPECTED.light.sliders)
  })

  it('dirty 通道跨基底重派走预览通道：激活另一基底的已存主题时按新基底色重派生', async () => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([
        {
          id: 'ct-src',
          name: '源',
          base: 'dark',
          overrides: { '--msg-user-bg': 'rgba(21, 34, 56, 0.6)' },
        },
        { id: 'ct-dst', name: '目标', base: 'light', overrides: { '--surface-0': '#ffffff' } },
      ]),
    )
    localStorage.setItem('nuphus_custom_active', 'ct-src')
    // 用户拖过气泡通道（持久 dirty 意图，customTheme.readOpacityIntent）
    markOpacityIntent('bubbles')
    const view = await renderPanel()

    // 源主题激活态：滑块读自身覆盖（60%）
    expect(opacitySliderValues(view)[0]).toBe(60)

    // 激活另一基底的已存主题（dark → light）：dirty 通道按新基底颜色重派，
    // 产物走预览通道（「放弃修改」随之出现），而不是覆写基底显示值
    await act(async () => {
      fireEvent.click(themeCards(view)[0].querySelector('.apnp-theme-face') as HTMLElement)
    })

    expect(screen.getByRole('button', { name: '放弃修改' })).toBeInTheDocument()
    expect(document.documentElement.style.getPropertyValue('--msg-user-bg')).toBe(
      'rgba(240, 240, 242, 0.6)',
    )
    expect(document.documentElement.style.getPropertyValue('--msg-assistant-bg')).toBe(
      'rgba(255, 255, 255, 0.6)',
    )
    // 颜色区：生效覆盖优先（ct-dst 的 surface-0），其余回落 light 基底（非 #000000）
    expect(colorTokenValues(view)).toEqual(EXPECTED.light.colors)
  })
})
