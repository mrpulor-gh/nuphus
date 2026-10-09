import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DesktopToolbar } from './DesktopToolbar'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

describe('user-selected desktop application registration', () => {
  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
  })
  afterEach(cleanup)

  it('opens the host picker without a model-provided path and reports the registered name', async () => {
    invoke.mockResolvedValue({ name: 'Portable Editor', app_ref: 'app:local' })
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    expect(await screen.findByText('应用已登记')).toBeTruthy()
    expect(screen.getByText(/Portable Editor/)).toBeTruthy()
    expect(invoke).toHaveBeenCalledExactlyOnceWith('desktop_register_application', undefined)
  })

  it('cancelling the native picker neither launches an application nor reports success', async () => {
    invoke.mockResolvedValue(null)
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '登记应用' })).not.toBeDisabled())
    expect(screen.queryByText('应用已登记')).toBeNull()
    expect(invoke).toHaveBeenCalledTimes(1)
  })
})

/**
 * 大王 2026-10-09 定稿（两轮）：
 * ① 桌面工具条 = **右侧固定按钮列 + hover 从上到下逐次显形**，常驻渲染。
 *    Ctrl+U / visible / onClose 废弃；不可拖拽；**无置顶**；**无把手、无竖条包裹层**。
 * ③ 坐标读数 + 关闭是**常驻分组**（不受整列 hover 显隐约束），点关闭才收 ——
 *    钉死 2026-10-09 报障「点击后坐标位置获取不到」不得复发。
 * ② 反向钉死「被删掉的东西不得复活」——这不是「暂时没测」。
 */
describe('DesktopToolbar 常驻按钮列结构（废弃 Ctrl+U / 废弃拖拽 / 废弃置顶 / 废弃竖条）', () => {
  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
  })
  afterEach(cleanup)

  const dock = () => document.querySelector('.desktop-toolbar-dock') as HTMLElement
  /** 按钮列 = dock 的直接子节点中带 .desktop-toolbar-btn 的那些（顺序 = 视觉从上到下）。
   *  不能按 tagName===BUTTON 取：坐标读数 .desktop-toolbar-pos 也是 BUTTON 且同为
   *  dock 的直接子节点，但它不是列成员且不吃错峰。 */
  const column = () =>
    Array.from(dock().children).filter(el =>
      el.classList.contains('desktop-toolbar-btn'),
    ) as HTMLElement[]
  const delays = () => column().map(el => el.style.getPropertyValue('--stagger-delay'))

  it('无 props 即可渲染：按钮列直接挂在 dock 上，不再有把手 / 竖条两层', () => {
    render(<DesktopToolbar />)
    expect(dock()).toBeTruthy()
    expect(document.querySelector('.desktop-toolbar-handle')).toBeNull()
    expect(document.querySelector('.desktop-toolbar')).toBeNull()
    // 每一个按钮的父节点就是 dock 本身
    for (const btn of column()) expect(btn.parentElement).toBe(dock())
  })

  it('按钮列顺序：截图 → 选区 → 鼠标 → 取色 → 字典 → 登记应用（DOM 序 = 视觉从上到下）', () => {
    render(<DesktopToolbar />)
    expect(column().map(el => el.getAttribute('aria-label'))).toEqual([
      '截图',
      '选区',
      '鼠标',
      '取色',
      '字典',
      '登记应用',
    ])
  })

  it('置顶已整体移除：无置顶按钮、不落盘 pinned 键', async () => {
    render(<DesktopToolbar />)
    expect(screen.queryByRole('button', { name: '固定窗口置顶' })).toBeNull()
    expect(screen.queryByRole('button', { name: '取消置顶' })).toBeNull()
    // 即使旧 localStorage 里还留着 pinned=true，也不应有任何置顶入口消费它
    localStorage.setItem('desktop_toolbar_pinned', 'true')
    fireEvent.click(screen.getByRole('button', { name: '截图' }))
    await waitFor(() => expect(invoke).toHaveBeenCalled())
    // 命令名已从后端整体删除（2026-10-09），这里改为「不得有任何置顶类窗口控制调用」
    for (const [cmd] of invoke.mock.calls) expect(String(cmd)).not.toMatch(/topmost/i)
    expect(localStorage.getItem('desktop_toolbar_pinned')).toBe('true')
  })

  it('分隔符已随置顶一并移除（列内不再有 divider）', () => {
    render(<DesktopToolbar />)
    expect(document.querySelector('.desktop-toolbar-divider')).toBeNull()
  })

  it('无 .panel-grip 拖拽把手（大王明令不可拖动）', () => {
    render(<DesktopToolbar />)
    expect(document.querySelector('.panel-grip')).toBeNull()
  })

  it('dock 无 inline 定位：left / top 均为空串（定位全部交给 CSS）', () => {
    render(<DesktopToolbar />)
    expect(dock().style.left).toBe('')
    expect(dock().style.top).toBe('')
  })

  it('旧存储键 desktop_toolbar_pos 被忽略（预置坐标也不产生 inline 定位）', () => {
    localStorage.setItem('desktop_toolbar_pos', JSON.stringify({ x: 333, y: 444 }))
    render(<DesktopToolbar />)
    expect(dock().style.left).toBe('')
    expect(dock().style.top).toBe('')
  })

  it('在按钮列上按下并拖动，既不改位置也不落盘', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseDown(dock(), { clientX: 50, clientY: 70 })
    fireEvent.mouseMove(window, { clientX: 900, clientY: 900 })
    fireEvent.mouseUp(window)
    expect(dock().style.left).toBe('')
    expect(dock().style.top).toBe('')
    expect(localStorage.getItem('desktop_toolbar_pos')).toBeNull()
  })

  it('全文档 [title] 不含 Ctrl+U（快捷键确已废弃）', () => {
    render(<DesktopToolbar />)
    for (const node of Array.from(document.querySelectorAll('[title]'))) {
      expect(node.getAttribute('title')).not.toMatch(/Ctrl\s*\+\s*U/i)
    }
  })

  it('卸载重挂后列结构不变（无 visible 开关导致的条件渲染）', () => {
    const first = render(<DesktopToolbar />)
    expect(column()).toHaveLength(6)
    first.unmount()
    render(<DesktopToolbar />)
    expect(column()).toHaveLength(6)
    expect(screen.getByRole('button', { name: '截图' })).toBeTruthy()
  })
})

