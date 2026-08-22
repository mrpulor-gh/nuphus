import { useState, useEffect } from 'react'
import { getSupportedProviders, configureLlm } from '../lib/api'
import type { ProviderInfo } from '../lib/api'
import '../../styles/onboarding.css'

interface OnboardingModalProps {
  onComplete: (provider: ProviderInfo) => void
  onSkip: () => void
}

export function OnboardingModal({ onComplete, onSkip }: OnboardingModalProps) {
  const [step, setStep] = useState<'select' | 'configure'>('select')
  const [providers, setProviders] = useState<ProviderInfo[]>([])
  const [selected, setSelected] = useState<ProviderInfo | null>(null)
  const [apiKey, setApiKey] = useState('')
  const [model, setModel] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    getSupportedProviders()
      .then(list => {
        setProviders(list || [])
      })
      .catch(() => {})
  }, [])

  const handleSelect = (p: ProviderInfo) => {
    setSelected(p)
    setModel(p.default_model || '')
    setApiKey('')
    setError('')
    setStep('configure')
  }

  const handleBack = () => {
    setStep('select')
    setError('')
  }

  const handleComplete = async () => {
    if (!apiKey.trim()) {
      setError('请输入 API Key')
      return
    }
    if (!model.trim()) {
      setError('请输入模型名称')
      return
    }
    if (!selected) return
    setLoading(true)
    setError('')
    try {
      await configureLlm(apiKey.trim(), model.trim(), selected.id, selected.base_url)
      localStorage.setItem('nuphus_onboarding_done', 'true')
      // 同步当前 provider 的 model 到 localStorage，供快捷切换弹窗读取
      try {
        localStorage.setItem(`nuphus_current_model_${selected.id}`, model.trim())
      } catch {
        /* localStorage 写入失败不阻塞首装 */
      }
      onComplete(selected)
    } catch (e: unknown) {
      const msg =
        e instanceof Error
          ? e.message
          : typeof e === 'string'
            ? e
            : '配置失败，请检查 API Key 和网络'
      setError(msg)
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="onboarding-overlay">
      <div className="onboarding-card">
        {step === 'select' ? (
          <>
            <div className="onboarding-header">
              <h2>欢迎使用 Nuphus</h2>
              <p>请选择一个模型厂商开始</p>
            </div>
            <div className="onboarding-provider-list">
              {providers.map(p => (
                <button
                  key={p.id}
                  className="onboarding-provider-btn"
                  onClick={() => handleSelect(p)}
                >
                  <span className="onboarding-provider-name">{p.name}</span>
                  <span className="onboarding-provider-model">{p.default_model || '自定义'}</span>
                </button>
              ))}
            </div>
            <div className="onboarding-footer">
              <button className="onboarding-skip" onClick={onSkip}>
                跳过，稍后配置
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="onboarding-header">
              <button className="onboarding-back" onClick={handleBack}>
                ← 返回
              </button>
              <h2>{selected?.name}</h2>
            </div>
            <div className="onboarding-form">
              <label className="onboarding-label">
                API Key
                <input
                  className="onboarding-input"
                  type="password"
                  value={apiKey}
                  onChange={e => setApiKey(e.target.value)}
                  placeholder="输入您的 API Key"
                  autoFocus
                  onKeyDown={e => e.key === 'Enter' && handleComplete()}
                />
              </label>
              <label className="onboarding-label">
                模型名称
                <input
                  className="onboarding-input"
                  type="text"
                  value={model}
                  onChange={e => setModel(e.target.value)}
                  placeholder="如 gpt-4o, claude-sonnet-4"
                  onKeyDown={e => e.key === 'Enter' && handleComplete()}
                />
              </label>
              {error && <div className="onboarding-error">{error}</div>}
            </div>
            <div className="onboarding-footer">
              <button className="onboarding-skip" onClick={onSkip}>
                跳过
              </button>
              <button className="onboarding-submit" onClick={handleComplete} disabled={loading}>
                {loading ? '配置中...' : '完成'}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
