import { describe, expect, it, vi } from 'vitest'
import { modelSetupHint, routePrimaryK } from '../main-window/shortcutRouting'
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

  it('未配置模型提示不会在 Workflow 模式误导用户使用主快捷键', () => {
    expect(modelSetupHint('workflow', 'Cmd+K')).toBe('⚠ 尚未配置模型，请前往模型设置')
    expect(modelSetupHint('leader', 'Cmd+K')).toContain('Cmd+K → 模型设置')
  })
})

describe('命令面板滚轮累计', () => {
  it('高频触控板小增量只推进合理数量的选项', () => {
    const accumulator = createWheelSelectionAccumulator()
    let moved = 0
    for (let i = 0; i < 20; i++) moved += accumulator.push(4, 0, i * 5)
    expect(moved).toBe(2)
  })

  it('离散鼠标滚轮保持一事件一格，不受触控板节流影响', () => {
    const accumulator = createWheelSelectionAccumulator()
    expect(accumulator.push(100, 0, 0)).toBe(1)
    expect(accumulator.push(100, 0, 10)).toBe(1)
    expect(accumulator.push(100, 0, 60)).toBe(1)
  })

  it('方向反转会清除上一方向未完成的累计量', () => {
    const accumulator = createWheelSelectionAccumulator()
    expect(accumulator.push(30, 0, 0)).toBe(0)
    expect(accumulator.push(-15, 0, 10)).toBe(0)
    expect(accumulator.push(-25, 0, 55)).toBe(-1)
  })
})
