/**
 * 配对引导页：无 token / token 失效（401）时展示
 * 密码为主入口：输入桌面端设置的配对密码 → POST /pair → 换取 token 回传 App 保存。
 * 密码错误 / 锁定倒计时 / 未设置密码 / 网络错误 四种失败均有明确提示。
 */

import { useEffect, useState } from 'react'
import { postPair, PairError } from '../api'
import { t } from '../i18n'

interface Props {
  /** true = 曾经配对但 token 被拒（已重置或失效） */
  invalid: boolean
  /** 配对成功回调：postPair 已换到 token，回传由 App 保存（App 不接触密码） */
  onPair: (token: string) => void
}

export default function PairingGuide({ invalid, onPair }: Props) {
  const [password, setPassword] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  /** 429 防破解锁定倒计时秒数；>0 时禁用输入与按钮，每秒递减 */
  const [lockSec, setLockSec] = useState(0)

  useEffect(() => {
    if (lockSec <= 0) return
    const id = setInterval(() => setLockSec(s => Math.max(0, s - 1)), 1000)
    return () => clearInterval(id)
    // 依赖刻意用布尔量：只在「开始倒计时」与「倒计时归零」两个时机重建 interval
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lockSec > 0])

  const handleSubmit = async () => {
    const pwd = password.trim()
    if (!pwd || submitting || lockSec > 0) return
    setSubmitting(true)
    setError(null)
    try {
      const token = await postPair(pwd)
      onPair(token)
    } catch (e) {
      if (e instanceof PairError) {
        if (e.kind === 'locked') {
          const sec = e.retryAfterSec ?? 60
          setLockSec(sec)
          setError(`${t('mobile.pairTooManyAttempts')} ${sec} ${t('mobile.secondsRetry')}`)
        } else if (e.kind === 'wrong_password') {
          setError(t('mobile.pairWrongPassword'))
        } else if (e.kind === 'no_password') {
          setError(t('mobile.pairNoPassword'))
        } else {
          setError(t('mobile.pairNetworkError'))
        }
      } else {
        setError(t('mobile.pairNetworkError'))
      }
    } finally {
      setSubmitting(false)
    }
  }

  const disabled = submitting || lockSec > 0

  return (
    <div className="mobile-pair">
      <svg
        className="mobile-pair-logo"
        viewBox="0 0 256 256"
        fill="none"
        role="img"
        aria-label="Nuphus"
      >
        <g stroke="currentColor" strokeWidth={24} strokeLinecap="round" fill="none">
          <path d="M64 20 H192 A44 44 0 0 1 236 64 V156" />
          <path d="M200 236 H64 A44 44 0 0 1 20 192 V64 A44 44 0 0 1 64 20" />
          <path d="M80 180 L80 76 M176 180 L176 76" />
        </g>
      </svg>
      <h1 className="mobile-pair-title">{t('mobile.pairTitle')}</h1>
      {invalid ? (
        <p className="mobile-pair-invalid">{t('mobile.pairInvalid')}</p>
      ) : (
        <p className="mobile-pair-desc">{t('mobile.pairDesc')}</p>
      )}
      <ol className="mobile-pair-steps">
        <li>{t('mobile.pairStep1')}</li>
        <li>{t('mobile.pairStep2')}</li>
        <li>{t('mobile.pairStep3')}</li>
      </ol>
      <div className="mobile-pair-form">
        <input
          type="password"
          className="mobile-pair-input"
          value={password}
          onChange={e => setPassword(e.target.value)}
          placeholder={t('mobile.pairInputPlaceholder')}
          autoComplete="off"
          autoCapitalize="none"
          autoCorrect="off"
          disabled={disabled}
          aria-label={t('mobile.pairInputLabel')}
          onKeyDown={e => {
            if (e.key === 'Enter') void handleSubmit()
          }}
        />
        <button
          type="button"
          className="mobile-pair-submit"
          disabled={password.trim().length === 0 || disabled}
          onClick={() => void handleSubmit()}
        >
          {lockSec > 0
            ? `${lockSec} ${t('mobile.secondsRetry')}`
            : submitting
              ? t('mobile.connecting')
              : t('mobile.connect')}
        </button>
      </div>
      {error && <p className="mobile-pair-error">{error}</p>}
    </div>
  )
}
