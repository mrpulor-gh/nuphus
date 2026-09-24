import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { StepNode, type StepNodeFlow } from './nodes/StepNode'
import { compactEdgeLabel } from './edges/DataEdge'
import { layoutLayer, NODE_SIZE } from './layout'
import type { CanvasLayer } from './types'
import type { NodeProps } from '@xyflow/react'

vi.mock('@xyflow/react', async importOriginal => ({
  ...(await importOriginal<object>()),
  Handle: () => <span data-testid="handle" />,
}))

describe('node compact layout', () => {
  it('keeps full capture/source explanations and both handles for badge-heavy cards', () => {
    const props = {
      data: {
        canvas: {
          id: 'script',
          name: '脚本',
          kind: 'script',
          category: 'leaf',
          lane: 'main',
          capture: 'very_long_capture_name',
          shadowedBy: 'later',
          danglingVars: ['missing'],
          onErrorLabel: '失败时跳过此步骤',
        },
      },
    } as NodeProps<StepNodeFlow>
    render(<StepNode {...props} />)
    expect(screen.getByTitle('保存输出到 very_long_capture_name')).toBeInTheDocument()
    expect(screen.getByTitle(/未找到来源：missing/)).toBeInTheDocument()
    expect(screen.getAllByTestId('handle')).toHaveLength(2)
    expect(screen.queryByText('外部注入')).not.toBeInTheDocument()
  })
  it('bounds both Chinese and ASCII edge captions', () => {
    expect(compactEdgeLabel('short')).toBe('short')
    expect(compactEdgeLabel('a'.repeat(40))).toBe(`${'a'.repeat(18)}…`)
    expect(compactEdgeLabel('长'.repeat(40))).toBe(`${'长'.repeat(9)}…`)
  })
  it('aligns layout dimensions while retaining manual sidecar positions', () => {
    const layer: CanvasLayer = {
      layerId: 'root',
      parentChain: [],
      containerKind: 'root',
      swimlanes: [{ id: 'main', title: '' }],
      nodes: [
        { id: 'a', name: 'a', kind: 'script', category: 'leaf', lane: 'main' },
        { id: 'b', name: 'b', kind: 'script', category: 'leaf', lane: 'main' },
      ],
      edges: [{ id: 'edge', kind: 'sequence', source: 'a', target: 'b' }],
    }
    const positions = layoutLayer(layer)
    expect(positions.get('b')!.x - positions.get('a')!.x).toBeGreaterThan(NODE_SIZE.leaf.width)
    expect(layoutLayer(layer, { a: { x: 123, y: 456 } }).get('a')).toEqual({ x: 123, y: 456 })
  })
})
