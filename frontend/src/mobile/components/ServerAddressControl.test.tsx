import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import ServerAddressControl from './ServerAddressControl'

describe('ServerAddressControl', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('shows the current endpoint and reports the no-op state', () => {
    render(<ServerAddressControl defaultExpanded />)

    expect(screen.getByText('Nuphus 服务端')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '打开' }))

    expect(screen.getByText('当前已连接这个服务端')).toBeTruthy()
  })

  it('validates a malformed endpoint without navigating', () => {
    render(<ServerAddressControl defaultExpanded />)
    const input = screen.getByLabelText('服务端页面地址')

    fireEvent.change(input, { target: { value: 'ftp://example.com' } })
    fireEvent.click(screen.getByRole('button', { name: '打开' }))

    expect(screen.getByText('地址无效，请使用 http:// 或 https:// 地址')).toBeTruthy()
  })
})
