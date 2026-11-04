import { act, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import type { NodeProps } from '@xyflow/react'
import type { WorkflowStep } from '../../core/types'
import { LangProvider } from '../../locales'
import type { VariableCatalog } from './variableCatalog'
import { Inspector } from './Inspector'
import { InspectorDraftContext, InspectorDraftStore } from './inspectorDrafts'
import { WorkflowTraceContext } from './WorkflowTraceContext'
import { KIND_ICONS, StepNode, type StepNodeFlow } from './nodes/StepNode'
import { newStep } from './irEdit'
import { stepKind } from './projection'
import { nodeKindLabel, defaultCanvasTranslate } from './presentation'

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
    {
      id: 'image-01',
      provider: 'minimax',
      alias: [],
      supports_streaming: true,
      supports_vision: false,
      supports_audio: false,
      supports_image_generation: true,
      reasoning_efforts: [],
    },
    {
      id: 'MiniMax-H3',
      provider: 'minimax',
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

vi.mock('@xyflow/react', async importOriginal => ({
  ...(await importOriginal<object>()),
  Handle: () => <span data-testid="handle" />,
}))

function mountModelStep(step: WorkflowStep, catalog?: VariableCatalog) {
  const store = new InspectorDraftStore()
  let current = step
  const patched = vi.fn()
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
            onPatchAction={action => {
              patched(action)
              setValue(old => ({ ...old, do: action }))
            }}
            variableCatalog={catalog ?? { references: [], captures: [] }}
          />
        </InspectorDraftContext.Provider>
      </WorkflowTraceContext.Provider>
    )
  }
  const rendered = render(<Harness />)
  return { store, current: () => current, patched, unmount: rendered.unmount }
}

describe('model step node', () => {
  it('registers a dedicated icon and renders the model node with its localized label', () => {
    expect(KIND_ICONS.model).toBeTruthy()
    const node = {
      data: {
        canvas: {
          id: 'shot',
          name: '生成首帧',
          kind: 'model' as const,
          category: 'leaf' as const,
          lane: 'main' as const,
          capture: 'frame',
        },
      },
    } as NodeProps<StepNodeFlow>
    const { container } = render(
      <LangProvider>
        <StepNode {...node} />
      </LangProvider>,
    )
    const icon = container.querySelector('.wfc-node-icon')
    expect(icon).toHaveAttribute('data-kind', 'model')
    expect(icon?.querySelector('svg')).not.toBeNull()
    expect(screen.getByText('模型')).toBeInTheDocument()
    expect(screen.getByText('→ frame')).toBeInTheDocument()
  })

  it('creates a text-kind skeleton and projects it back to the model kind', () => {
    const step = newStep('model', new Set())
    expect(step.do).toEqual({ model: '', with: { kind: 'text' } })
    expect(stepKind(step)).toBe('model')
    expect(nodeKindLabel('model', defaultCanvasTranslate)).toBe('模型')
  })
})

