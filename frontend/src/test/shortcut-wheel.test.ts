import { describe, expect, it, vi } from 'vitest'
import { routePrimaryK } from '../main-window/shortcutRouting'
import { formatPrimaryShortcut } from '../ui/platformShortcut'
import { createWheelSelectionAccumulator } from '../ui/wheelSelection'

describe('主快捷键路由', () => {
  it('Workflow 模式直接打开工作流列表', () => {
    const openWorkflow = vi.fn()
    const togglePalette = vi.fn()
    routePrimaryK('workflow', openWorkflow, togglePalette)
    expect(openWorkflow).toHaveBeenCalledOnce()
    expect(togglePalette).not.toHaveBeenCalled()
  })

  it('其他模式继续切换全局命令面板，并按平台显示文案', () => {
    const openWorkflow = vi.fn()
    const togglePalette = vi.fn()
    routePrimaryK('leader', openWorkflow, togglePalette)
    expect(togglePalette).toHaveBeenCalledOnce()
    expect(formatPrimaryShortcut('K', 'MacIntel')).toBe('Cmd+K')
    expect(formatPrimaryShortcut('K', 'Win32')).toBe('Ctrl+K')
  })
})

describe('命令面板滚轮累计', () => {
  it('高频触控板小增量只推进合理数量的选项', () => {
    const accumulator = createWheelSelectionAccumulator()
    let moved = 0
    for (let i = 0; i < 20; i++) moved += accumulator.push(4, 0, i * 5)
    expect(moved).toBe(2)
  })

  it('单个滚轮刻度只推进一项，快速重复事件受节流', () => {
    const accumulator = createWheelSelectionAccumulator()
    expect(accumulator.push(100, 0, 0)).toBe(1)
    expect(accumulator.push(100, 0, 10)).toBe(0)
    expect(accumulator.push(100, 0, 60)).toBe(1)
  })

  it('方向反转会清除上一方向未完成的累计量', () => {
    const accumulator = createWheelSelectionAccumulator()
    expect(accumulator.push(30, 0, 0)).toBe(0)
    expect(accumulator.push(-15, 0, 10)).toBe(0)
    expect(accumulator.push(-25, 0, 55)).toBe(-1)
  })
})
