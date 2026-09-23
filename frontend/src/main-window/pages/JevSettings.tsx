import { useEffect, useState } from 'react'
import { IconBrushCleaning, IconEye, IconEyeOff, IconPlug } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { FormRow, Section } from '../../ui/PageLayout'
import { CompactModal } from '../layout/CompactModal'
import {
  clearJevApiKey,
  getJevConfig,
  getWorkflowEnhancedMode,
  saveJevConfig,
  testJevConnection,
  type JevConfig,
  type JevConnectionStatus,
} from '../lib/api'
import { publishWorkflowEnhancedMode } from '../workflow-canvas/enhancedModeEvents'

const DEFAULT_BASE_URL = 'https://api.typesafe.ai'
const DEFAULT_MODEL = 'jev-latest'
const DEFAULT_TIMEOUT_MS = 10_000
const DEFAULT_MAX_RETRIES = 2

function connectionMessage(result: JevConnectionStatus | string): string {
  if (typeof result === 'string') return result
  if (result.message) return result.message
  const status = result.status.toLowerCase()
  if (['ok', 'ready', 'available', 'connected'].includes(status)) {
    return result.model ? `连接正常（${result.model}）` : '连接正常'
  }
  return `连接状态：${result.status}`
}

function connectionOk(result: JevConnectionStatus | string): boolean {
  if (typeof result === 'string') {
    return ['ok', 'ready', 'available', 'connected', 'success'].includes(result.toLowerCase())
  }
  return ['ok', 'ready', 'available', 'connected', 'success'].includes(result.status.toLowerCase())
}

async function refreshEnhancedModeStatus(): Promise<void> {
  try {
    const enhancedMode = await getWorkflowEnhancedMode()
    if (enhancedMode) publishWorkflowEnhancedMode(enhancedMode)
  } catch {
    // 配置保存本身已经成功时，不因状态徽标刷新失败而误报保存失败。
  }
}

