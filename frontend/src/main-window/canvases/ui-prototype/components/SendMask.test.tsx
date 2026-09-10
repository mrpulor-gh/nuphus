import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { PALETTES } from '../lib/tokens'
import { SEND_MASK_Z, SendMask } from './SendMask'

describe('SendMask', () => {
  it('portal 到 document.body 的全屏遮罩：文案可见、阻止指针、层级压住画布与工作台', () => {
    const text = '正在生成原型图并发送…'
    const { unmount } = render(<SendMask p={PALETTES[0]} text={text} />)

    const mask = screen.getByRole('status')
    // 挂载点：必须是 body 直属，否则会被 .canvas-workbench-host 的 stacking context 困住
    expect(mask.parentElement).toBe(document.body)
    expect(mask).toHaveAttribute('aria-busy', 'true')
    expect(mask).toHaveAttribute('aria-label', text)
    expect(mask).toHaveTextContent(text)
    expect(mask.style.position).toBe('fixed')
    expect(mask.style.inset).toBe('0px')
    expect(mask.style.zIndex).toBe(String(SEND_MASK_Z))

    // 卸载不残留：组件消失时 portal 随之移除
    unmount()
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('层级取值高于画布全屏壳（2500）与工作台宿主（100）', () => {
    // 依据：canvases.css `.canvas-hub` / `.canvas-page-host` 为 fixed + z-index:2500；
    // components.css `.canvas-workbench-host` 为 fixed + z-index:100。
    expect(SEND_MASK_Z).toBeGreaterThan(2500)
    expect(SEND_MASK_Z).toBeGreaterThan(100)
  })
})
