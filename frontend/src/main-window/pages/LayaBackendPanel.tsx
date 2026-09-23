/**
 * Laya 决策后端面板 —— 「增强判断模型」页内与 Jev 并列的后端选项。
 *
 * 本组件不渲染自己的 Section：它由 JevSettings 在同一 Section 内按 tab 切换，
 * 复用既有的 .jev-* 样式，避免第二套视觉实现。
 *
 * Laya（github.com/NandhaKishorM/laya）是独立模型，只是恰好兼容 Jev 的
 * /v1/systemone 协议。两者各自独立配置，同一时刻只有一个生效
 * （后端 ModelRegistry::decision_backend：Jev 已配 Key 时优先）。
 */
import { useEffect, useState } from 'react'
import { IconBrushCleaning, IconEye, IconEyeOff, IconPlug } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { FormRow } from '../../ui/PageLayout'
import { CompactModal } from '../layout/CompactModal'
import {
  clearLayaApiKey,
  getLayaConfig,
  saveLayaConfig,
  testLayaConnection,
  type LayaConnectionStatus,
} from '../lib/api'

const DEFAULT_BASE_URL = 'http://127.0.0.1:8000'
const DEFAULT_TIMEOUT_MS = 10_000
const DEFAULT_MAX_RETRIES = 2

function connectionMessage(result: LayaConnectionStatus | string): string {
  if (typeof result === 'string') return result
  if (result.message) return result.message
  const status = result.status.toLowerCase()
  if (['ok', 'ready', 'available', 'connected'].includes(status)) {
    return result.model ? `服务可达（路由到 ${result.model}）` : '服务可达'
  }
  return `连接状态：${result.status}`
}

function connectionOk(result: LayaConnectionStatus | string): boolean {
  if (typeof result === 'string') {
    return ['ok', 'ready', 'available', 'connected', 'success'].includes(result.toLowerCase())
  }
  return ['ok', 'ready', 'available', 'connected', 'success'].includes(result.status.toLowerCase())
}

