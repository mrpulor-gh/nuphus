import { describe, expect, it } from 'vitest'
import { mergeEditorProblems } from './editorProblems'
import { editorFieldId, focusEditorField } from './editorFields'

describe('actionable diagnostics', () => {
  it('keeps different variables separate and merges aggregate compiler input diagnostics', () => {
    const local = ['a', 'b'].map(subject => ({
      rule: 'input_reference',
      subject,
      level: 'error' as const,
      message: subject,
      stepId: 's',
      fieldPath: '/do/with/text',
    }))
    const issues = mergeEditorProblems(local, {
      errors: [],
      warnings: [],
      diagnostics: [
        {
          code: 'input_reference',
          subject: 'a',
          category: 'variable',
          severity: 'error',
          field_path: '/inputs',
          detail: 'backend',
        },
      ],
    })
    expect(issues).toHaveLength(2)
    expect(issues[0].details).toEqual(['a', 'backend'])
    expect(issues[0].fieldPath).toBe('/do/with/text')
  })
  it('merges the same rule and location, retaining raw details', () => {
    const issues = mergeEditorProblems(
      [{ rule: 'V12', level: 'error', message: 'local', stepId: 'call' }],
      {
        errors: ['backend'],
        warnings: [],
        diagnostics: [
          {
            code: 'required',
            category: 'missing',
            severity: 'error',
            step_id: 'call',
            field_path: '/do/call',
            detail: 'backend',
          },
        ],
      },
    )
    expect(issues).toHaveLength(1)
    expect(issues[0].details).toEqual(['local', 'backend'])
  })
  it('does not invent locations for legacy strings', () => {
    const issues = mergeEditorProblems([], { errors: ['step x: failed'], warnings: [] })
    expect(issues[0].stepId).toBeUndefined()
    expect(issues[0].fieldPath).toBeUndefined()
  })
  it('uses stable field identities and opens advanced fields for navigation', () => {
    expect(editorFieldId('步骤标识 ID')).toBe(
      editorFieldId('步骤标识 ID（有历史记录，修改需确认）'),
    )
    const panel = document.createElement('div')
    panel.innerHTML = '<details><label data-draft-field="/timeout_secs"><input /></label></details>'
    document.body.append(panel)
    expect(focusEditorField(panel, '/timeout_secs')).toBe(true)
    expect(panel.querySelector('details')?.open).toBe(true)
    expect(document.activeElement).toBe(panel.querySelector('input'))
    panel.remove()
  })
})
