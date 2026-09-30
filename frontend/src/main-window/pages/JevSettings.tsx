import { useEffect, useState } from 'react'
import { IconBrushCleaning, IconExternalLink, IconEye, IconEyeOff, IconPlug } from '../../ui/Icons'
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
import { LayaBackendPanel } from './LayaBackendPanel'
import { useLanguage } from '../../locales'

const DEFAULT_BASE_URL = 'https://api.typesafe.ai'
const DEFAULT_MODEL = 'jev-latest'
const DEFAULT_TIMEOUT_MS = 10_000
const DEFAULT_MAX_RETRIES = 2
/** TypeSafe 控制台：API Key 的唯一获取入口，在 API Key 行给出直达链接。 */
const TYPESAFE_CONSOLE_URL = 'https://console.typesafe.ai/'

type TFunc = (key: string, ...args: string[]) => string

/**
 * 词条取值：字典命中 → 用字典文案；未命中 → 用中文兜底。
 * `t()` 在 miss 时返回原 key，故此处显式判等。
 */
const tr = (t: TFunc, key: string, fallback: string): string => {
  const v = t(key)
  return v === key ? fallback : v
}

/** 页内文案表：按当前语言生成，键名稳定。 */
function makeTXT(t: TFunc) {
  return {
    title: tr(t, 'jev.title', '增强判断模型'),
    tabJev: tr(t, 'jev.tabJev', 'Jev（云端）'),
    tabLaya: tr(t, 'jev.tabLaya', 'Laya（自托管）'),
    configured: tr(t, 'jev.configured', '已配置'),
    notConfigured: tr(t, 'jev.notConfigured', '未配置'),
    privacyNote: tr(
      t,
      'jev.privacyNote',
      '开启增强模式后，只会发送经过裁剪和脱敏的候选动作元数据；完整截图和 API Key 不会发送给增强判断模型。',
    ),
    baseUrlLabel: tr(t, 'jev.baseUrlLabel', '接口地址'),
    baseUrlHint: tr(
      t,
      'jev.baseUrlHint',
      'TypeSafe System One API 地址；可替换为兼容的企业网关地址。',
    ),
    baseUrlAria: tr(t, 'jev.baseUrlAria', '增强判断模型接口地址'),
    modelLabel: tr(t, 'jev.modelLabel', '模型'),
    modelHint: tr(
      t,
      'jev.modelHint',
      '建议开发阶段使用 jev-latest；生产环境可固定经过评测的具体版本。',
    ),
    modelAria: tr(t, 'jev.modelAria', '增强判断模型'),
    apiKeyLabel: tr(t, 'jev.apiKeyLabel', 'API Key'),
    getApiKey: tr(t, 'jev.getApiKey', '获取 API Key'),
    apiKeyHint: tr(
      t,
      'jev.apiKeyHint',
      '密钥仅由本机后端安全保存；页面只读取是否已配置，不会回显原文。',
    ),
    apiKeyPlaceholderSet: tr(t, 'jev.apiKeyPlaceholderSet', '已配置；输入新密钥可覆盖'),
    apiKeyPlaceholderEmpty: tr(t, 'jev.apiKeyPlaceholderEmpty', '输入 API Key'),
    apiKeyAria: tr(t, 'jev.apiKeyAria', '增强判断模型 API Key'),
    showKey: tr(t, 'jev.showKey', '显示'),
    hideKey: tr(t, 'jev.hideKey', '隐藏'),
    showKeyAria: tr(t, 'jev.showKeyAria', '显示增强判断模型 API Key'),
    hideKeyAria: tr(t, 'jev.hideKeyAria', '隐藏增强判断模型 API Key'),
    clearKeyAria: tr(t, 'jev.clearKeyAria', '清除已保存的增强判断模型 API Key'),
    timeoutLabel: tr(t, 'jev.timeoutLabel', '请求超时（毫秒）'),
    timeoutHint: tr(t, 'jev.timeoutHint', '单次请求的最长等待时间，范围 100–120000。'),
    timeoutAria: tr(t, 'jev.timeoutAria', '增强判断模型请求超时'),
    retriesLabel: tr(t, 'jev.retriesLabel', '最大重试次数'),
    retriesHint: tr(t, 'jev.retriesHint', '只对可重试的临时错误生效，范围 0–10。'),
    retriesAria: tr(t, 'jev.retriesAria', '增强判断模型最大重试次数'),
    fallbackLabel: tr(t, 'jev.fallbackLabel', '增强判断模型不可用时回退主模型'),
    fallbackHint: tr(
      t,
      'jev.fallbackHint',
      '服务请求失败时，由主模型继续从同一有限候选动作空间选择。',
    ),
    fallbackAria: tr(t, 'jev.fallbackAria', '回退到主模型'),
    enabled: tr(t, 'jev.enabled', '已启用'),
    disabled: tr(t, 'jev.disabled', '已关闭'),
    saveBtn: tr(t, 'jev.saveBtn', '保存配置'),
    testBtn: tr(t, 'jev.testBtn', '测试连接'),
    boundaryTitle: tr(t, 'jev.boundaryTitle', '增强模式边界'),
    boundaryDesc: tr(
      t,
      'jev.boundaryDesc',
      '无论增强判断模型是否可用，下列执行约束都由本地代码保证。',
    ),
    boundaryItem1: tr(
      t,
      'jev.boundaryItem1',
      '增强判断模型只能从本地生成的候选动作中选择，不能自由生成坐标、脚本或选择器。',
    ),
    boundaryItem2: tr(
      t,
      'jev.boundaryItem2',
      '风险、权限、新鲜度检查、实际执行与结果验证都留在本机。',
    ),
    clearModalTitle: tr(t, 'jev.clearModalTitle', '清除 API Key'),
    clearModalBody: tr(
      t,
      'jev.clearModalBody',
      '确定清除已保存的增强判断模型 API Key？接口地址和模型配置会保留。',
    ),
    cancel: tr(t, 'jev.cancel', '取消'),
    confirmClear: tr(t, 'jev.confirmClear', '确认清除'),
    // ── 反馈消息（函数型，带 {0} 占位符）──
    connNormal: (m: string) =>
      t('jev.connNormal', m) === 'jev.connNormal' ? `连接正常（${m}）` : t('jev.connNormal', m),
    connOk: tr(t, 'jev.connOk', '连接正常'),
    connStatus: (s: string) =>
      t('jev.connStatus', s) === 'jev.connStatus' ? `连接状态：${s}` : t('jev.connStatus', s),
    errReadConfig: (e: string) =>
      t('jev.errReadConfig', e) === 'jev.errReadConfig'
        ? `读取增强判断模型配置失败：${e}`
        : t('jev.errReadConfig', e),
    errNeedBaseUrl: tr(t, 'jev.errNeedBaseUrl', '请输入接口地址'),
    errNeedModel: tr(t, 'jev.errNeedModel', '请输入模型名称'),
    errNeedApiKey: tr(t, 'jev.errNeedApiKey', '请输入 API Key'),
    errTimeoutRange: tr(t, 'jev.errTimeoutRange', '请求超时必须是 100–120000 毫秒之间的整数'),
    errRetriesRange: tr(t, 'jev.errRetriesRange', '最大重试次数必须是 0–10 之间的整数'),
    errNoConfig: tr(t, 'jev.errNoConfig', '后端未返回增强判断模型配置'),
    saveOk: tr(t, 'jev.saveOk', '增强判断模型配置已保存'),
    saveFail: (e: string) =>
      t('jev.saveFail', e) === 'jev.saveFail' ? `保存失败：${e}` : t('jev.saveFail', e),
    errNoTestResult: tr(t, 'jev.errNoTestResult', '后端未返回连接测试结果'),
    testFail: (e: string) =>
      t('jev.testFail', e) === 'jev.testFail' ? `连接测试失败：${e}` : t('jev.testFail', e),
    clearOk: tr(t, 'jev.clearOk', '增强判断模型 API Key 已清除'),
    clearFail: (e: string) =>
      t('jev.clearFail', e) === 'jev.clearFail' ? `清除失败：${e}` : t('jev.clearFail', e),
  }
}

