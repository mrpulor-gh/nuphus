import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../../locales'
import { ThinkingIndicator } from './ThinkingIndicator'

vi.mock('../../ui/MoodFace', () => ({ MoodFace: () => <span /> }))
vi.mock('../../ui/Button', () => ({
  IconButton: ({ children }: { children: React.ReactNode }) => <button>{children}</button>,
}))
vi.mock('../../ui/Icons', () => ({
  IconChevronRight: () => <span />,
  IconX: () => <span />,
}))
vi.mock('../../ui/sound', () => ({ playUiSound: vi.fn() }))

class ResizeObserverMock {
  observe() {}
  disconnect() {}
}

describe('ThinkingIndicator 总步数', () => {
  it('徽标显示共享 turnMeta 的 Leader + ExecAgent 累计值', () => {
    vi.stubGlobal('ResizeObserver', ResizeObserverMock)
    render(
      <LangProvider>
        <ThinkingIndicator step="执行中" isThinking toolCallCount={5} timeline={[]} />
      </LangProvider>,
    )

    expect(screen.getByText('5 步')).toHaveClass('thinking-call-badge')
    vi.unstubAllGlobals()
  })
})
