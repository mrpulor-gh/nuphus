import { describe, expect, it } from 'vitest'
import { intentDraftKey, readIntentDraft, writeIntentDraft } from './intentDraft'

describe('intent drafts', () => {
  it('isolates workspaces and workflows, preserving untrimmed user input', () => {
    const stages = [
      { id: 'stage', name: ' unfinished ', steps: [{ id: 'step', intent: ' raw\ntext ' }] },
    ]
    const key = intentDraftKey('workspace1', 'workflow1')
    writeIntentDraft(localStorage, key, stages)
    expect(readIntentDraft(localStorage, key)).toEqual(stages)
    expect(readIntentDraft(localStorage, intentDraftKey('workspace2', 'workflow1'))).toBeNull()
    expect(readIntentDraft(localStorage, intentDraftKey('workspace1', 'workflow2'))).toBeNull()
  })
  it('does not silently replace corrupted or unsupported drafts', () => {
    expect(() => readIntentDraft({ getItem: () => 'broken' }, 'draft')).toThrow()
    expect(() => readIntentDraft({ getItem: () => '{"version":2,"stages":[]}' }, 'draft')).toThrow()
  })
  it('propagates storage failures so the UI cannot claim the draft was saved', () => {
    expect(() =>
      writeIntentDraft(
        {
          setItem: () => {
            throw new Error('quota')
          },
        },
        'key',
        [],
      ),
    ).toThrow('quota')
  })
})