describe('DesktopToolbar hover 错峰：从上到下逐次显示（CSS 变量，无 JS 定时器）', () => {
  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
  })
  afterEach(cleanup)

  const dock = () => document.querySelector('.desktop-toolbar-dock') as HTMLElement
  const column = () =>
    Array.from(dock().children).filter(el =>
      el.classList.contains('desktop-toolbar-btn'),
    ) as HTMLElement[]

  it('第 i 个按钮的 --stagger-delay 等于 i × 步长（首个 0ms，末位 200ms）', () => {
    render(<DesktopToolbar />)
    const d = column().map(el => el.style.getPropertyValue('--stagger-delay'))
    expect(d).toEqual(['0ms', '40ms', '80ms', '120ms', '160ms', '200ms'])
  })

  it('错峰严格递增且步长恒为 40ms（逐项相减，不是"大概是 40"）', () => {
    render(<DesktopToolbar />)
    const ms = column().map(el => parseInt(el.style.getPropertyValue('--stagger-delay'), 10))
    for (let i = 1; i < ms.length; i++) {
      expect(ms[i] - ms[i - 1]).toBe(40)
      expect(ms[i]).toBeGreaterThan(ms[i - 1])
    }
    expect(ms[0]).toBe(0)
  })

  it('按钮本体不带 transition-delay / opacity —— 延迟只经 --stagger-delay 交给 hover 规则', () => {
    render(<DesktopToolbar />)
    for (const btn of column()) {
      // 收起必须零延迟；若这里被写成错峰，鼠标快速划过会留下逐个消失的拖尾
      expect(btn.style.transitionDelay).toBe('')
      expect(btn.style.opacity).toBe('')
    }
  })

  it('鼠标工具激活后：读数与关闭并排成一组，且都不占按钮列、不吃错峰', () => {
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))

    const group = document.querySelector('.desktop-toolbar-pos-group') as HTMLElement
    expect(group).toBeTruthy()
    // 关**闭钮与读数同级**（并排），不再排在按钮列末尾、落在读数下方
    const readout = screen.getByTitle('点击复制坐标')
    const close = screen.getByRole('button', { name: '关闭鼠标坐标' })
    expect(readout.parentElement).toBe(group)
    expect(close.parentElement).toBe(group)
    // DOM 序：读数在前、关闭在后（视觉上关闭在坐标右侧）
    expect(Array.from(group.children).indexOf(readout)).toBeLessThan(
      Array.from(group.children).indexOf(close),
    )
    // 两者都不是列成员、也不吃错峰
    expect(column()).toHaveLength(6)
    expect(column().includes(close)).toBe(false)
    expect(close.style.getPropertyValue('--stagger-delay')).toBe('')
    expect(readout.style.getPropertyValue('--stagger-delay')).toBe('')
  })

  it('点关闭后读数与关闭钮一起消失（常驻直到点关闭）', () => {
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    expect(document.querySelector('.desktop-toolbar-pos-group')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: '关闭鼠标坐标' }))
    expect(document.querySelector('.desktop-toolbar-pos-group')).toBeNull()
    expect(screen.queryByTitle('点击复制坐标')).toBeNull()
  })

  it('关闭钮不带 inline 颜色（配色归 CSS 的 --error token，不写死十六进制）', () => {
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    const close = screen.getByRole('button', { name: '关闭鼠标坐标' })
    expect(close.style.color).toBe('')
    expect(close.className).toContain('desktop-toolbar-pos-close')
    expect(document.body.innerHTML).not.toContain('#f87171')
    expect(document.body.innerHTML).not.toContain('#3b82f6')
  })

  it('图标尺寸统一 15（与 header 三个按钮的 IconSettings/IconPalette/IconBrowser 同规格）', () => {
    render(<DesktopToolbar />)
    const widths = () =>
      Array.from(document.querySelectorAll('.desktop-toolbar-dock svg')).map(svg =>
        svg.getAttribute('width'),
      )
    expect(widths()).toHaveLength(6)
    expect(new Set(widths())).toEqual(new Set(['15']))

    // mouse_pos 的「停止跟踪」按钮是条件渲染的，必须单独覆盖 —— 否则改回 14 无人发现
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    const all = widths()
    expect(all).toHaveLength(7) // 6 工具图标 + 关闭钮 IconX（坐标读数是纯文本，无 svg）
    expect(new Set(all)).toEqual(new Set(['15']))
  })

  it('图标 stroke-width 一律取 lucide 默认 2（不显式传 strokeWidth，与 header 一致）', () => {
    render(<DesktopToolbar />)
    const strokes = () =>
      Array.from(document.querySelectorAll('.desktop-toolbar-dock svg')).map(svg =>
        svg.getAttribute('stroke-width'),
      )
    expect(strokes()).toHaveLength(6)
    expect(new Set(strokes())).toEqual(new Set(['2']))

    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    expect(new Set(strokes())).toEqual(new Set(['2']))
  })
})
