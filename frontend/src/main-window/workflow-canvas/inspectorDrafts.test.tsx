import { act, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { InspectorDraftContext, InspectorDraftStore, useInspectorDraft } from './inspectorDrafts'

function Field({
  name,
  value,
  commit,
  validate,
}: {
  name: string
  value: string
  commit: (text: string) => unknown
  validate?: (text: string) => string | null
}) {
  const draft = useInspectorDraft(name, value, { onCommit: commit, validate })
  return (
    <input
      aria-label={name}
      value={draft.text}
      onChange={e => draft.setText(e.target.value)}
      onBlur={() => void draft.commit()}
    />
  )
}

describe('canvas inspector drafts', () => {
  it('flushes typing without blur, merging fields against the fresh state', async () => {
    const store = new InspectorDraftStore()
    let final = {}
    function Form() {
      const [value, setValue] = useState({ name: 'old', description: 'before' })
      final = value
      return (
        <InspectorDraftContext.Provider value={{ store, nodeId: 'a' }}>
          <Field name="名称" value={value.name} commit={name => setValue({ ...value, name })} />
          <Field
            name="描述"
            value={value.description}
            commit={description => setValue({ ...value, description })}
          />
        </InspectorDraftContext.Provider>
      )
    }
    render(<Form />)
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'new' } })
    fireEvent.change(screen.getByLabelText('描述'), { target: { value: 'after' } })
    expect(store.dirty).toBe(true)
    await act(async () => {
      expect(await store.flush()).toBeNull()
    })
    expect(final).toEqual({ name: 'new', description: 'after' })
    expect(store.dirty).toBe(false)
  })

  it('validates all fields before applying a batch and reports node and field', async () => {
    const store = new InspectorDraftStore()
    const commit = vi.fn()
    render(
      <InspectorDraftContext.Provider value={{ store, nodeId: 'broken' }}>
        <Field name="名称" value="old" commit={commit} />
        <Field
          name="JSON"
          value="{}"
          commit={commit}
          validate={text => {
            try {
              JSON.parse(text)
              return null
            } catch {
              return 'JSON 格式错误'
            }
          }}
        />
      </InspectorDraftContext.Provider>,
    )
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'new' } })
    fireEvent.change(screen.getByLabelText('JSON'), { target: { value: '{' } })
    await act(async () => {
      expect(await store.flush()).toMatchObject({
        nodeId: 'broken',
        field: 'JSON',
        error: 'JSON 格式错误',
      })
    })
    expect(commit).not.toHaveBeenCalled()
    expect(store.dirty).toBe(true)
  })

  it('retains drafts in hidden inspectors and removes only deleted nodes', async () => {
    const store = new InspectorDraftStore()
    const commit = vi.fn()
    const view = (hidden: boolean) => (
      <div hidden={hidden}>
        <InspectorDraftContext.Provider value={{ store, nodeId: 'a' }}>
          <Field name="名称" value="old" commit={commit} />
        </InspectorDraftContext.Provider>
      </div>
    )
    const { rerender } = render(view(false))
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'draft' } })
    rerender(view(true))
    expect(store.dirty).toBe(true)
    rerender(view(false))
    expect(screen.getByLabelText('名称')).toHaveValue('draft')
    await act(async () => {
      await store.flush()
    })
    expect(commit).toHaveBeenCalledWith('draft')
    rerender(<></>)
    act(() => store.prune(new Set()))
    expect(store.entries.size).toBe(0)
  })

  it('keeps edits typed while a commit is awaiting confirmation dirty', async () => {
    const store = new InspectorDraftStore()
    let finish!: (result: boolean) => void
    const waiting = new Promise<boolean>(resolve => {
      finish = resolve
    })
    render(
      <InspectorDraftContext.Provider value={{ store, nodeId: 'a' }}>
        <Field name="ID" value="old" commit={() => waiting} />
      </InspectorDraftContext.Provider>,
    )
    fireEvent.change(screen.getByLabelText('ID'), { target: { value: 'submitted' } })
    fireEvent.blur(screen.getByLabelText('ID'))
    fireEvent.change(screen.getByLabelText('ID'), { target: { value: 'later' } })
    await act(async () => {
      finish(true)
      await waiting
    })
    expect(store.dirty).toBe(true)
    expect(screen.getByLabelText('ID')).toHaveValue('later')
  })

  it('does not mark rejected changes clean and refreshes clean fields on undo', async () => {
    const store = new InspectorDraftStore()
    const commit = vi.fn(() => false)
    const view = (value: string) => (
      <InspectorDraftContext.Provider value={{ store, nodeId: 'a' }}>
        <Field name="ID" value={value} commit={commit} />
      </InspectorDraftContext.Provider>
    )
    const { rerender } = render(view('old'))
    rerender(view('undo'))
    expect(screen.getByLabelText('ID')).toHaveValue('undo')
    fireEvent.change(screen.getByLabelText('ID'), { target: { value: 'new' } })
    await act(async () => {
      expect(await store.flush()).not.toBeNull()
    })
    expect(store.dirty).toBe(true)
  })
})