export function JevSettings() {
  const [baseUrl, setBaseUrl] = useState(DEFAULT_BASE_URL)
  const [model, setModel] = useState(DEFAULT_MODEL)
  const [apiKey, setApiKey] = useState('')
  const [hasKey, setHasKey] = useState(false)
  const [timeoutMs, setTimeoutMs] = useState(String(DEFAULT_TIMEOUT_MS))
  const [maxRetries, setMaxRetries] = useState(String(DEFAULT_MAX_RETRIES))
  const [fallbackToPrimaryModel, setFallbackToPrimaryModel] = useState(true)
  const [showKey, setShowKey] = useState(false)
  const [showClearConfirm, setShowClearConfirm] = useState(false)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [feedback, setFeedback] = useState<{ ok: boolean; message: string } | null>(null)

  useEffect(() => {
    let alive = true
    void getJevConfig()
      .then(config => {
        if (!alive || !config) return
        setBaseUrl(config.base_url || DEFAULT_BASE_URL)
        setModel(config.model || DEFAULT_MODEL)
        setHasKey(config.has_key)
        setTimeoutMs(String(config.timeout_ms ?? DEFAULT_TIMEOUT_MS))
        setMaxRetries(String(config.max_retries ?? DEFAULT_MAX_RETRIES))
        setFallbackToPrimaryModel(config.fallback_to_primary_model ?? true)
        // 安全边界：后端只返回 has_key；已保存密钥绝不进入输入框状态。
        setApiKey('')
      })
      .catch(error => {
        if (alive) {
          setFeedback({ ok: false, message: `读取增强判断模型配置失败：${String(error)}` })
        }
      })
      .finally(() => alive && setLoading(false))
    return () => {
      alive = false
    }
  }, [])

  const validate = (): boolean => {
    if (!baseUrl.trim()) {
      setFeedback({ ok: false, message: '请输入接口地址' })
      return false
    }
    if (!model.trim()) {
      setFeedback({ ok: false, message: '请输入模型名称' })
      return false
    }
    if (!hasKey && !apiKey.trim()) {
      setFeedback({ ok: false, message: '请输入 API Key' })
      return false
    }
    const parsedTimeout = timeoutMs.trim() ? Number(timeoutMs) : Number.NaN
    if (!Number.isInteger(parsedTimeout) || parsedTimeout < 100 || parsedTimeout > 120_000) {
      setFeedback({ ok: false, message: '请求超时必须是 100–120000 毫秒之间的整数' })
      return false
    }
    const parsedRetries = maxRetries.trim() ? Number(maxRetries) : Number.NaN
    if (!Number.isInteger(parsedRetries) || parsedRetries < 0 || parsedRetries > 10) {
      setFeedback({ ok: false, message: '最大重试次数必须是 0–10 之间的整数' })
      return false
    }
    return true
  }

  const saveCurrent = async (): Promise<JevConfig | null> => {
    if (!validate()) return null
    const config = await saveJevConfig({
      apiKey: apiKey.trim() || undefined,
      baseUrl: baseUrl.trim(),
      model: model.trim(),
      timeoutMs: Number(timeoutMs),
      maxRetries: Number(maxRetries),
      fallbackToPrimaryModel,
    })
    if (!config) {
      throw new Error('后端未返回增强判断模型配置')
    }
    setBaseUrl(config.base_url || baseUrl.trim())
    setModel(config.model || model.trim())
    setHasKey(config.has_key)
    setTimeoutMs(String(config.timeout_ms ?? Number(timeoutMs)))
    setMaxRetries(String(config.max_retries ?? Number(maxRetries)))
    setFallbackToPrimaryModel(config.fallback_to_primary_model ?? fallbackToPrimaryModel)
    setApiKey('')
    setShowKey(false)
    await refreshEnhancedModeStatus()
    return config
  }

  const save = async () => {
    setSaving(true)
    setFeedback(null)
    try {
      const config = await saveCurrent()
      if (config) setFeedback({ ok: true, message: '增强判断模型配置已保存' })
    } catch (error) {
      setFeedback({ ok: false, message: `保存失败：${String(error)}` })
    } finally {
      setSaving(false)
    }
  }

  const test = async () => {
    setTesting(true)
    setFeedback(null)
    try {
      // 测试命令不接收密钥参数，先安全落盘当前表单，再测试同一份配置。
      const config = await saveCurrent()
      if (!config) return
      const result = await testJevConnection()
      if (!result) throw new Error('后端未返回连接测试结果')
      setFeedback({ ok: connectionOk(result), message: connectionMessage(result) })
    } catch (error) {
      setFeedback({ ok: false, message: `连接测试失败：${String(error)}` })
    } finally {
      setTesting(false)
    }
  }

  const clear = async () => {
    setShowClearConfirm(false)
    setClearing(true)
    setFeedback(null)
    try {
      await clearJevApiKey()
      setHasKey(false)
      setApiKey('')
      setShowKey(false)
      setFeedback({ ok: true, message: '增强判断模型 API Key 已清除' })
      await refreshEnhancedModeStatus()
    } catch (error) {
      setFeedback({ ok: false, message: `清除失败：${String(error)}` })
    } finally {
      setClearing(false)
    }
  }

  return (
    <>
      <Section
        title="增强判断模型"
        description="用于在工作流开发中从有限候选动作里进行结构化判断，不替代聊天模型，也不能直接点击坐标或生成任意脚本。"
      >
        <div className="jev-settings-summary">
          <span className={`jev-settings-dot${hasKey ? ' is-ready' : ''}`} />
          <div>
            <strong>{hasKey ? '已配置' : '未配置'}</strong>
            <p>
              开启增强模式后，只会发送经过裁剪和脱敏的候选动作元数据；完整截图和 API Key
              不会发送给增强判断模型。
            </p>
          </div>
        </div>

        <FormRow
          stacked
          label="接口地址"
          hint="TypeSafe System One API 地址；可替换为兼容的企业网关地址。"
          control={
            <input
              className="compact-input"
              value={baseUrl}
              disabled={loading}
              onChange={event => setBaseUrl(event.target.value)}
              placeholder={DEFAULT_BASE_URL}
              aria-label="增强判断模型接口地址"
            />
          }
        />

        <FormRow
          stacked
          label="模型"
          hint="建议开发阶段使用 jev-latest；生产环境可固定经过评测的具体版本。"
          control={
            <input
              className="compact-input"
              value={model}
              disabled={loading}
              onChange={event => setModel(event.target.value)}
              placeholder={DEFAULT_MODEL}
              aria-label="增强判断模型"
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
          hint="密钥仅由本机后端安全保存；页面只读取是否已配置，不会回显原文。"
          control={
            <div className="models-key-row">
              <div className="models-key-field">
                <input
                  className="compact-input"
                  type={showKey ? 'text' : 'password'}
                  value={apiKey}
                  disabled={loading}
                  onChange={event => setApiKey(event.target.value)}
                  placeholder={hasKey ? '已配置；输入新密钥可覆盖' : '输入 API Key'}
                  aria-label="增强判断模型 API Key"
                />
                <button
                  type="button"
                  className="models-key-eye"
                  onClick={() => setShowKey(value => !value)}
                  tabIndex={-1}
                  title={showKey ? '隐藏' : '显示'}
                  aria-label={showKey ? '隐藏增强判断模型 API Key' : '显示增强判断模型 API Key'}
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
                  title="清除已保存的增强判断模型 API Key"
                  aria-label="清除已保存的增强判断模型 API Key"
                >
                  <IconBrushCleaning size={13} />
                </button>
              )}
            </div>
          }
        />

        <div className="jev-policy-grid">
          <FormRow
            stacked
            label="请求超时（毫秒）"
            hint="单次请求的最长等待时间，范围 100–120000。"
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
                aria-label="增强判断模型请求超时"
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
                aria-label="增强判断模型最大重试次数"
              />
            }
          />
        </div>

        <FormRow
          label="增强判断模型不可用时回退主模型"
          hint="服务请求失败时，由主模型继续从同一有限候选动作空间选择。"
          control={
            <label className="jev-fallback-toggle">
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

        <div className="jev-settings-actions">
          <Button variant="primary" size="sm" loading={saving} onClick={() => void save()}>
            保存配置
          </Button>
          <Button size="sm" loading={testing} onClick={() => void test()}>
            测试连接
          </Button>
          {feedback && (
            <span
              role="status"
              className={
                feedback.ok ? 'jev-settings-feedback is-ok' : 'jev-settings-feedback is-error'
              }
            >
              {feedback.message}
            </span>
          )}
        </div>
      </Section>

      <Section
        title="增强模式边界"
        description="无论增强判断模型是否可用，下列执行约束都由本地代码保证。"
      >
        <ul className="jev-settings-boundaries">
          <li>增强判断模型只能从本地生成的候选动作中选择，不能自由生成坐标、脚本或选择器。</li>
          <li>风险、权限、新鲜度检查、实际执行与结果验证都留在本机。</li>
        </ul>
      </Section>

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
        <p>确定清除已保存的增强判断模型 API Key？接口地址和模型配置会保留。</p>
      </CompactModal>
    </>
  )
}
