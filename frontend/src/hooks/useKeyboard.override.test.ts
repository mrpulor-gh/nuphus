/**
 * useKeyboard 覆盖生效单测（v1.5 最小 resolver）：
 * 模拟 keydown 验证「有覆盖按覆盖键位、无/非法覆盖回落默认」的真实生效链路。
 */
import { cleanup, fireEvent, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useKeyboard } from './useKeyboard'

const keydown = (init: KeyboardEventInit) => fireEvent.keyDown(window, init)

describe('useKeyboard override resolver', () => {
  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
  })

  it('存在覆盖：仅新组合触发，默认组合不再触发', () => {
    const handler = vi.fn()
    renderHook(() =>
      useKeyboard([{ id: 'palette', key: 'k', ctrl: true, handler }], {
        palette: 'Ctrl+Shift+K',
      }),
    )

    keydown({ key: 'k', ctrlKey: true })
    expect(handler).not.toHaveBeenCalled()

    keydown({ key: 'K', ctrlKey: true, shiftKey: true })
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('无覆盖：默认键位照常生效', () => {
    const handler = vi.fn()
    renderHook(() => useKeyboard([{ id: 'focusInput', key: 'l', ctrl: true, handler }]))

    keydown({ key: 'l', ctrlKey: true })
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('非法覆盖（解析失败）：回落默认键位', () => {
    const handler = vi.fn()
    renderHook(() =>
      useKeyboard([{ id: 'focusInput', key: 'l', ctrl: true, handler }], { focusInput: 'Ctrl+' }),
    )

    keydown({ key: 'l', ctrlKey: true })
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('覆盖匹配后 preventDefault 被接管（原键位不再冒泡到页面）', () => {
    const handler = vi.fn()
    renderHook(() =>
      useKeyboard([{ id: 'newChat', key: 'n', ctrl: true, handler }], { newChat: 'Ctrl+Alt+N' }),
    )

    const event = new KeyboardEvent('keydown', {
      key: 'n',
      ctrlKey: true,
      altKey: true,
      cancelable: true,
    })
    window.dispatchEvent(event)
    expect(handler).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
  })
})
