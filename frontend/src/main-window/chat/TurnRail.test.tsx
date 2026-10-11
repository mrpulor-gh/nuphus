/**
 * TurnRail 组件测试 —— 渲染门、刻度语义、预览气泡、跳转落点。
 * 只断言组件自身契约；切轮/激活推断的纯逻辑由 turnRailModel.test.ts 覆盖。
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { useRef } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { TurnRail, type TurnRailMessage } from './TurnRail'

const ZH: Record<string, string> = {
  'chat.turnRail.label': '对话轮次导航',
  'chat.turnRail.jump': '跳转到第 {0} 轮',
  'chat.turnRail.turn': '第 {0} 轮',
}
const t = (key: string, ...args: string[]) => {
  let text = ZH[key] ?? key
  args.forEach((arg, index) => {
    text = text.replace(`{${index}}`, arg)
  })
  return text
}

/** 宿主桩：滚动容器 + 锚点行 + 轮次轨（与 ChatPanel 的接线同构）。 */
function HostStub({ messages }: { messages: TurnRailMessage[] }) {
  const scrollRef = useRef<HTMLDivElement>(null)
  return (
    <div ref={scrollRef} data-testid="scroller">
      {messages.map(m => (
        <div key={m.id} id={`turn-anchor-${m.id}`}>
          {m.content}
        </div>
      ))}
      <TurnRail messages={messages} scrollRef={scrollRef} t={t} />
    </div>
  )
}

const three: TurnRailMessage[] = [
  { id: 'u1', role: 'user', content: '第一问' },
  { id: 'a1', role: 'assistant', content: '第一答' },
  { id: 'u2', role: 'user', content: '第二问' },
  { id: 'a2', role: 'assistant', content: '第二答' },
  { id: 'u3', role: 'user', content: '第三问' },
  { id: 'a3', role: 'assistant', content: '第三答' },
]

describe('TurnRail', () => {
  it('不足两轮不渲染（无导航意义）', () => {
    const { container } = render(
      <HostStub
        messages={[
          { id: 'u1', role: 'user', content: '问' },
          { id: 'a1', role: 'assistant', content: '答' },
        ]}
      />,
    )
    expect(container.querySelector('.turn-rail')).toBeNull()
    expect(screen.queryByRole('navigation')).toBeNull()
  })

  it('每轮一枚刻度，aria-label 带轮次号', () => {
    render(<HostStub messages={three} />)
    const marks = screen.getAllByRole('button')
    expect(marks).toHaveLength(3)
    expect(marks[0]).toHaveAttribute('aria-label', '跳转到第 1 轮')
    expect(marks[2]).toHaveAttribute('aria-label', '跳转到第 3 轮')
  })

  it('轨道带确定高度（模块不为 0：刻度全绝对定位，无高度会整体塌陷）', () => {
    render(<HostStub messages={three} />)
    const track = document.querySelector('.turn-rail-track') as HTMLElement
    // RAIL_INSET_PX*2 + (count-1)*TURN_PITCH_PX = 6*2 + 2*10 = 32
    expect(track.style.height).toBe('32px')
  })

  it('悬停刻度弹出预览气泡（提问一行 + 答复三行）', async () => {
    render(<HostStub messages={three} />)
    expect(document.querySelector('.turn-rail-preview')).toBeNull()
    fireEvent.pointerMove(screen.getAllByRole('button')[1])
    const preview = document.querySelector('.turn-rail-preview')
    expect(preview).not.toBeNull()
    expect(preview?.querySelector('.turn-rail-preview-prompt')?.textContent).toBe('第二问')
    expect(preview?.querySelector('.turn-rail-preview-response')?.textContent).toBe('第二答')
  })

  it('点击刻度跳转到该轮锚点元素', () => {
    const scrollIntoView = vi.fn()
    Element.prototype.scrollIntoView = scrollIntoView
    render(<HostStub messages={three} />)
    fireEvent.click(screen.getAllByRole('button')[1])
    expect(scrollIntoView).toHaveBeenCalledTimes(1)
    expect(scrollIntoView.mock.calls[0][0]).toMatchObject({ block: 'start' })
  })

  it('滚动后推断激活轮并标记 aria-current', async () => {
    render(<HostStub messages={three} />)
    await waitFor(() => {
      const marks = screen.getAllByRole('button')
      // jsdom 无真实布局，所有锚点 top 均为 0 → 激活轮取最后一轮
      expect(marks[2]).toHaveAttribute('aria-current', 'true')
    })
  })

  it('新消息到达（锚点数增加）后最新刻度自动成为激活轮', async () => {
    const { rerender } = render(<HostStub messages={three} />)
    await waitFor(() => {
      expect(screen.getAllByRole('button')[2]).toHaveAttribute('aria-current', 'true')
    })
    // 发送第四轮：新刻度必须即时点亮，而不是把锚定留在上一轮
    rerender(
      <HostStub
        messages={[
          ...three,
          { id: 'u4', role: 'user', content: '第四问' },
          { id: 'a4', role: 'assistant', content: '第四答' },
        ]}
      />,
    )
    await waitFor(() => {
      const marks = screen.getAllByRole('button')
      expect(marks).toHaveLength(4)
      expect(marks[3]).toHaveAttribute('aria-current', 'true')
      expect(marks[2]).not.toHaveAttribute('aria-current')
    })
  })
})
