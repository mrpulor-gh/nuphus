import { describe, expect, it } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import { profileStep } from './dataEdges'
import { projectWorkflow } from './projection'

describe('workflow input anchors', () => {
  it('maps inputs.name references to the declared field name', () => {
    const step: WorkflowStep = {
      id: 'consume',
      name: 'Consume',
      do: { tool: 'Read', with: { path: '{{inputs.topic}}' } },
    }
    expect(profileStep(step).consumes).toEqual([{ varName: 'topic', pipes: [] }])
  })

  it('renders declared inputs even when they are unused', () => {
    const projection = projectWorkflow({
      steps: [],
      inputs: [{ name: 'topic', type: 'string' }],
    })
    const anchor = projection.layers
      .get('root')
      ?.nodes.find(node => node.externalVar === 'topic')
    expect(anchor).toMatchObject({
      name: '外部 · topic',
      externalInput: true,
      externalInputDeclared: true,
    })
  })

  it('marks dangling variables as configurable undeclared inputs', () => {
    const projection = projectWorkflow({
      steps: [
        {
          id: 'consume',
          name: 'Consume',
          do: { tool: 'Read', with: { path: '{{missing}}' } },
        },
      ],
    })
    const anchor = projection.layers
      .get('root')
      ?.nodes.find(node => node.externalVar === 'missing')
    expect(anchor).toMatchObject({ externalInput: true, externalInputDeclared: false })
  })
})