describe('model step inspector form', () => {
  it('renders the three-way generation type selector and writes the choice back to with.kind', () => {
    const { current } = mountModelStep({
      id: 'shot',
      name: '生成首帧',
      do: { model: '一只橘猫趴在窗台', with: { kind: 'image' } },
    })
    expect(screen.getByRole('radio', { name: '图片' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('radio', { name: '文本' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: '视频' })).toBeInTheDocument()
    // image 模态：主区只有画面规格，不出现 text 专属字段与 params 裸 JSON 框
    expect(screen.queryByRole('textbox', { name: /系统提示词/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: /参数透传/ })).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('radio', { name: '视频' }))
    expect(current().do).toMatchObject({ with: { kind: 'video' } })
    // 切类型不清空：提示词原样保留
    expect(current().do).toMatchObject({ model: '一只橘猫趴在窗台' })
  })

  it('shows only the current modality frame settings and keeps advanced collapsed', () => {
    const { unmount } = mountModelStep({
      id: 'shot',
      name: '生成首帧',
      do: { model: '画猫', with: { kind: 'image' } },
    })
    // image：画面比例 7 项 + 生成数量 4 项；不出现时长 / 首帧图
    expect(screen.getByRole('radiogroup', { name: '画面比例' })).toBeInTheDocument()
    expect(screen.getAllByRole('radio', { name: /^\d+:\d+$/ })).toHaveLength(7)
    expect(screen.getByRole('radiogroup', { name: '生成数量' })).toBeInTheDocument()
    expect(screen.queryByRole('radiogroup', { name: '时长' })).not.toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: /首帧图/ })).not.toBeInTheDocument()
    // 高级默认折叠：可视化选项与逃生舱都在同一个高级 details 里，未展开
    expect(screen.getByText('高级设置').closest('details')).not.toHaveAttribute('open')
    unmount()

    const video = mountModelStep({
      id: 'clip',
      name: '拍猫',
      do: { model: '拍猫', with: { kind: 'video' } },
    })
    // video：比例只剩常用 3 项 + 时长 + 首帧图；不出现生成数量
    expect(screen.getAllByRole('radio', { name: /^\d+:\d+$/ })).toHaveLength(3)
    expect(screen.getByRole('radiogroup', { name: '时长' })).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: /首帧图/ })).toBeInTheDocument()
    expect(screen.queryByRole('radiogroup', { name: '生成数量' })).not.toBeInTheDocument()
    video.unmount()

    mountModelStep({ id: 'copy', name: '写文案', do: { model: '写文案', with: { kind: 'text' } } })
    // text：不渲染画面规格区
    expect(screen.queryByRole('radiogroup', { name: '画面比例' })).not.toBeInTheDocument()
    expect(screen.queryByRole('radiogroup', { name: '时长' })).not.toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: /首帧图/ })).not.toBeInTheDocument()
    // text 专属三件套在高级里（默认折叠）
    expect(screen.getByRole('textbox', { name: /系统提示词/ })).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: /随机程度/ })).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: /最大输出长度/ })).toBeInTheDocument()
  })

  it('writes ratio / count / duration into the named with fields', () => {
    const { current } = mountModelStep({
      id: 'shot',
      name: '生成首帧',
      do: { model: '画猫', with: { kind: 'image' } },
    })
    fireEvent.click(screen.getByRole('radio', { name: '4:3' }))
    fireEvent.click(screen.getByRole('radio', { name: '3' }))
    expect(current().do).toMatchObject({ with: { kind: 'image', ratio: '4:3', count: 3 } })

    fireEvent.click(screen.getByRole('radio', { name: '视频' }))
    fireEvent.click(screen.getByRole('radio', { name: '9:16' }))
    fireEvent.click(screen.getByRole('radio', { name: '10s' }))
    expect(current().do).toMatchObject({
      with: { kind: 'video', ratio: '9:16', duration: 10, count: 3 },
    })
    // 跨模态残留值不清空（count 是 image 字段，切到 video 仍留在 with 里）
  })

  it('writes first_frame and supports variable references', async () => {
    const catalog: VariableCatalog = {
      references: [{ name: 'frame', source: 'capture', sourceLabel: '上一步', maybeUnset: false }],
      captures: [],
    }
    // 1) 手填路径直接落具名字段
    const { store, current, unmount } = mountModelStep(
      { id: 'clip', name: '拍猫', do: { model: '拍猫', with: { kind: 'video' } } },
      catalog,
    )
    const firstFrame = screen.getByRole('textbox', { name: /首帧图/ }) as HTMLInputElement
    // 技术名只作次要说明（title），可见文案是业务用语
    expect(screen.getByTitle('first_frame')).toBeInTheDocument()
    fireEvent.change(firstFrame, { target: { value: 'C:/shots/frame.png' } })
    firstFrame.blur()
    await store.flush()
    expect(current().do).toMatchObject({ with: { first_frame: 'C:/shots/frame.png' } })
    unmount()

    // 2) 复用 VariablePicker 插入 {{变量}}（与提示词同一套引用能力）
    const inserted = mountModelStep(
      { id: 'clip', name: '拍猫', do: { model: '拍猫', with: { kind: 'video' } } },
      catalog,
    )
    fireEvent.click(screen.getByRole('combobox', { name: '为 首帧图 插入变量' }))
    fireEvent.click(screen.getByRole('option', { name: /frame/ }))
    await act(async () => {
      expect(await inserted.store.flush()).toBeNull()
    })
    expect(inserted.current().do).toMatchObject({ with: { first_frame: '{{frame}}' } })
  })

  it('writes the prompt back to do.model and keeps unknown with keys', async () => {
    const { store, current } = mountModelStep({
      id: 'copy',
      name: '写文案',
      do: { model: '初稿', with: { kind: 'text', unknown_future_key: 7 } },
    })
    const prompt = screen.getByRole('textbox', {
      name: /提示词（任务描述）/,
    }) as HTMLTextAreaElement
    fireEvent.change(prompt, { target: { value: '为 {{topic}} 写一句卖点' } })
    prompt.blur()
    await store.flush()
    expect(current().do).toMatchObject({ model: '为 {{topic}} 写一句卖点' })
    // 合并语义：表单未列出的键不丢失
    expect((current().do as { with?: Record<string, unknown> }).with).toMatchObject({
      unknown_future_key: 7,
    })
  })

  it('filters the model list by modality through selectableModels', async () => {
    const { current } = mountModelStep({
      id: 'shot',
      name: '生成首帧',
      do: { model: '一只橘猫趴在窗台', with: { kind: 'image' } },
    })
    const select = (await screen.findByLabelText('选择模型（留空用绑定能力）')) as HTMLSelectElement
    // image → image_generation：只有声明了生成能力的 image-01
    expect(screen.getByRole('option', { name: 'image-01 · minimax' })).toBeInTheDocument()
    expect(screen.queryByRole('option', { name: /deepseek-v4-pro/ })).not.toBeInTheDocument()
    expect(select).toHaveValue('')
    // 节点只选模型：一次写回 model + provider 精确路由对
    fireEvent.change(select, { target: { value: 'minimax::image-01' } })
    expect(current().do).toMatchObject({
      with: { kind: 'image', model: 'image-01', provider: 'minimax' },
    })
    // video → video_generation（复用 image 生成能力声明）：未声明生成能力的模型不列
    fireEvent.click(screen.getByRole('radio', { name: '视频' }))
    expect(screen.queryByRole('option', { name: 'MiniMax-H3 · minimax' })).not.toBeInTheDocument()
    expect(screen.getByText('视频生成能力以模型界面的「视频生成」绑定为准。')).toBeInTheDocument()
    // text → 不过滤：全部模型可选
    fireEvent.click(screen.getByRole('radio', { name: '文本' }))
    expect(screen.getByRole('option', { name: 'MiniMax-H3 · minimax' })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: 'deepseek-v4-pro · deepseek' })).toBeInTheDocument()
  })

  it('retains a saved model that is not in the current modality candidate list', async () => {
    mountModelStep({
      id: 'shot',
      name: '生成首帧',
      do: {
        model: '画猫',
        with: { kind: 'image', model: 'MiniMax-H3', provider: 'minimax' },
      },
    })
    const select = (await screen.findByLabelText('选择模型（留空用绑定能力）')) as HTMLSelectElement
    expect(select).toHaveValue('minimax::MiniMax-H3')
    expect(
      screen.getByRole('option', { name: /MiniMax-H3（不在当前模态的模型列表中，已保留）/ }),
    ).toBeInTheDocument()
  })

  it('shows text-only fields for the text modality and the capture field', () => {
    mountModelStep({
      id: 'copy',
      name: '写文案',
      do: { model: '写一句', with: { kind: 'text' } },
    })
    // text 三件套与逃生舱都在默认折叠的高级区里（T2b 起合并为单一高级分区）
    expect(screen.getByRole('textbox', { name: /系统提示词/ })).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: /随机程度/ })).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: /最大输出长度/ })).toBeInTheDocument()
    expect(screen.getByText('高级设置').closest('details')).not.toHaveAttribute('open')
    expect(screen.queryByLabelText('生成参数 params（JSON）')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('轮询超时 timeout_secs（秒，默认 300）')).not.toBeInTheDocument()
    // 旧的「其他对话选项（JSON）」裸 JSON 框不在 model 主区
    expect(screen.queryByLabelText('其他对话选项（JSON）')).not.toBeInTheDocument()
    // capture 段对 model 节点开放（主产物入库）
    expect(screen.getByLabelText('保存输出到变量')).toBeInTheDocument()
  })
})
