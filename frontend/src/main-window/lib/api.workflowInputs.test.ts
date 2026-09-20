import { beforeEach, describe, expect, it, vi } from 'vitest'

// 只关心「参数如何透传 / IR 如何映射」，bridge 整体打桩，避免真实 IPC
vi.mock('../../core/bridge', () => ({ invoke: vi.fn(async () => null) }))

import { invoke } from '../../core/bridge'
import { listWorkflows, wfRun } from './api'

const mockedInvoke = vi.mocked(invoke)

describe('wfRun 外部输入透传', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockedInvoke.mockResolvedValue(null as never)
  })

  it('第三参 inputs 原样随 wf_run 下发', async () => {
    await wfRun('wf-1', true, { topic: 'a', count: 2 })
    expect(mockedInvoke).toHaveBeenCalledWith('wf_run', {
      id: 'wf-1',
      fresh: true,
      inputs: { topic: 'a', count: 2 },
    })
  })

  it('省略 inputs → 保持既有调用形状（旧调用方兼容）', async () => {
    await wfRun('wf-1')
    expect(mockedInvoke).toHaveBeenCalledWith('wf_run', {
      id: 'wf-1',
      fresh: undefined,
      inputs: undefined,
    })
  })
})

describe('normalizeWorkflow 外部输入声明映射', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  const rawWorkflow = (extra: Record<string, unknown>) => ({
    workflows: [
      {
        id: 'wf-1',
        name: '带输入的工作流',
        status: 'Draft',
        steps: [],
        ...extra,
      },
    ],
  })

  it('后端 inputs 数组映射为前端声明（type/required/default/sensitive/description）', async () => {
    const inputs = [
      {
        name: 'topic',
        type: 'string',
        required: true,
        description: '写什么',
        sensitive: false,
      },
      { name: 'secret', type: 'string', sensitive: true, default: 'd' },
      { name: 'count', type: 'number' },
    ]
    mockedInvoke.mockResolvedValue(rawWorkflow({ inputs }) as never)

    const [wf] = await listWorkflows()
    expect(wf.inputs).toEqual([
      {
        name: 'topic',
        type: 'string',
        required: true,
        default: undefined,
        description: '写什么',
        sensitive: false,
      },
      {
        name: 'secret',
        type: 'string',
        required: false,
        default: 'd',
        description: undefined,
        sensitive: true,
      },
      {
        name: 'count',
        type: 'number',
        required: false,
        default: undefined,
        description: undefined,
        sensitive: false,
      },
    ])
  })

  it('旧工作流（无 inputs / 空数组）→ undefined，不出现空表单区', async () => {
    mockedInvoke.mockResolvedValue(rawWorkflow({}) as never)
    const [legacy] = await listWorkflows()
    expect(legacy.inputs).toBeUndefined()

    mockedInvoke.mockResolvedValue(rawWorkflow({ inputs: [] }) as never)
    const [empty] = await listWorkflows()
    expect(empty.inputs).toBeUndefined()
  })

  it('丢弃无名项与非法形状，保留合法声明', async () => {
    mockedInvoke.mockResolvedValue(
      rawWorkflow({ inputs: [{ type: 'string' }, null, 'x', { name: 'ok' }] }) as never,
    )
    const [wf] = await listWorkflows()
    expect(wf.inputs).toEqual([
      {
        name: 'ok',
        type: undefined,
        required: false,
        default: undefined,
        description: undefined,
        sensitive: false,
      },
    ])
  })
})
