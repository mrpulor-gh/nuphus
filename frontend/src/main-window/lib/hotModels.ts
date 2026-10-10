/**
 * hotModels — 模型设置页「热门模型」快捷 chip 的内置清单。
 *
 * chip 存两段信息：展示名（label）+ 用于**子串匹配**的关键词片段（match）。
 * 为什么不直接存完整模型 id：模型多经中转站接入，id 前缀会变
 * （如 `anthropic/claude-opus-4.6`），存 `claude-opus` 这类片段才能命中；
 * 点击 chip = `setFilterInput(match)`，复用列表既有的 `includes` 子串筛选。
 *
 * 纯展示常量，不参与模型解析 / 切换 / 偏好读写。
 */
export type HotModelChip = {
  /** chip 上的展示名（品牌名，无需 i18n）。 */
  label: string
  /** 填进筛选框的关键词片段（子串匹配）。 */
  match: string
}

export const HOT_MODELS: readonly HotModelChip[] = [
  { label: 'Claude Opus', match: 'claude-opus' },
  { label: 'GPT', match: 'gpt-5' },
  { label: 'Gemini', match: 'gemini' },
  { label: 'DeepSeek', match: 'deepseek-v4' },
  { label: 'Grok', match: 'grok' },
  { label: 'Qwen', match: 'qwen' },
]
