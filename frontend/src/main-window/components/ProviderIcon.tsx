import anthropicSvg from '../../assets/provider-icons/anthropic.svg?raw'
import bytedanceSvg from '../../assets/provider-icons/bytedance.svg?raw'
import deepseekSvg from '../../assets/provider-icons/deepseek.svg?raw'
import googleSvg from '../../assets/provider-icons/google.svg?raw'
import minimaxSvg from '../../assets/provider-icons/minimax.svg?raw'
import moonshotaiSvg from '../../assets/provider-icons/moonshotai.svg?raw'
import openaiSvg from '../../assets/provider-icons/openai.svg?raw'
import opencodeGoSvg from '../../assets/provider-icons/opencode-go.svg?raw'
import openrouterSvg from '../../assets/provider-icons/openrouter.svg?raw'
import qwenSvg from '../../assets/provider-icons/qwen.svg?raw'
import stepfunSvg from '../../assets/provider-icons/stepfun.svg?raw'
import tokenrhythmSvg from '../../assets/provider-icons/tokenrhythm.svg?raw'
import xaiSvg from '../../assets/provider-icons/xai.svg?raw'
import zhipuaiSvg from '../../assets/provider-icons/zhipuai.svg?raw'

/**
 * 模型提供商图标（共享组件）。
 *
 * 图标源：
 * - opencode 开源仓库 packages/ui/src/components/provider-icons/sprite.svg（拆分为单文件）
 * - qwen / bytedance 由用户手动提供（opencode 图标集未覆盖这两个）
 * - stepfun 取官网品牌 logo 的图形部分（www.stepfun.com/assets/logo-B0FsyLQP.svg
 *   的圆形像素方块 path，viewBox 裁到 0 0 36 36；大王 2026-10-19 指定。wordmark
 *   字标不进图标位——16px 方形容器里塞整条横版 logo 不可读）
 * - tokenrhythm 取官网 favicon（tokenrhythm.studio 的 512×512 透明底 TR monogram），
 *   缩放 128×128 后把 alpha 通道转成 <mask>、用 currentColor 填充——官网只有位图
 *   与横版白色 wordmark，直接内嵌位图会在暗色主题下消失
 * 全部为单色 + fill="currentColor"，随 .provider-icon 的 color 继承主题色
 * （用 <img> 会丢失 currentColor，暗色主题下会变黑，故内联渲染）。
 *
 * key 为 Nuphus 的 ProviderKind id（见 src/api/mod.rs），非 opencode 的图标名：
 * kimi → moonshotai（Kimi 属 Moonshot AI）、zhipu → zhipuai。
 *
 * 消费方：ChatPanel 模型管理弹窗的提供商选项（.cmd-modal-provider-icon）。
 * 未收录的 provider（custom / local）无对应图标，返回 null 由调用方回退首字母。
 */
const PROVIDER_ICON: Record<string, string> = {
  openai: openaiSvg,
  anthropic: anthropicSvg,
  google: googleSvg,
  deepseek: deepseekSvg,
  minimax: minimaxSvg,
  openrouter: openrouterSvg,
  kimi: moonshotaiSvg,
  zhipu: zhipuaiSvg,
  qwen: qwenSvg,
  bytedance: bytedanceSvg,
  xai: xaiSvg,
  'opencode-go': opencodeGoSvg,
  stepfun: stepfunSvg,
  tokenrhythm: tokenrhythmSvg,
}

/** 该 provider 是否有图标（调用方据此决定是否回退首字母） */
export function hasProviderIcon(provider: string) {
  return Boolean(PROVIDER_ICON[provider])
}

/** 渲染提供商图标；无图标时返回 null */
export function ProviderIcon({ provider, size = 18 }: { provider: string; size?: number }) {
  const svg = PROVIDER_ICON[provider]
  if (!svg) return null
  return (
    <span
      className="provider-icon"
      style={{ width: size, height: size }}
      aria-hidden="true"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}
