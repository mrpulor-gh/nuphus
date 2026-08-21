/**
 * 执行点评卡（assistant 消息「点评」按钮触发）：
 * 星级 + 可选评语 → POST /rating（复用桌面 submit_execution_rating 记忆评分）。
 * 轻量实现：goal 取消息正文前 80 字，工具摘要从 traceItems 提取，无桌面端工具链元数据。
 */

import { useState } from 'react'
import { postRating } from '../api'
import type { ChatMessage } from '../store'
import { t } from '../i18n'

interface Props {
  message: ChatMessage
  token: string
  onClose: () => void
  /** 提交成功回调（上层 toast 反馈） */
  onSubmitted: () => void
}

const STAR_LABEL: (() => string)[] = [
  () => '',
  () => t('mobile.rating1'),
  () => t('mobile.rating2'),
  () => t('mobile.rating3'),
  () => t('mobile.rating4'),
  () => t('mobile.rating5'),
]

export default function RatingSheet({ message, token, onClose, onSubmitted }: Props) {
  const [rating, setRating] = useState(0)
  const [hover, setHover] = useState(0)
  const [comment, setComment] = useState('')
  const [name, setName] = useState(() => message.content.slice(0, 80) || t('mobile.unnamedTask'))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const toolsSummary = (message.traceItems || [])
    .filter(i => i.kind === 'tool')
    .map(i => (i.kind === 'tool' ? i.name : ''))
    .filter(Boolean)
    .join(', ')

  const submit = async () => {
    if (busy || rating === 0) return
    setBusy(true)
    setError(null)
    try {
      await postRating(token, {
        goal: name.trim() || t('mobile.unnamedTask'),
        rating,
        comment,
        toolsSummary,
        stepsJson: JSON.stringify([]),
        sessionId: '',
      })
      onSubmitted()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <div className="mobile-rating-overlay" onClick={onClose}>
      <div
        className="mobile-rating"
        role="dialog"
        aria-label={t('mobile.ratingTitle')}
        onClick={e => e.stopPropagation()}
      >
        <div className="mobile-rating-head">
          <span className="mobile-rating-title">{t('mobile.ratingTitle')}</span>
        </div>
        <input
          className="mobile-rating-goal"
          value={name}
          onChange={e => setName(e.target.value)}
          placeholder={t('mobile.unnamedTask')}
          aria-label={t('mobile.taskName')}
        />
        <div className="mobile-rating-stars">
          {[1, 2, 3, 4, 5].map(star => (
            <button
              key={star}
              type="button"
              className={`mobile-rating-star${star <= (hover || rating) ? ' active' : ''}`}
              onClick={() => setRating(star)}
              onMouseEnter={() => setHover(star)}
              onMouseLeave={() => setHover(0)}
              aria-label={`${star} ${t('mobile.stars')}`}
            >
              ★
            </button>
          ))}
          <span className="mobile-rating-star-label">{STAR_LABEL[hover || rating]?.()}</span>
        </div>
        <textarea
          className="mobile-rating-comment"
          placeholder={t('mobile.ratingCommentPlaceholder')}
          value={comment}
          onChange={e => setComment(e.target.value)}
          rows={3}
        />
        {error && <p className="mobile-rating-error">{error}</p>}
        <div className="mobile-rating-actions">
          <button type="button" className="mobile-rating-btn is-skip" disabled={busy} onClick={onClose}>
            {t('mobile.cancel')}
          </button>
          <button
            type="button"
            className="mobile-rating-btn is-confirm"
            disabled={busy || rating === 0}
            onClick={() => void submit()}
          >
            {rating >= 4 ? t('mobile.saveAsStrategy') : t('mobile.submitRating')}
          </button>
        </div>
      </div>
    </div>
  )
}