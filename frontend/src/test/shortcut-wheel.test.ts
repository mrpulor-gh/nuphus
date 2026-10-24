import { describe, expect, it, vi } from 'vitest'
import { modelSetupHint, routePrimaryF, routePrimaryK } from '../main-window/shortcutRouting'
import { formatPrimaryShortcut } from '../ui/platformShortcut'
import { createWheelSelectionAccumulator } from '../ui/wheelSelection'

describe('主快捷键路由', () => {
  it('Ctrl+K 在任何模式下都只切换全局命令面板', () => {
    const togglePalette = vi.fn()
    routePrimaryK(togglePalette)
    expect(togglePalette).toHaveBeenCalledOnce()
    expect(formatPrimaryShortcut('K', 'MacIntel')).toBe('Cmd+K')
    expect(formatPrimaryShortcut('K', 'Win32')).toBe('Ctrl+K')
  })

  it('Ctrl+F 打开工作流列表（与模式无关）', () => {
    const openWorkflow = vi.fn()
    routePrimaryF(openWorkflow)
    expect(openWorkflow).toHaveBeenCalledOnce()
    expect(formatPrimaryShortcut('F', 'Win32')).toBe('Ctrl+F')
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
