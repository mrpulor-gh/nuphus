/**
 * 会话提炼弹窗：refine_prompt 事件触发，上下文超阈值时询问用户是否提炼。
 * - 确认 → 调 /refine（App 层处理）→ 后端执行提炼
 * - 跳过 → 关闭弹窗（后端在下一轮再次提示）
 * - refining=true 时显示提炼中状态（refine_executing / 手动触发）
 * 与桌面 RefineModal 并存，先响应者生效（refine_active 原子锁后端保证互斥）。
 */

import { Loader2 } from 'lucide-react'
import type { PendingRefine } from '../store'
import { t } from '../i18n'

interface Props {
  refine: PendingRefine
  refining: boolean
  onConfirm: () => void
  onSkip: () => void
}

export default function RefineModal({ refine, refining, onConfirm, onSkip }: Props) {
  const pct =
    refine.currentTokens && refine.contextWindow
      ? Math.round((refine.currentTokens / refine.contextWindow) * 100)
      : 0

  return (
    <div className="mobile-refine" role="alertdialog" aria-label="会话提炼">
      <div className="mobile-refine-head">
        <Loader2 size={16} aria-hidden="true" />
        <span className="mobile-refine-title">上下文即将满载</span>
      </div>
      <p className="mobile-refine-desc">
        当前已使用约 <strong>{pct}%</strong> 上下文（{Math.round(refine.currentTokens / 1000)}K /{' '}
        {refine.contextWindow > 0 ? `${Math.round(refine.contextWindow / 1000)}K` : '--'} tokens）。
        提炼后保留关键信息并精简上下文，会话可继续更长时间。
      </p>
      <div className="mobile-refine-actions">
        <button
          type="button"
          className="mobile-refine-btn is-confirm"
          disabled={refining}
          onClick={onConfirm}
        >
          {refining ? t('mobile.refining') : t('mobile.refine')}
        </button>
        <button
          type="button"
          className="mobile-refine-btn is-skip"
          disabled={refining}
          onClick={onSkip}
        >
          {t('mobile.skip')}
        </button>
      </div>
    </div>
  )
}
