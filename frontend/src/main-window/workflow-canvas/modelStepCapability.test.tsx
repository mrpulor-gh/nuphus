import { fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import { Inspector } from './Inspector'
import { InspectorDraftContext, InspectorDraftStore } from './inspectorDrafts'
import { WorkflowTraceContext } from './WorkflowTraceContext'

// 只声明了对话能力的模型：没有任何 supports_image_generation —— 用来验证
// 「候选为空时给出去模型界面绑定能力的指引」，不与 modelStep.test.tsx 的
// 混合候选列表抢模块级模型缓存（每个测试文件独立注册表）。
vi.mock('../lib/api', () => ({
  wfTools: vi.fn(async () => []),
  listModels: vi.fn(async () => [
    {
      id: 'deepseek-v4-pro',
      provider: 'deepseek',
      alias: [],
      supports_streaming: true,
      supports_vision: false,
      supports_audio: false,
      supports_image_generation: false,
      reasoning_efforts: [],
    },
  ]),
  wfTraceList: vi.fn(async () => []),
  wfTraceRead: vi.fn(async () => null),
}))

function mountModelStep(step: WorkflowStep) {
  const store = new InspectorDraftStore()
  let current = step
  function Harness() {
    const [value, setValue] = useState(step)
    current = value
    return (
      <WorkflowTraceContext.Provider value={{ workflowId: 'flow' }}>
        <InspectorDraftContext.Provider value={{ store, nodeId: step.id }}>
          <Inspector
            step={value}
            readOnly={false}
            idReferenced={false}
            onClose={vi.fn()}
            onPatch={patch => setValue(old => ({ ...old, ...patch }))}
            onPatchAction={action => setValue(old => ({ ...old, do: action }))}
            variableCatalog={{ references: [], captures: [] }}
          />
        </InspectorDraftContext.Provider>
      </WorkflowTraceContext.Provider>
    )
  }
  render(<Harness />)
  return { store, current: () => current }
}

describe('model step inspector form (no generation-capable candidate)', () => {
  it('guides to the model UI capability binding when the modality has no candidate', async () => {
    mountModelStep({
      id: 'shot',
      name: '生成首帧',
      do: { model: '画猫', with: { kind: 'image' } },
    })
    // 候选为空：不渲染模型下拉，改为给出去模型行打开能力声明的指引
    expect(await screen.findByText(/生成能力候选为空/)).toBeInTheDocument()
    expect(screen.queryByLabelText('选择模型（留空用绑定能力）')).not.toBeInTheDocument()
  })

  it('keeps listing every model for the text modality (no capability filter)', async () => {
    mountModelStep({
      id: 'copy',
      name: '写文案',
      do: { model: '写一句', with: { kind: 'text' } },
    })
    const select = (await screen.findByLabelText(
      '选择模型（留空用绑定能力）',
    )) as HTMLSelectElement
    expect(screen.getByRole('option', { name: 'deepseek-v4-pro · deepseek' })).toBeInTheDocument()
    expect(select).toHaveValue('')
  })
})