function connectionMessage(
  result: JevConnectionStatus | string,
  TXT: ReturnType<typeof makeTXT>,
): string {
  if (typeof result === 'string') return result
  if (result.message) return result.message
  const status = result.status.toLowerCase()
  if (['ok', 'ready', 'available', 'connected'].includes(status)) {
    return result.model ? TXT.connNormal(result.model) : TXT.connOk
  }
  return TXT.connStatus(result.status)
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
  const { t } = useLanguage()
  const TXT = makeTXT(t)
  const [backend, setBackend] = useState<'jev' | 'laya'>('jev')
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
          setFeedback({ ok: false, message: TXT.errReadConfig(String(error)) })
        }
      })
      .finally(() => alive && setLoading(false))
    return () => {
      alive = false
    }
  }, [])

  const validate = (): boolean => {
    if (!baseUrl.trim()) {
      setFeedback({ ok: false, message: TXT.errNeedBaseUrl })
      return false
    }
    if (!model.trim()) {
      setFeedback({ ok: false, message: TXT.errNeedModel })
      return false
    }
    if (!hasKey && !apiKey.trim()) {
      setFeedback({ ok: false, message: TXT.errNeedApiKey })
      return false
    }
    const parsedTimeout = timeoutMs.trim() ? Number(timeoutMs) : Number.NaN
    if (!Number.isInteger(parsedTimeout) || parsedTimeout < 100 || parsedTimeout > 120_000) {
      setFeedback({ ok: false, message: TXT.errTimeoutRange })
      return false
    }
    const parsedRetries = maxRetries.trim() ? Number(maxRetries) : Number.NaN
    if (!Number.isInteger(parsedRetries) || parsedRetries < 0 || parsedRetries > 10) {
      setFeedback({ ok: false, message: TXT.errRetriesRange })
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
      throw new Error(TXT.errNoConfig)
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
      if (config) setFeedback({ ok: true, message: TXT.saveOk })
    } catch (error) {
      setFeedback({ ok: false, message: TXT.saveFail(String(error)) })
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
      if (!result) throw new Error(TXT.errNoTestResult)
      setFeedback({ ok: connectionOk(result), message: connectionMessage(result, TXT) })
    } catch (error) {
      setFeedback({ ok: false, message: TXT.testFail(String(error)) })
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
      setFeedback({ ok: true, message: TXT.clearOk })
      await refreshEnhancedModeStatus()
    } catch (error) {
      setFeedback({ ok: false, message: TXT.clearFail(String(error)) })
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
        <div className="decision-backend-tabs" role="tablist" aria-label="决策后端">
          <button
            type="button"
            role="tab"
            aria-selected={backend === 'jev'}
            className={['decision-backend-tab', backend === 'jev' ? 'is-active' : '']
              .filter(Boolean)
              .join(' ')}
            onClick={() => setBackend('jev')}
          >
            {TXT.tabJev}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={backend === 'laya'}
            className={['decision-backend-tab', backend === 'laya' ? 'is-active' : '']
              .filter(Boolean)
              .join(' ')}
            onClick={() => setBackend('laya')}
          >
            {TXT.tabLaya}
          </button>
        </div>

        {backend === 'laya' ? (
          <LayaBackendPanel />
        ) : (
          <>
            <div className="decision-settings-summary">
              <span className={`decision-settings-dot${hasKey ? ' is-ready' : ''}`} />
              <div>
                <strong>{hasKey ? TXT.configured : TXT.notConfigured}</strong>
                <p>{TXT.privacyNote}</p>
              </div>
            </div>

            <FormRow
              stacked
              label={TXT.baseUrlLabel}
              hint={TXT.baseUrlHint}
              control={
                <input
                  className="compact-input"
                  value={baseUrl}
                  disabled={loading}
                  onChange={event => setBaseUrl(event.target.value)}
                  placeholder={DEFAULT_BASE_URL}
                  aria-label={TXT.baseUrlAria}
                />
              }
            />

            <FormRow
              stacked
              label={TXT.modelLabel}
              hint={TXT.modelHint}
              control={
                <input
                  className="compact-input"
                  value={model}
                  disabled={loading}
                  onChange={event => setModel(event.target.value)}
                  placeholder={DEFAULT_MODEL}
                  aria-label={TXT.modelAria}
                />
              }
            />

            <FormRow
              stacked
              label={
                <span className="models-field-label decision-key-label">
                  <IconPlug size={12} className="icon-prefix" /> {TXT.apiKeyLabel}
                  {hasKey && <span className="model-badge label-badge">{TXT.configured}</span>}
                  {/* 外链由 App 层捕获阶段接管并交系统浏览器（WebView 不处理 _blank） */}
                  <a
                    className="decision-key-link"
                    href={TYPESAFE_CONSOLE_URL}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {TXT.getApiKey}
                    <IconExternalLink size={11} />
                  </a>
                </span>
              }
              hint={TXT.apiKeyHint}
              control={
                <div className="models-key-row">
                  <div className="models-key-field">
                    <input
                      className="compact-input"
                      type={showKey ? 'text' : 'password'}
                      value={apiKey}
                      disabled={loading}
                      onChange={event => setApiKey(event.target.value)}
                      placeholder={hasKey ? TXT.apiKeyPlaceholderSet : TXT.apiKeyPlaceholderEmpty}
                      aria-label={TXT.apiKeyAria}
                    />
                    <button
                      type="button"
                      className="models-key-eye"
                      onClick={() => setShowKey(value => !value)}
                      tabIndex={-1}
                      title={showKey ? TXT.hideKey : TXT.showKey}
                      aria-label={showKey ? TXT.hideKeyAria : TXT.showKeyAria}
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
                      title={TXT.clearKeyAria}
                      aria-label={TXT.clearKeyAria}
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
                label={TXT.timeoutLabel}
                hint={TXT.timeoutHint}
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
                    aria-label={TXT.timeoutAria}
                  />
                }
              />

              <FormRow
                stacked
                label={TXT.retriesLabel}
                hint={TXT.retriesHint}
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
                    aria-label={TXT.retriesAria}
                  />
                }
              />
            </div>

            <FormRow
              label={TXT.fallbackLabel}
              hint={TXT.fallbackHint}
              control={
                <label className="decision-fallback-toggle">
                  <input
                    type="checkbox"
                    checked={fallbackToPrimaryModel}
                    disabled={loading}
                    onChange={event => setFallbackToPrimaryModel(event.target.checked)}
                    aria-label={TXT.fallbackAria}
                  />
                  <span>{fallbackToPrimaryModel ? TXT.enabled : TXT.disabled}</span>
                </label>
              }
            />

            <div className="decision-settings-actions">
              <Button variant="primary" size="sm" loading={saving} onClick={() => void save()}>
                {TXT.saveBtn}
              </Button>
              <Button size="sm" loading={testing} onClick={() => void test()}>
                {TXT.testBtn}
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
          </>
        )}
      </Section>

      <Section title={TXT.boundaryTitle} description={TXT.boundaryDesc}>
        <ul className="decision-settings-boundaries">
          <li>{TXT.boundaryItem1}</li>
          <li>{TXT.boundaryItem2}</li>
        </ul>
      </Section>

      <CompactModal
        open={showClearConfirm}
        onClose={() => setShowClearConfirm(false)}
        title={TXT.clearModalTitle}
        size="sm"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setShowClearConfirm(false)}>
              {TXT.cancel}
            </Button>
            <Button variant="danger" size="sm" loading={clearing} onClick={() => void clear()}>
              {TXT.confirmClear}
            </Button>
          </>
        }
      >
        <p>{TXT.clearModalBody}</p>
      </CompactModal>
    </>
  )
}
