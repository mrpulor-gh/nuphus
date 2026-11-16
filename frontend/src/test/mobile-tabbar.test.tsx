import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import NavBar from '../mobile/components/NavBar'
import TabBar from '../mobile/components/TabBar'
import type { ActivityState } from '../mobile/store'

/**
 * 移动端主导航（底部 Tab + 页面化）回归。
 *
 * 替代旧「点品牌区 → 右侧抽屉」：入口可见性由 TabBar 承担，
 * 页面由 page prop 直接渲染（聊天 / 会话 / 设置），品牌区不再可点。
 */

vi.mock('../mobile/api', () => ({
  fetchCustomAgents: vi.fn(() => Promise.resolve([])),
  fetchModelConfig: vi.fn(() => Promise.resolve(null)),
  switchMobileMode: vi.fn(() => Promise.resolve({ ok: true })),
  switchMobileModel: vi.fn(() => Promise.resolve({ ok: true })),
}))

const activity: ActivityState = {
  running: false,
  goal: '',
  mode: 'leader',
  tools: [],
  paused: false,
}

function renderNav(page: 'chat' | 'sessions' | 'settings', onPageChange = vi.fn()) {
  render(
    <NavBar
      page={page}
      onPageChange={onPageChange}
      wsStatus="online"
      activity={activity}
      token="token"
      model="test-model"
      sessions={null}
    />,
  )
  return { onPageChange }
}

describe('移动端底部导航', () => {
  it('TabBar 渲染三个入口，点击回传目标页', () => {
    const onChange = vi.fn()
    render(<TabBar page="chat" onChange={onChange} />)

    expect(screen.getByRole('tab', { name: '聊天' })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByRole('tab', { name: '会话' })).toHaveAttribute('aria-selected', 'false')

    fireEvent.click(screen.getByRole('tab', { name: '设置' }))
    expect(onChange).toHaveBeenCalledWith('settings')

    fireEvent.click(screen.getByRole('tab', { name: '会话' }))
    expect(onChange).toHaveBeenCalledWith('sessions')
  })

  it('品牌区不再承担菜单职责（无「菜单」按钮）', () => {
    renderNav('chat')
    expect(screen.queryByLabelText('菜单')).not.toBeInTheDocument()
  })

  it('会话页只渲染会话列表，不渲染设置内容', () => {
    renderNav('sessions')
    const page = screen.getByRole('dialog', { name: '会话' })
    expect(page).toBeInTheDocument()
    // 会话页独有：新建会话入口
    expect(screen.getByLabelText('新会话')).toBeInTheDocument()
    // 设置页独有：模型卡的「切换」按钮不应出现
    expect(screen.queryByText('切换')).not.toBeInTheDocument()
  })

  it('设置页渲染模式与模型卡，不渲染会话列表', () => {
    renderNav('settings')
    const page = screen.getByRole('dialog', { name: '设置' })
    expect(page).toBeInTheDocument()
    expect(screen.getByText('切换')).toBeInTheDocument()
    expect(screen.queryByLabelText('新会话')).not.toBeInTheDocument()
  })

  it('聊天页不渲染任何页面容器', () => {
    renderNav('chat')
    expect(screen.queryByRole('dialog', { name: '会话' })).not.toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: '设置' })).not.toBeInTheDocument()
  })
})
