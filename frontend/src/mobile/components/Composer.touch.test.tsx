import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import Composer from './Composer'

/**
 * 移动端输入栏「点一次就发送」契约。
 *
 * 真机链路（本用例守护的缺陷）：点击输入栏内的非输入控件时，浏览器先把焦点从
 * textarea 移走 → 软键盘收起 → 布局视口变高 + TabBar 重新挂载（ChatScreen 按
 * focusout 卸载）→ 输入栏在手指下方位移约一个键盘高度 → touchend 命中测试已不在
 * 按钮上，合成 click 被丢弃 → 用户必须点第二次。
 * 因此：触摸必须在 pointerdown 阶段受理；鼠标 / 键盘 / 读屏仍走 click；
 * 同一次物理点击只允许执行一次。
 */

const onSend = vi.fn(async (_content: string) => {})
const onStopExecution = vi.fn()

/** jsdom 不实现 PointerEvent：与 ProblemsPanel.test 同法，用 MouseEvent 补 pointer 字段 */
function pointer(el: Element, type: string, pointerType: string) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 })
  Object.defineProperty(event, 'pointerId', { value: 1 })
  Object.defineProperty(event, 'pointerType', { value: pointerType })
  fireEvent(el, event)
}

function mount(props: { isProcessing?: boolean } = {}) {
  return render(
    <Composer
      disabled={false}
      token="test-token"
      mode="leader"
      isProcessing={props.isProcessing ?? false}
      onStopExecution={onStopExecution}
      onSend={onSend}
    />,
  )
}

beforeEach(() => {
  onSend.mockClear()
  onStopExecution.mockClear()
})

describe('mobile composer 触摸手势', () => {
  it('触摸端 pointerdown 即受理发送，随后补发的 click 不重复发送', () => {
    mount()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '你好' } })
    const send = screen.getByRole('button', { name: '发送' })

    pointer(send, 'pointerdown', 'touch')

    expect(onSend).toHaveBeenCalledTimes(1)
    expect(onSend.mock.calls[0][0]).toBe('你好')

    // 浏览器在布局位移后仍可能补发 click：不得重复发送
    fireEvent.click(send)
    expect(onSend).toHaveBeenCalledTimes(1)
  })

  it('鼠标点击仍走 click 路径（pointerdown 不触发）', () => {
    mount()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'hi' } })
    const send = screen.getByRole('button', { name: '发送' })

    pointer(send, 'pointerdown', 'mouse')
    expect(onSend).not.toHaveBeenCalled()

    fireEvent.click(send)
    expect(onSend).toHaveBeenCalledTimes(1)
    expect(onSend.mock.calls[0][0]).toBe('hi')
  })

  it('执行中空输入的终止按钮同样一次点击生效且不重复', () => {
    mount({ isProcessing: true })
    const stop = screen.getByRole('button', { name: '终止' })

    pointer(stop, 'pointerdown', 'touch')
    expect(onStopExecution).toHaveBeenCalledTimes(1)

    fireEvent.click(stop)
    expect(onStopExecution).toHaveBeenCalledTimes(1)
  })
})