export function LayaBackendPanel() {
  const [baseUrl, setBaseUrl] = useState(DEFAULT_BASE_URL)
  const [model, setModel] = useState('')
  const [enabled, setEnabled] = useState(false)
  const [apiKey, setApiKey] = useState('')
  const [hasKey, setHasKey] = useState(false)
  const [showKey, setShowKey] = useState(false)
  const [timeoutMs, setTimeoutMs] = useState(String(DEFAULT_TIMEOUT_MS))
  const [maxRetries, setMaxRetries] = useState(String(DEFAULT_MAX_RETRIES))
  const [fallbackToPrimaryModel, setFallbackToPrimaryModel] = useState(true)

  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [showClearConfirm, setShowClearConfirm] = useState(false)
  const [feedback, setFeedback] = useState<{ ok: boolean; message: string } | null>(null)

  useEffect(() => {
    let alive = true
    void getLayaConfig()
      .then(config => {
        if (!alive || !config) return
        setBaseUrl(config.base_url || DEFAULT_BASE_URL)
        setModel(config.model || '')
        setEnabled(Boolean(config.enabled))
        setHasKey(Boolean(config.has_key))
        setTimeoutMs(String(config.timeout_ms ?? DEFAULT_TIMEOUT_MS))
        setMaxRetries(String(config.max_retries ?? DEFAULT_MAX_RETRIES))
        setFallbackToPrimaryModel(config.fallback_to_primary_model !== false)
      })
      .catch(() => {
        // 读取失败不打断首次配置：默认值本身就是可用的初始状态
        // （本机 8000、无 Key、10s 超时）。真的用不了会在保存或测试时暴露，
        // 那时用户已经明确表达了意图，报错才有意义。
      })
      .finally(() => alive && setLoading(false))
    return () => {
      alive = false
    }
  }, [])

  const validate = (): boolean => {
    if (!baseUrl.trim()) {
      setFeedback({ ok: false, message: '请输入 Laya 服务地址' })
      return false
    }
    const timeout = Number(timeoutMs)
    if (!Number.isFinite(timeout) || timeout < 100 || timeout > 120_000) {
      setFeedback({ ok: false, message: '请求超时必须介于 100 与 120000 毫秒之间' })
      return false
    }
    const retries = Number(maxRetries)
    if (!Number.isInteger(retries) || retries < 0 || retries > 10) {
      setFeedback({ ok: false, message: '最大重试次数必须是 0 到 10 之间的整数' })
      return false
    }
    return true
  }

  const save = async (): Promise<void> => {
    setFeedback(null)
    if (!validate()) return
    setSaving(true)
    try {
      const config = await saveLayaConfig({
        apiKey: apiKey.trim() || undefined,
        baseUrl: baseUrl.trim(),
        model: model.trim(),
        enabled,
        timeoutMs: Number(timeoutMs),
        maxRetries: Number(maxRetries),
        fallbackToPrimaryModel,
      })
      if (!config) throw new Error('后端未返回 Laya 配置')
      setHasKey(Boolean(config.has_key))
      setApiKey('')
      setFeedback({ ok: true, message: 'Laya 配置已保存' })
    } catch (error) {
      setFeedback({ ok: false, message: String(error) })
    } finally {
      setSaving(false)
    }
  }

  const test = async (): Promise<void> => {
    setFeedback(null)
    if (!validate()) return
    setTesting(true)
    try {
      const result = await testLayaConnection()
      if (!result) throw new Error('后端未返回连接测试结果')
      setFeedback({ ok: connectionOk(result), message: connectionMessage(result) })
    } catch (error) {
      setFeedback({ ok: false, message: String(error) })
    } finally {
      setTesting(false)
    }
  }

  const clear = async (): Promise<void> => {
    setClearing(true)
    try {
      await clearLayaApiKey()
      setHasKey(false)
      setApiKey('')
      setShowClearConfirm(false)
      setFeedback({ ok: true, message: 'Laya API Key 已清除' })
    } catch (error) {
      setFeedback({ ok: false, message: String(error) })
    } finally {
      setClearing(false)
    }
  }

  return (
    <>
      <div className="decision-settings-summary">
        <span className={`decision-settings-dot${enabled ? ' is-ready' : ''}`} />
        <div>
          <strong>{enabled ? '已启用' : '未启用'}</strong>
          <p>
            本功能未在开发环境中完成端到端实测（缺少可运行 Laya 的机器），协议对接依据 Laya
            源码实现，尚未经过真实模型验证。投入使用前请按实际部署自行验证并按需修改。
          </p>
          <p>
            另请注意：Jev 与 Laya 各自独立配置，同一时刻只有一个生效，Jev 已配置 API Key
            时优先生效。
          </p>
        </div>
      </div>

      <FormRow
        label="启用 Laya 决策后端"
        hint="关闭时不会联系任何 Laya 服务，增强模式仍按既有的 Jev 或主模型逻辑运行。"
        control={
          <label className="decision-fallback-toggle">
            <input
              type="checkbox"
              checked={enabled}
              disabled={loading}
              onChange={event => setEnabled(event.target.checked)}
              aria-label="启用 Laya 决策后端"
            />
            <span>{enabled ? '已启用' : '已关闭'}</span>
          </label>
        }
      />

      <FormRow
        stacked
        label="服务地址"
        hint="自托管 Laya 服务地址，默认本机 8000 端口；非本机地址必须使用 HTTPS。"
        control={
          <input
            className="compact-input"
            value={baseUrl}
            disabled={loading}
            onChange={event => setBaseUrl(event.target.value)}
            placeholder={DEFAULT_BASE_URL}
            aria-label="Laya 服务地址"
          />
        }
      />

      <FormRow
        stacked
        label="模型"
        hint="可填 english、multilingual、typed-decisions；留空则由 Laya 路由按语言自动选择。"
        control={
          <input
            className="compact-input"
            value={model}
            disabled={loading}
            onChange={event => setModel(event.target.value)}
            placeholder="留空即自动路由"
            aria-label="Laya 模型"
          />
        }
      />

      <FormRow
        stacked
        label={
          <span className="models-field-label">
            <IconPlug size={12} className="icon-prefix" /> API Key
            {hasKey && <span className="model-badge label-badge">已配置</span>}
          </span>
        }
        hint="自托管 Laya 通常无需 Key，仅当服务端设置了 LAYA_API_KEY 时才需要填写。"
        control={
          <div className="models-key-row">
            <div className="models-key-field">
              <input
                className="compact-input"
                type={showKey ? 'text' : 'password'}
                value={apiKey}
                disabled={loading}
                onChange={event => setApiKey(event.target.value)}
                placeholder={hasKey ? '已配置；输入新密钥可覆盖' : '通常留空'}
                aria-label="Laya API Key"
              />
              <button
                type="button"
                className="models-key-eye"
                onClick={() => setShowKey(value => !value)}
                tabIndex={-1}
                title={showKey ? '隐藏' : '显示'}
                aria-label={showKey ? '隐藏 Laya API Key' : '显示 Laya API Key'}
              >
                {showKey ? <IconEyeOff size={14} /> : <IconEye size={14} />}
              </button>
            </div>
            {hasKey && (
              <button
                type="button"
                className="models-key-clear"
                onClick={() => setShowClearConfirm(true)}
                disabled={clearing}
                title="清除已保存的 Laya API Key"
                aria-label="清除已保存的 Laya API Key"
              >
                <IconBrushCleaning size={13} />
              </button>
            )}
          </div>
        }
      />

      <div className="decision-policy-grid">
        <FormRow
          stacked
          label="请求超时（毫秒）"
          hint="单次请求的最长等待时间，范围 100–120000。CPU 推理需要适当放宽。"
          control={
            <input
              className="compact-input"
              type="number"
              min={100}
              max={120000}
              step={1000}
              value={timeoutMs}
              disabled={loading}
              onChange={event => setTimeoutMs(event.target.value)}
              aria-label="Laya 请求超时"
            />
          }
        />

        <FormRow
          stacked
          label="最大重试次数"
          hint="只对可重试的临时错误生效，范围 0–10。"
          control={
            <input
              className="compact-input"
              type="number"
              min={0}
              max={10}
              step={1}
              value={maxRetries}
              disabled={loading}
              onChange={event => setMaxRetries(event.target.value)}
              aria-label="Laya 最大重试次数"
            />
          }
        />
      </div>

      <FormRow
        label="Laya 不可用时回退主模型"
        hint="服务请求失败时，由主模型继续从同一有限候选动作空间选择。"
        control={
          <label className="decision-fallback-toggle">
            <input
              type="checkbox"
              checked={fallbackToPrimaryModel}
              disabled={loading}
              onChange={event => setFallbackToPrimaryModel(event.target.checked)}
              aria-label="回退到主模型"
            />
            <span>{fallbackToPrimaryModel ? '已启用' : '已关闭'}</span>
          </label>
        }
      />

      <div className="decision-settings-actions">
        <Button variant="primary" size="sm" loading={saving} onClick={() => void save()}>
          保存
        </Button>
        <Button variant="ghost" size="sm" loading={testing} onClick={() => void test()}>
          <IconPlug size={13} />
          测试连接
        </Button>
        {feedback && (
          <span
            role="status"
            className={
              feedback.ok
                ? 'decision-settings-feedback is-ok'
                : 'decision-settings-feedback is-error'
            }
          >
            {feedback.message}
          </span>
        )}
      </div>

      <CompactModal
        open={showClearConfirm}
        onClose={() => setShowClearConfirm(false)}
        title="清除 API Key"
        size="sm"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setShowClearConfirm(false)}>
              取消
            </Button>
            <Button variant="danger" size="sm" loading={clearing} onClick={() => void clear()}>
              确认清除
            </Button>
          </>
        }
      >
        <p>确定清除已保存的 Laya API Key？服务地址和模型配置会保留。</p>
      </CompactModal>
    </>
  )
}
