import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DesktopToolbar } from './DesktopToolbar'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

describe('user-selected desktop application registration', () => {
  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
  })
  afterEach(cleanup)

  it('opens the host picker without a model-provided path and reports the registered name', async () => {
    invoke.mockResolvedValue({ name: 'Portable Editor', app_ref: 'app:local' })
    render(<DesktopToolbar visible onClose={() => {}} />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    expect(await screen.findByText('应用已登记')).toBeTruthy()
    expect(screen.getByText(/Portable Editor/)).toBeTruthy()
    expect(invoke).toHaveBeenCalledExactlyOnceWith('desktop_register_application', undefined)
  })

  it('cancelling the native picker neither launches an application nor reports success', async () => {
    invoke.mockResolvedValue(null)
    render(<DesktopToolbar visible onClose={() => {}} />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '登记应用' })).not.toBeDisabled())
    expect(screen.queryByText('应用已登记')).toBeNull()
    expect(invoke).toHaveBeenCalledTimes(1)
  })
})
