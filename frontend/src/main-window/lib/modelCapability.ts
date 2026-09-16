import type { ModelInfo } from './api'

/** 能力模型选择器的过滤维度（与后端 capabilities.vision / stt 等键对应） */
export type CapabilityFilter = 'vision' | 'audio'

/**
 * 能力模型选择器的候选集合。
 *
 * 纪律：**能力即过滤条件**。图像理解模型列表只列出 `supports_vision = true` 的模型——
 * `supports_vision` 是用户可配置的（模型行内的视觉能力开关写 providers.toml），
 * 因此「列表里没有我的模型」有明确的自助出口：去模型行打开该开关。
 *
 * 之所以不做「未知能力也放行」的宽松兜底：那会把「确实不支持图片输入」的模型
 * 一并放进图像理解列表，用户选中后要到实际发图时才失败，坏在更晚、更难查。
 * 能力未知 ≠ 能力具备 —— 未知的处置是「让用户显式声明」，不是「当作可用」。
 */
export function selectableModels(
  models: ModelInfo[] | null | undefined,
  filter?: CapabilityFilter,
): ModelInfo[] {
  if (!Array.isArray(models)) return []
  const list =
    filter === 'audio'
      ? models.filter(m => m.supports_audio)
      : filter === 'vision'
        ? models.filter(m => m.supports_vision)
        : [...models]
  // 具备该能力的排在前面（audio 维度按 supports_audio；其余按 supports_vision，
  // 与既有排序行为保持一致，过滤后为稳定排序即无位移）。
  if (filter === 'audio') {
    list.sort((a, b) => (b.supports_audio ? 1 : 0) - (a.supports_audio ? 1 : 0))
  } else {
    list.sort((a, b) => (b.supports_vision ? 1 : 0) - (a.supports_vision ? 1 : 0))
  }
  return list
}
