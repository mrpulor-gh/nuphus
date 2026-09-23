import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ApprovalModal } from '../main-window/components/ApprovalModal'
import { invoke } from '../core/bridge'

vi.mock('../core/bridge', () => ({ invoke: vi.fn(async () => 'approved') }))
vi.mock('../locales', () => ({ useLanguage: () => ({ t: (key: string) => key }) }))
vi.mock('../ui/sound', () => ({ playPopupSound: vi.fn() }))

const props = {
  open: true,
  kind: 'desktop_action',
  title: 'Delete temporary record',
  content: 'This deletes the selected temporary record.',
  actionId: 'desktop-one',
  tenetCount: 3,
  onClose: vi.fn(),
}

describe('desktop approval', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(invoke).mockResolvedValue('approved')
  })

  it('approves only the pending action and never displays tenet/session authorization', async () => {
    render(<ApprovalModal {...props} />)
    expect(screen.getByText('approval.desktopDesc')).toBeTruthy()
    expect(screen.queryByText('approval.count')).toBeNull()
    expect(screen.queryByText('approval.approve')).toBeNull()
    fireEvent.keyDown(window, { key: 'Enter' })
    expect(invoke).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'approval.desktopApprove' }))
    await waitFor(() => expect(screen.getByText('approval.desktopApproved')).toBeTruthy())
    expect(invoke).toHaveBeenCalledExactlyOnceWith('approve_pending', { actionId: 'desktop-one' })
    expect(screen.getByText('approval.desktopApprovedDesc')).toBeTruthy()
    expect(screen.queryByText('approval.saved')).toBeNull()
  })

  it('Escape rejects without executing', async () => {
    render(<ApprovalModal {...props} />)
    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() => expect(screen.getByText('approval.desktopRejectedDesc')).toBeTruthy())
    expect(invoke).toHaveBeenCalledExactlyOnceWith('reject_pending', { actionId: 'desktop-one' })
  })

  it('preserves tenet approval text and keyboard behavior', async () => {
    render(<ApprovalModal {...props} kind="tenet" />)
    expect(screen.getByText('approval.count')).toBeTruthy()
    fireEvent.keyDown(window, { key: 'Enter' })
    await waitFor(() => expect(screen.getByText('approval.saved')).toBeTruthy())
    expect(screen.queryByText('approval.desktopApproved')).toBeNull()
  })

  it('does not carry the previous result into the next request', async () => {
    const view = render(<ApprovalModal {...props} />)
    fireEvent.click(screen.getByRole('button', { name: 'approval.desktopApprove' }))
    await waitFor(() => expect(screen.getByText('approval.desktopApproved')).toBeTruthy())
    view.rerender(<ApprovalModal {...props} actionId="desktop-two" />)
    expect(screen.queryByText('approval.desktopApproved')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'approval.desktopApprove' }))
    await waitFor(() =>
      expect(invoke).toHaveBeenLastCalledWith('approve_pending', { actionId: 'desktop-two' }),
    )
  })

  it('ignores a late response after the host presents another action', async () => {
    let finish: (value: string) => void = () => {}
    vi.mocked(invoke).mockReturnValueOnce(
      new Promise<string>(resolve => {
        finish = resolve
      }),
    )
    const view = render(<ApprovalModal {...props} />)
    fireEvent.click(screen.getByRole('button', { name: 'approval.desktopApprove' }))
    view.rerender(<ApprovalModal {...props} actionId="desktop-two" />)
    await act(async () => {
      finish('approved')
    })
    expect(screen.queryByText('approval.desktopApproved')).toBeNull()
    expect(screen.getByRole('button', { name: 'approval.desktopApprove' })).toBeTruthy()
  })
})
