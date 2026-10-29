import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, render, screen, waitFor } from '@testing-library/react'

/** 前置校验的 api 依赖；vi.hoisted 供 vi.mock 工厂安全引用。
 *  VISION_RESOLUTION_CHANGED_EVENT 必须出现在 mock 里——
 *  useVisionResolution 依赖它注册失效监听（策略唯一事实源的失效通道）。 */
const { getVisionResolution } = vi.hoisted(() => ({ getVisionResolution: vi.fn() }))
vi.mock('../lib/api', () => ({
  getVisionResolution,
  VISION_RESOLUTION_CHANGED_EVENT: 'nuphus:vision-resolution-changed',
}))

import ReferenceBar from './ReferenceBar'
import { VISION_RESOLUTION_CHANGED_EVENT } from '../lib/api'

const image = { name: 'shot.png', dataUrl: 'data:image/png;base64,AAAA' }

function renderWithImage() {
  return render(
    <ReferenceBar
      references={[]}
      pendingImages={[image]}
      pendingFiles={[]}
      onRemoveReference={() => {}}
      onRemoveImage={() => {}}
      onRemoveFile={() => {}}
    />,
  )
}

describe('ReferenceBar 图像理解前置校验（共享策略源）', () => {
  afterEach(() => {
    cleanup()
    getVisionResolution.mockReset()
  })

  it('主模型直读图片时不渲染任何能力提示', async () => {
    getVisionResolution.mockResolvedValue({
      strategy: 'main',
      model: 'leader-vision',
      provider: 'p',
      leader_model: 'leader-vision',
    })
    const { container } = renderWithImage()
    await waitFor(() => expect(getVisionResolution).toHaveBeenCalled())
    expect(container.querySelector('.ref-vision-hint')).toBeNull()
  })

  it('capability 路径时提示交给视觉模型按需识别（不带组件层级 {0} 占位）', async () => {
    getVisionResolution.mockResolvedValue({
      strategy: 'capability',
      model: 'vision-x',
      provider: 'p',
      leader_model: 'leader',
    })
    renderWithImage()
    await screen.findByText('图片将由 vision-x 按需识别（主模型上下文不含图片）')
  })

  it('none 时渲染警告提示（宽松放行不在此层，阻断在入栏判定）', async () => {
    getVisionResolution.mockResolvedValue({
      strategy: 'none',
      model: '',
      provider: '',
      leader_model: 'leader',
    })
    const { container } = renderWithImage()
    const warning = await screen.findByText(/当前模型无法识图/)
    expect(warning.className).toContain('ref-vision-hint--warning')
    expect(container.querySelector('.ref-vision-hint')).not.toBeNull()
  })

  it('策略失效事件触发重查——配置保存后提示自动更新（根因回归守卫）', async () => {
    getVisionResolution.mockResolvedValue({
      strategy: 'none',
      model: '',
      provider: '',
      leader_model: 'leader',
    })
    const { container } = renderWithImage()
    await screen.findByText(/当前模型无法识图/)

    // 模拟「设置里保存视觉绑定」：api.ts 包装器 dispatch 失效事件 → hook 重查
    getVisionResolution.mockResolvedValue({
      strategy: 'capability',
      model: 'vision-x',
      provider: 'p',
      leader_model: 'leader',
    })
    await act(async () => {
      window.dispatchEvent(new Event(VISION_RESOLUTION_CHANGED_EVENT))
    })
    await screen.findByText('图片将由 vision-x 按需识别（主模型上下文不含图片）')
    expect(container.querySelector('.ref-vision-hint--warning')).toBeNull()
  })

  it('无待发图片时不展示策略提示（策略源仍保持就绪供入栏判定）', async () => {
    getVisionResolution.mockResolvedValue({
      strategy: 'none',
      model: '',
      provider: '',
      leader_model: 'leader',
    })
    const { container } = render(
      <ReferenceBar
        references={[]}
        pendingImages={[]}
        pendingFiles={[]}
        onRemoveReference={() => {}}
        onRemoveImage={() => {}}
        onRemoveFile={() => {}}
      />,
    )
    expect(container.querySelector('.ref-vision-hint')).toBeNull()
  })
})
