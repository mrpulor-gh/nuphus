import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../../../locales'
import { PanelFrame } from './PanelFrame'

function renderPanel(overrides: Partial<Parameters<typeof PanelFrame>[0]> = {}) {
  const onWidthPersist = vi.fn()
  const onClose = vi.fn()
  render(
    <LangProvider>
      <PanelFrame
        title="生成主视觉图"
        badge={<span className="wfc-node-kind">model</span>}
        summary="调用模型生成图像 · 文生图"
        width={400}
        onWidthPersist={onWidthPersist}
        onClose={onClose}
        {...overrides}
      >
        <div data-testid="panel-body-content">字段内容</div>
      </PanelFrame>
    </LangProvider>,
  )
  return { onWidthPersist, onClose }
}

describe('PanelFrame resize 手柄', () => {
  it('separator 暴露完整 aria（orientation / 边界 / 当前值）', () => {
    renderPanel({ width: 480 })
    const handle = screen.getByRole('separator')
    expect(handle).toHaveAttribute('aria-orientation', 'vertical')
    expect(handle).toHaveAttribute('aria-valuemin', '360')
    expect(handle).toHaveAttribute('aria-valuemax', '720')
    expect(handle).toHaveAttribute('aria-valuenow', '480')
    expect(handle).toHaveAttribute('tabindex', '0')
  })

  it('方向键 ±20 调整宽度（左键加宽，与现有 Inspector 一致）', () => {
    const { onWidthPersist } = renderPanel({ width: 400 })
    const handle = screen.getByRole('separator')
    fireEvent.keyDown(handle, { key: 'ArrowLeft' })
    expect(onWidthPersist).toHaveBeenCalledWith(420)
    fireEvent.keyDown(handle, { key: 'ArrowRight' })
    expect(onWidthPersist).toHaveBeenCalledWith(380)
  })

  it('宽度钳制在 360–720', () => {
    const { onWidthPersist } = renderPanel({ width: 720 })
    fireEvent.keyDown(screen.getByRole('separator'), { key: 'ArrowLeft' })
    expect(onWidthPersist).toHaveBeenCalledWith(720)

    const atMin = renderPanel({ width: 360 })
    fireEvent.keyDown(screen.getAllByRole('separator')[1], { key: 'ArrowRight' })
    expect(atMin.onWidthPersist).toHaveBeenCalledWith(360)
  })
})

describe('PanelFrame 骨架', () => {
  it('身份头渲染 title / badge / 关闭按钮，body 承载 children', () => {
    const { onClose } = renderPanel()
    expect(screen.getByText('生成主视觉图')).toBeInTheDocument()
    expect(screen.getByText('model')).toBeInTheDocument()
    expect(screen.getByTestId('panel-body-content')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '关闭（Esc）' }))
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('摘要行渲染文本并带 title（超宽省略的兜底）', () => {
    renderPanel({ summary: '调用模型生成图像 · 文生图' })
    const summary = screen.getByText('调用模型生成图像 · 文生图')
    expect(summary).toHaveClass('wfp-panel__summary')
    expect(summary).toHaveAttribute('title', '调用模型生成图像 · 文生图')
  })

  it('面板内聚焦时 Esc 关闭；面板外按键不触发', () => {
    const { onClose } = renderPanel()
    fireEvent.keyDown(screen.getByRole('separator'), { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)

    fireEvent.keyDown(document.body, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
