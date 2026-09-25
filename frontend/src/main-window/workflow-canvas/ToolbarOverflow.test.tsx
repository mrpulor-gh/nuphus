import { createRef } from 'react'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ToolbarOverflow } from './ToolbarOverflow'

describe('adaptive canvas toolbar', () => {
  it('moves low-priority actions into More without losing actions or duplicating controls', () => {
    const click = vi.fn()
    const menuRef = createRef<HTMLDetailsElement>()
    render(
      <div className="wfc-toolbar">
        <ToolbarOverflow
          menuRef={menuRef}
          actions={['Details', 'AI', 'Undo', 'Redo'].map(label => (
            <button key={label} onClick={click}>
              {label}
            </button>
          ))}
        >
          <button>Check</button>
        </ToolbarOverflow>
      </div>,
    )
    const toolbar = document.querySelector('.wfc-toolbar')!
    const bounds = vi.spyOn(toolbar, 'getBoundingClientRect')
    for (const [width, hidden] of [
      [1440, 0],
      [1100, 1],
      [1000, 2],
      [900, 4],
    ]) {
      bounds.mockReturnValue({ width } as DOMRect)
      act(() => {
        window.dispatchEvent(new Event('resize'))
      })
      expect(menuRef.current!.querySelectorAll('button')).toHaveLength(hidden + 1)
      expect(toolbar.querySelectorAll('button')).toHaveLength(5)
    }
    fireEvent.click(screen.getByText('更多'))
    fireEvent.click(screen.getByRole('button', { name: 'AI' }))
    expect(click).toHaveBeenCalledOnce()
    expect(menuRef.current!.open).toBe(false)
  })
})
