import { useLanguage } from '../../locales'
import type { VisionResolution } from '../lib/api'

/** 视觉策略分支语义的**唯一收敛点**。
 *
 * 'main'/'capability'/'none' 三态的判定与文案在这里只写一次；
 * 引用栏提示与气泡占位是同一语义的两种展示形态（variant），禁止在
 * 消费方各自复制分支——那是同一事实多副本的漂移源。
 *
 * null resolution = 未知（查询失败/未查到）：按既有行为宽松放行，
 * 后端 to_api_messages 仍按真实能力分流。 */

/** 图片能否进入待发列表。none = 当前链路完全不识图 → 入栏即拒。 */
export function canAttachImages(resolution: VisionResolution | null): boolean {
  return resolution === null || resolution.strategy !== 'none'
}

/** 气泡里图片能否直接渲染为 <img>。null 未知按可读渲染（既有行为）；
 *  capability/none 时图片走路径注入，主模型上下文不含图片，不直读。 */
export function showsImagesInline(resolution: VisionResolution | null): boolean {
  return resolution === null || resolution.strategy === 'main'
}

export function VisionHint({
  resolution,
  variant,
}: {
  resolution: VisionResolution | null
  variant: 'refbar' | 'bubble'
}) {
  const { t } = useLanguage()

  // 未知 / 主模型直读：不需要任何提示（主模型看得见图，或后端会如实分流）
  if (resolution === null || resolution.strategy === 'main') return null

  if (variant === 'refbar') {
    return (
      <div
        className={`ref-vision-hint${resolution.strategy === 'none' ? ' ref-vision-hint--warning' : ''}`}
        role="status"
      >
        {resolution.strategy === 'none'
          ? t('chat.visionHintNone')
          : t('chat.visionHintCapability', resolution.model)}
      </div>
    )
  }

  // bubble：主模型不读图时，历史/在途消息的图片区显示占位而非破图。
  // 文案面向用户（图片交给谁识别），不暴露路径注入等传输机制。
  return (
    <div className="msg-files-placeholder">
      {resolution.strategy === 'none'
        ? t('chat.visionBubbleNone')
        : t('chat.visionBubbleCapability', resolution.model)}
    </div>
  )
}
