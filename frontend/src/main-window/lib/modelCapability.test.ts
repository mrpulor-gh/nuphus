import { describe, expect, it } from 'vitest'
import type { ModelInfo } from './api'
import { selectableModels } from './modelCapability'

function model(id: string, provider: string, extra: Partial<ModelInfo> = {}): ModelInfo {
  return {
    id,
    provider,
    alias: [],
    supports_streaming: true,
    supports_vision: false,
    supports_audio: false,
    supports_image_generation: false,
    reasoning_efforts: [],
    ...extra,
  }
}

describe('capability model selector candidates', () => {
  const models = [
    model('gpt-4o', 'custom-team-a', { supports_vision: true }),
    model('gpt-4o', 'custom-team-b'),
    model('deepseek-chat', 'deepseek'),
    model('voice-model', 'kimi', { supports_audio: true }),
  ]

  it('lists only vision-capable models for the vision selector', () => {
    const picked = selectableModels(models, 'vision')
    expect(picked.map(m => `${m.provider}/${m.id}`)).toEqual(['custom-team-a/gpt-4o'])
  })

  it('keeps same-name models distinct by provider', () => {
    // 同名模型分别来自两个自定义中转站：能力开关只影响被标记的那一个实例，
    // 另一个不应因为 id 相同而被连带放行。
    const picked = selectableModels(models, 'vision')
    expect(picked.some(m => m.provider === 'custom-team-b')).toBe(false)
  })

  it('lists audio-capable models for the audio selector', () => {
    expect(selectableModels(models, 'audio').map(m => m.id)).toEqual(['voice-model'])
  })

  it('returns every model when no capability filter is given', () => {
    expect(selectableModels(models).length).toBe(models.length)
  })

  it('tolerates missing or malformed input', () => {
    expect(selectableModels(null, 'vision')).toEqual([])
    expect(selectableModels(undefined, 'vision')).toEqual([])
  })
})
