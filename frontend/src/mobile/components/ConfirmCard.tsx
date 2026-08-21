/**
 * 危险操作确认卡：security_check 事件触发，三选项（允许一次/此对话允许/拒绝）
 * 与桌面 SecurityPrompt 并存——先响应者生效（后端信号队列一次性消费）。
 * 输入类请求（user_input_request）不走此卡，store 已转为系统提示消息。
 */

import { useEffect, useState } from 'react'
import { ShieldAlert } from 'lucide-react'
import { postConfirm } from '../api'
import type { PendingConfirm } from '../store'
import { t } from '../i18n'
import { playPopupSound } from '../../ui/sound'

interface Props {
  confirm: PendingConfirm
  token: string
  /** 提交成功回调：携带 approved 结果，供上层 toast 反馈 */
  onResolved: (approved: boolean) => void
}

const RISK_LABEL: Record<string, () => string> = {
  low: () => t('mobile.riskLow'),
  medium: () => t('mobile.riskMedium'),
  high: () => t('mobile.riskHigh'),
  critical: () => t('mobile.riskCritical'),
}

export default function ConfirmCard({ confirm, token, onResolved }: Props) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // 权限确认弹窗出现即播放提示音
  useEffect(() => {
    playPopupSound('confirm')
  }, [])

  const submit = async (approved: boolean, session: boolean) => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await postConfirm(token, {
        action_id: confirm.actionId,
        approved,
        session,
        tool: confirm.tool,
      })
      onResolved(approved)
    } catch (e) {
      // 提交失败保留卡片（可能已被桌面端处理，下一轮事件会自然清理）
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="mobile-confirm" role="alertdialog" aria-label="危险操作确认">
      <div className="mobile-confirm-head">
        <ShieldAlert size={16} aria-hidden="true" />
        <span className={`mobile-confirm-risk is-${confirm.risk}`}>
          {RISK_LABEL[confirm.risk]?.() ?? confirm.risk}
        </span>
        <span className="mobile-confirm-tool">{confirm.tool}</span>
      </div>
      <p className="mobile-confirm-reason">{confirm.reason}</p>
      {confirm.params && confirm.params !== '{}' && (
        <p className="mobile-confirm-params">{confirm.params}</p>
      )}
      {error && <p className="mobile-confirm-error">{error}</p>}
      <div className="mobile-confirm-actions">
        <button
          type="button"
          className="mobile-confirm-btn is-approve"
          disabled={busy}
          onClick={() => void submit(true, false)}
        >
            {t('mobile.allowOnce')}
        </button>
        <button
          type="button"
          className="mobile-confirm-btn is-approve-session"
          disabled={busy}
          onClick={() => void submit(true, true)}
        >
          {t('mobile.allowSession')}
        </button>
        <button
          type="button"
          className="mobile-confirm-btn is-deny"
          disabled={busy}
          onClick={() => void submit(false, false)}
        >
          {t('mobile.deny')}
        </button>
      </div>
    </div>
  )
}