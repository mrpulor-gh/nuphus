import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import { ScopedEditDialog } from './ScopedEditDialog'
import { scopedRevision, type ScopedEditProposal, type ScopedEditRequest } from './scopedEdit'

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('../../core/bridge', () => ({ invoke: (...args: unknown[]) => mocks.invoke(...args) }))
vi.mock('../../locales', () => ({ useLanguage: () => ({ lang: 'en' }) }))
const steps: WorkflowStep[] = [
  { id: 'selected', name: 'Before', do: { sleep: 1 } },
  { id: 'outside', name: 'Outside', do: { sleep: 1 } },
]
const p = (): ScopedEditProposal => ({
  base_revision: scopedRevision(steps),
  summary: 'Updated name',
  updates: [{ step_id: 'selected', step: { ...steps[0], name: 'After' } }],
})
const generate = () => {
  fireEvent.change(screen.getByLabelText('Requested changes'), {
    target: { value: 'Rename the selected step' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Generate proposal' }))
}
beforeEach(() => {
  mocks.invoke.mockReset()
})

describe('ScopedEditDialog', () => {
  it('shows field differences before explicit apply and preserves unselected steps', async () => {
    const onApply = vi.fn()
    const onClose = vi.fn()
    mocks.invoke.mockResolvedValue(p())
    render(
      <ScopedEditDialog
        steps={steps}
        selectedIds={['selected']}
        onApply={onApply}
        onClose={onClose}
      />,
    )
    generate()
    await screen.findByText('Updated name')
    expect(screen.getByText('selected · name')).toBeTruthy()
    expect(onApply).not.toHaveBeenCalled()
    expect(mocks.invoke.mock.calls[0][0]).toBe('wf_propose_scoped_edit')
    const request = mocks.invoke.mock.calls[0][1].request as ScopedEditRequest
    expect(request.selected_ids).toEqual(['selected'])
    fireEvent.click(screen.getByRole('button', { name: 'Apply changes' }))
    expect(onApply).toHaveBeenCalledWith([{ ...steps[0], name: 'After' }, steps[1]])
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('disables apply when editor data changes while a request is pending', async () => {
    let resolve!: (value: ScopedEditProposal) => void
    mocks.invoke.mockReturnValue(
      new Promise<ScopedEditProposal>(done => {
        resolve = done
      }),
    )
    const onApply = vi.fn()
    const props = { steps, selectedIds: ['selected'], onApply, onClose: vi.fn() }
    const { rerender } = render(<ScopedEditDialog {...props} />)
    generate()
    rerender(
      <ScopedEditDialog
        {...props}
        steps={[steps[0], { ...steps[1], name: 'Changed outside scope' }]}
      />,
    )
    await act(async () => resolve(p()))
    expect(screen.getByRole('alert').textContent).toContain('stale')
    expect(screen.getByRole('button', { name: 'Apply changes' })).toBeDisabled()
    expect(onApply).not.toHaveBeenCalled()
  })

  it('blocks scope violations and does not apply when cancelled', async () => {
    mocks.invoke.mockResolvedValue({
      ...p(),
      updates: [{ step_id: 'outside', step: { ...steps[1], name: 'Bad' } }],
    })
    const onApply = vi.fn()
    const onClose = vi.fn()
    render(
      <ScopedEditDialog
        steps={steps}
        selectedIds={['selected']}
        onApply={onApply}
        onClose={onClose}
      />,
    )
    generate()
    await screen.findByRole('alert')
    expect(screen.getByRole('button', { name: 'Apply changes' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(onApply).not.toHaveBeenCalled()
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('ignores a late response after closing', async () => {
    let resolve!: (value: ScopedEditProposal) => void
    mocks.invoke.mockReturnValue(
      new Promise<ScopedEditProposal>(done => {
        resolve = done
      }),
    )
    const onApply = vi.fn()
    const onClose = vi.fn()
    render(
      <ScopedEditDialog
        steps={steps}
        selectedIds={['selected']}
        onApply={onApply}
        onClose={onClose}
      />,
    )
    generate()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await act(async () => resolve(p()))
    await waitFor(() => expect(screen.queryByText('Updated name')).toBeNull())
    expect(onApply).not.toHaveBeenCalled()
  })
})
