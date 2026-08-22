/**
 * request_user_input 输入卡（user_input_request 事件触发）：
 * - text 类型：标题 + 提示 + 输入框（sensitive 用密码框），提交后 agent 继续执行
 * - 其他类型（screenshot/region/mouse_pos/color/icon_confirm）：依赖桌面截图/坐标能力，
 *   手机端提示去桌面完成，仅提供取消
 * 与桌面 UserInputPrompt 并存——先响应者生效（后端信号一次性消费，重复提交幂等）。
 */

import { useEffect, useState } from 'react'
import { postUserInput, postUserInputReject } from '../api'
import MobileMarkdown from './MobileMarkdown'
import type { PendingUserInput } from '../store'
import { t } from '../i18n'
import { playPopupSound } from '../../ui/sound'

interface Props {
  input: PendingUserInput
  token: string
  /** 处理完成回调（submitted=true 且 text 类时携带提交值） */
  onResolved: (submitted: boolean, value?: string) => void
}

export default function UserInputCard({ input, token, onResolved }: Props) {
  const [value, setValue] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const isText = input.inputType === 'text'

  // 弹窗出现即播放请求音效
  useEffect(() => {
    playPopupSound('request')
  }, [])

  const submit = async () => {
    if (busy || (isText && !value.trim())) return
    setBusy(true)
    setError(null)
    try {
      await postUserInput(token, input.actionId, value)
      onResolved(true, value)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  const reject = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await postUserInputReject(token, input.actionId)
      onResolved(false)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <div className="mobile-user-input" role="dialog" aria-label={input.title}>
      <div className="mobile-user-input-head">
        <span className="mobile-user-input-title">{input.title}</span>
      </div>
      <div className="mobile-user-input-desc">
        <MobileMarkdown content={input.prompt} />
      </div>
      {isText ? (
        <input
          className="mobile-user-input-field"
          type={input.sensitive ? 'password' : 'text'}
          value={value}
          onChange={e => setValue(e.target.value)}
          placeholder={
            input.sensitive ? t('mobile.inputPlaceholderSensitive') : t('mobile.inputPlaceholder')
          }
          autoFocus
          enterKeyHint="done"
        />
      ) : (
        <p className="mobile-user-input-desc-alt">
          {t('mobile.inputTypeDesktopHint')}（{input.inputType}）
        </p>
      )}
      {error && <p className="mobile-user-input-error">{error}</p>}
      <div className="mobile-user-input-actions">
        <button
          type="button"
          className="mobile-user-input-btn is-skip"
          disabled={busy}
          onClick={() => void reject()}
        >
          {t('mobile.cancel')}
        </button>
        {isText && (
          <button
            type="button"
            className="mobile-user-input-btn is-confirm"
            disabled={busy || !value.trim()}
            onClick={() => void submit()}
          >
            {t('mobile.submit')}
          </button>
        )}
      </div>
    </div>
  )
}
