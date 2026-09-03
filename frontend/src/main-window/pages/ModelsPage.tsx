import { useState, useEffect, useRef, useCallback } from 'react'
import { createPortal } from 'react-dom'
import {
  getCurrentConfig,
  configureLlm,
  clearProviderApiKey,
  switchModel as switchModelCmd,
  getSupportedProviders,
  getCapabilities,
  setCapability,
  listModels,
  listProviderModels,
  refreshProviderModels,
  getAgentModels,
  setAgentModel,
  sttStatus,
} from '../lib/api'
import type {
  ProviderInfo,
  ModelInfo,
  ProviderModelBrief,
  SttStatus,
  AgentModels,
} from '../lib/api'
import {
  useSttModelDownload,
  sttDownloadProgressPct,
  sttDownloadProgressText,
} from '../lib/useSttModelDownload'
import {
  useVisionModelDownload,
  modelsDownloadProgressPct,
  modelsDownloadProgressText,
} from '../lib/useVisionModelDownload'
import {
  IconCheck,
  IconTrash2,
  IconEye,
  IconMic,
  IconImage,
  IconAlertTriangle,
  IconRefresh,
  IconBrushCleaning,
} from '../../ui/Icons'
import { Section, FormRow } from '../../ui/PageLayout'
import { Button } from '../../ui/Button'
import { useLanguage } from '../../locales'
import '../../styles/models.css'

function ProviderSelect({
  value,
  options,
  onChange,
}: {
  value: string
  options: ProviderInfo[]
  onChange: (id: string) => void
}) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  const close = useCallback(() => setOpen(false), [])

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open, close])

  const selected = options.find(o => o.id === value)

  return (
    <div className="compact-select-wrap" ref={ref}>
      <div
        className="compact-select-trigger"
        tabIndex={0}
        onClick={() => setOpen(v => !v)}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            setOpen(v => !v)
          }
        }}
      >
        <span>{selected?.name || value}</span>
        <span className={`compact-select-arrow ${open ? 'open' : ''}`}>▾</span>
      </div>
      {open && (
        <div className="compact-select-menu">
          {options.map(o => (
            <div
              key={o.id}
              className={`compact-select-option ${value === o.id ? 'active' : ''}`}
              onClick={() => {
                onChange(o.id)
                close()
              }}
            >
              {o.name}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// Vision Model Select — 模块级组件（参考 ProviderSelect）
// ⚠️ 禁止移回 ModelsPage 内部：组件内定义的组件每次父渲染都是新类型，
//    React 会卸载重挂载，open 状态丢失 → 下拉弹起立即缩回、反复抖动
function VisionModelSelect({
  value,
  models,
  onChange,
  t,
  placeholder,
  showVisionIcons,
  filterCapability,
}: {
  value: string
  models: ModelInfo[]
  onChange: (id: string) => void
  t: any
  placeholder?: string
  showVisionIcons?: boolean
  filterCapability?: 'vision' | 'audio'
}) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const close = useCallback(() => setOpen(false), [])

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open, close])

  // Sort: vision-capable first (or audio-capable first for audio filter)
  const filtered = Array.isArray(models)
    ? (() => {
        let list = [...models]
        if (filterCapability === 'vision') {
          list = list.filter(m => m.supports_vision)
        } else if (filterCapability === 'audio') {
          list = list.filter(m => m.supports_audio)
        }
        // Sort: matching capability first
        if (filterCapability === 'audio') {
          list.sort((a, b) => (b.supports_audio ? 1 : 0) - (a.supports_audio ? 1 : 0))
        } else {
          list.sort((a, b) => (b.supports_vision ? 1 : 0) - (a.supports_vision ? 1 : 0))
        }
        return list
      })()
    : []
  const selected = filtered.find(m => m.id === value)
  const emptyText = placeholder || t('models.visionModelNone')

  return (
    <div className="compact-select-wrap" ref={ref}>
      <div
        className="compact-select-trigger"
        tabIndex={0}
        onClick={() => setOpen(v => !v)}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            setOpen(v => !v)
          }
        }}
      >
        <span>{selected ? `${selected.id} (${selected.provider})` : emptyText}</span>
        <span className={`compact-select-arrow ${open ? 'open' : ''}`}>▾</span>
      </div>
      {open && (
        <div className="compact-select-menu select-menu--up">
          {/* Clear option */}
          <div
            className={`compact-select-option ${value === '' ? 'active' : ''}`}
            onClick={() => {
              onChange('')
              close()
            }}
          >
            {emptyText}
          </div>
          {filtered.map(m => (
            <div
              key={m.id}
              className={`compact-select-option ${value === m.id ? 'active' : ''}`}
              onClick={() => {
                onChange(m.id)
                close()
              }}
            >
              {showVisionIcons !== false && m.supports_vision && (
                <IconEye size={11} className="icon-prefix" />
              )}
              {m.id}
              <span className="select-option-provider">({m.provider})</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

const MODELS_KEY_PREFIX = 'nuphus_models_'
const DETECTED_KEY_PREFIX = 'nuphus_detected_models_'

function loadModels(provider: string): string[] {
  try {
    return JSON.parse(localStorage.getItem(MODELS_KEY_PREFIX + provider) || '[]')
  } catch {
    return []
  }
}

function saveModels(provider: string, models: string[]) {
  localStorage.setItem(MODELS_KEY_PREFIX + provider, JSON.stringify(models))
}

/** 兼容旧版 string[] 缓存（仅模型 id）：解析后统一升级为 ProviderModelBrief[] */
function loadDetectedModels(provider: string): ProviderModelBrief[] {
  try {
    const raw = JSON.parse(localStorage.getItem(DETECTED_KEY_PREFIX + provider) || '[]')
    if (!Array.isArray(raw)) return []
    return raw.map(item => (typeof item === 'string' ? { id: item } : item)) as ProviderModelBrief[]
  } catch {
    return []
  }
}

function saveDetectedModels(provider: string, models: ProviderModelBrief[]) {
  localStorage.setItem(DETECTED_KEY_PREFIX + provider, JSON.stringify(models))
}

/** 上下文窗口格式化：1_000_000 → 1M，128_000 → 128K，32_768 → 33K；未知返回空串 */
function formatContextWindow(n?: number): string {
  if (!n || n <= 0) return ''
  if (n >= 1_000_000) {
    const m = n / 1_000_000
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`
  }
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`
  return `${n}`
}

export function ModelsPage({
  onClose,
  onModelChanged,
}: {
  onClose: () => void
  onModelChanged?: () => void
}) {
  const { t } = useLanguage()
  const [currentModel, setCurrentModel] = useState('')
  const [providers, setProviders] = useState<ProviderInfo[]>([])
  const [providersLoading, setProvidersLoading] = useState(true)
  const [provider, setProvider] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [showKey, setShowKey] = useState(false)
  const [inputVal, setInputVal] = useState('')
  const [models, setModels] = useState<string[]>([])
  const [baseUrl, setBaseUrl] = useState('')
  const [feedback, setFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [visionModel, setVisionModel] = useState('')
  const [visionSaving, setVisionSaving] = useState(false)
  const [visionFeedback, setVisionFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  // 本地 sherpa-onnx STT 状态（进入 custom tab 时一次性探测；探测失败保持 null，静默降级）
  const [sttLocalStatus, setSttLocalStatus] = useState<SttStatus | null>(null)
  // 云端 STT 模型（capabilities.stt；配置后语音输入优先走云端 /audio/transcriptions）
  const [sttModel, setSttModel] = useState('')
  const [sttSaving, setSttSaving] = useState(false)
  const [sttFeedback, setSttFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [ttsModel, setTtsModel] = useState('')
  const [ttsSaving, setTtsSaving] = useState(false)
  const [ttsFeedback, setTtsFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  // 语音克隆模型（capabilities.voice；走云端克隆 API，配置后工具页「语音克隆」可用）
  const [voiceModel, setVoiceModel] = useState('')
  const [voiceSaving, setVoiceSaving] = useState(false)
  const [voiceFeedback, setVoiceFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [allModels, setAllModels] = useState<ModelInfo[]>([])
  // Agent 级模型配置（高级设置）：exec 模型
  const [agentModels, setAgentModels] = useState<AgentModels>({
    leader: '',
    workflow: '',
    exec: '',
    custom: '',
  })
  const [agentSaving, setAgentSaving] = useState(false)
  const [agentFeedback, setAgentFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [activeTab, setActiveTab] = useState<'basic' | 'custom'>('basic')
  // 当前 provider 是否已存储 API key（从不暴露 key 本身）
  const [hasKey, setHasKey] = useState(false)
  // 所有已配置（有 API key）的 provider 列表
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([])
  // API key 检测 — 通过 /v1/models 列出可用模型
  const [detecting, setDetecting] = useState(false)
  // 清除 API Key 进行中（防重复点击）
  const [clearingKey, setClearingKey] = useState(false)
  const [detectedModels, setDetectedModels] = useState<ProviderModelBrief[]>([])
  const [filterInput, setFilterInput] = useState('') // 实时筛选 detectedModels
  const [detectError, setDetectError] = useState<string | null>(null)
  // 模型列表刷新（用已存 key 拉取服务商最新模型）
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)

  // 本地 STT 探测（一次性，不轮询；调用失败静默降级，与 VoiceButton 策略一致）
  const probeStt = useCallback(() => {
    sttStatus()
      .then(s => setSttLocalStatus(s))
      .catch(() => {})
  }, [])

  // STT 模型下载（事件驱动；done 后重新探测，卡片自动转为就绪态）
  const sttDl = useSttModelDownload(probeStt)
  // 本地视觉模型（OCR / YOLO）自动下载状态（事件驱动；启动时 bootstrap 已自动发起）
  const visionDl = useVisionModelDownload()

  // 进入 custom tab 时探测本地 STT 状态 + 刷新视觉模型状态
  useEffect(() => {
    if (activeTab !== 'custom') return
    probeStt()
    visionDl.refresh()
  }, [activeTab, probeStt, visionDl.refresh])

  // Load specified provider state (baseUrl + model list only, API key is kept on backend)
  const loadProviderState = (id: string) => {
    setBaseUrl('')
    setModels(loadModels(id))
  }

  useEffect(() => {
    Promise.all([
      getSupportedProviders()
        .then(list => {
          if (Array.isArray(list)) setProviders(list)
        })
        .catch(() => {}),
      getCurrentConfig().then(cfg => {
        if (cfg) {
          setCurrentModel(cfg.model || '')
          const prov = cfg.provider || 'deepseek'
          try {
            if (cfg.model) {
              localStorage.setItem(`nuphus_current_model_${prov}`, cfg.model)
            }
          } catch {
            /* localStorage 写入失败不阻塞 UI */
          }
          setProvider(prov)
          setApiKey('')
          setHasKey(!!cfg.has_key)
          setBaseUrl(cfg.base_url || '')
          if (cfg.configured_providers) setConfiguredProviders(cfg.configured_providers)
        }
      }),
    ]).finally(() => setProvidersLoading(false))
    getCapabilities()
      .then(m => {
        if (m) {
          setVisionModel(m.vision)
          setTtsModel(m.tts)
          setSttModel(m.stt)
          setVoiceModel(m.voice)
        }
      })
      .catch(() => {})
    listModels()
      .then(list => {
        if (Array.isArray(list)) setAllModels(list)
      })
      .catch(() => {})
    getAgentModels()
      .then(v => {
        if (v) setAgentModels(v)
      })
      .catch(() => {})
  }, [])

  // Load data when provider changes
  useEffect(() => {
    setModels(loadModels(provider))
    setDetectedModels(loadDetectedModels(provider))
    setBaseUrl('') // 清空避免跨 provider 泄漏
    setFilterInput('') // 切换 provider 清空筛选
    // 加载该 provider 持久化的当前 model,供弹窗读取
    try {
      const saved = localStorage.getItem(`nuphus_current_model_${provider}`)
      if (saved) setCurrentModel(saved)
    } catch {
      setCurrentModel('')
    }
  }, [provider])

  const [localCtxWindow, setLocalCtxWindow] = useState(() => {
    try {
      return parseInt(localStorage.getItem('nuphus_local_context_window') || '128000')
    } catch {
      return 128000
    }
  })

  const curProvider = providers.find(p => p.id === provider)
  const isCustom = provider === 'custom'
  const isLocal = provider === 'local'

  // 持久化当前 provider + current model 到 localStorage，供快捷切换弹窗读取
  const persistCurrentProvider = (name?: string) => {
    try {
      if (provider && (name ?? currentModel)) {
        localStorage.setItem(`nuphus_current_model_${provider}`, name ?? currentModel)
      }
    } catch {
      /* localStorage 写入失败不阻塞 UI */
    }
  }
  const handleProviderChange = (id: string) => {
    setProvider(id)
    loadProviderState(id)
    setInputVal('')
    setApiKey('') // 清空 key，防止前一个 provider 的 key 泄漏到新 provider
    setFeedback(null)
    setDetectError(null)
    setRefreshError(null)
    setDetecting(false)
    // 判断新 provider 是否已有存储的 key
    setHasKey(configuredProviders.includes(id))
  }

  /** 通过 /v1/models 检测 API key 并列出可用模型 */
  const detectModels = async () => {
    const key = apiKey.trim()
    if (!key) {
      setDetectError('请先输入 API 密钥')
      return
    }
    setDetecting(true)
    setDetectError(null)
    setDetectedModels([])
    try {
      const p = providers.find(x => x.id === provider)
      const resolvedBaseUrl = baseUrl || p?.base_url || ''
      const models = await listProviderModels(key, provider, resolvedBaseUrl || undefined)
      setDetectedModels(models ?? [])
      saveDetectedModels(provider, models ?? [])
    } catch (e: any) {
      setDetectError(e?.message || '检测失败')
    } finally {
      setDetecting(false)
    }
  }

  /** 刷新当前服务商最新模型列表：复用 config.toml 已存 key（不暴露 key 本身） */
  const refreshModels = async () => {
    if (refreshing) return
    setRefreshing(true)
    setRefreshError(null)
    try {
      const p = providers.find(x => x.id === provider)
      const resolvedBaseUrl = baseUrl || p?.base_url || ''
      const models = await refreshProviderModels(provider, resolvedBaseUrl || undefined)
      setDetectedModels(models ?? [])
      saveDetectedModels(provider, models ?? [])
      // 刷新后同步 allModels：图像理解 / STT / TTS 选择器数据源来自 list_models，
      // 后端已把新模型 upsert 进 config.toml，这里重新拉取让选择器立刻看到新模型
      listModels()
        .then(list => {
          if (Array.isArray(list)) setAllModels(list)
        })
        .catch(() => {})
    } catch (e: any) {
      setRefreshError(e?.message || '刷新失败')
    } finally {
      setRefreshing(false)
    }
  }

  /** 清除当前 provider 已存储的 API Key（仅清 key，保留 provider/model 配置） */
  const handleClearKey = async () => {
    if (!window.confirm(t('models.clearKeyConfirm'))) return
    setClearingKey(true)
    try {
      await clearProviderApiKey(provider)
      // 重新拉取后端配置刷新 UI：apiKey 从不暴露；hasKey 按 configured_providers 重算
      // （getCurrentConfig 的 has_key 属于当前激活 provider，不一定是正在查看的 provider）
      const cfg = await getCurrentConfig()
      if (cfg) {
        setApiKey('')
        setBaseUrl(cfg.base_url || '')
        if (cfg.configured_providers) {
          setConfiguredProviders(cfg.configured_providers)
          setHasKey(cfg.configured_providers.includes(provider))
        }
      }
      setFeedback({ ok: true, msg: t('models.clearKeySuccess') })
      setTimeout(() => setFeedback(null), 2000)
    } catch (e: any) {
      setFeedback({ ok: false, msg: e?.message || t('models.clearKeyFail') })
      setTimeout(() => setFeedback(null), 3000)
    } finally {
      setClearingKey(false)
    }
  }

  const addModel = async () => {
    const name = inputVal.trim()
    if (!name) return
    const list = loadModels(provider)
    if (list.includes(name)) {
      setFeedback({ ok: false, msg: t('models.existFail', name) })
      setTimeout(() => setFeedback(null), 1500)
      return
    }
    list.push(name)
    saveModels(provider, list)
    setModels(list)
    setInputVal('')

    // 规则（provider-driven）：
    // - 用户填写了 key → configureLlm 保存密钥，再 switchModel 激活
    // - 用户未填写 key 但已有存储 → switchModel 直接从 config.toml 读取
    // - 两者都没有 → 提示用户输入 key
    const effectiveKey = apiKey.trim() ? apiKey : ''
    if (!isLocal && !effectiveKey.trim() && !hasKey) {
      setFeedback({ ok: false, msg: '请先输入API密钥' })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    const p = providers.find(x => x.id === provider)
    if (p) {
      const resolvedBaseUrl = baseUrl || p.base_url
      try {
        if (effectiveKey) {
          // 用户输入了新 key → 先保存，再激活
          await configureLlm(
            effectiveKey,
            name,
            provider,
            resolvedBaseUrl,
            isLocal ? localCtxWindow : undefined,
          )
        } else {
          // 已有存储的 key → switchModel 直接读取 config.toml
          await switchModelCmd(
            name,
            provider,
            resolvedBaseUrl,
            isLocal ? localCtxWindow : undefined,
            'global',
          )
        }
        setCurrentModel(name)
        persistCurrentProvider(name)
        onModelChanged?.()
        // Refresh model list for custom tab (vision/STT/TTS)
        listModels()
          .then(list => {
            if (Array.isArray(list)) setAllModels(list)
          })
          .catch(() => {})
        setFeedback({ ok: true, msg: t('models.switchTo', name) })
      } catch (e: any) {
        setFeedback({ ok: false, msg: e?.message || t('models.switchFail') })
      }
      setTimeout(() => setFeedback(null), 2500)
    }
  }

  // Agent 级模型保存（高级设置）：空串 = 清除（跟随 global fallback）
  const saveAgentModel = async (agent: string, model: string) => {
    setAgentSaving(true)
    setAgentFeedback(null)
    try {
      await setAgentModel(agent, model)
      setAgentModels(prev => ({ ...prev, [agent]: model }))
      setAgentFeedback({ ok: true, msg: `${agent} 模型已保存${model ? '' : '（跟随默认模型）'}` })
    } catch (e: any) {
      setAgentFeedback({ ok: false, msg: e?.message || '保存失败' })
    }
    setAgentSaving(false)
    setTimeout(() => setAgentFeedback(null), 2500)
  }

  const removeModel = (name: string) => {
    const list = loadModels(provider).filter(m => m !== name)
    saveModels(provider, list)
    setModels(list)
    if (currentModel === name) {
      setCurrentModel('')
    }
  }

  const switchModel = async (name: string) => {
    if (providersLoading || providers.length === 0) {
      setFeedback({ ok: false, msg: t('models.listLoading') })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    const p = providers.find(x => x.id === provider)
    if (!p) {
      setFeedback({ ok: false, msg: t('models.switchFail') })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    setFeedback(null)
    const resolvedBaseUrl = baseUrl || p.base_url
    try {
      // provider-driven: switch_model 从 config.toml 读取 API key，前端不传 key
      // mode='default'：模型页主切换写入默认模型（聊天界面按当前 mode 写对应 agent）
      await switchModelCmd(
        name,
        provider,
        resolvedBaseUrl,
        isLocal ? localCtxWindow : undefined,
        'default',
      )
      setCurrentModel(name)
      persistCurrentProvider(name)
      onModelChanged?.()
      setFeedback({ ok: true, msg: t('models.switchTo', name) })
    } catch (e: any) {
      setFeedback({ ok: false, msg: e?.message || t('models.switchFail') })
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  return (
    <div className="page">
      <div className="page-tabs">
        <button
          className={`page-tab ${activeTab === 'basic' ? 'active' : ''}`}
          onClick={() => setActiveTab('basic')}
        >
          {t('models.tabBasic') || '基础配置'}
        </button>
        <button
          className={`page-tab ${activeTab === 'custom' ? 'active' : ''}`}
          onClick={() => setActiveTab('custom')}
        >
          {t('models.tabCustom') || '自定义配置'}
        </button>
      </div>

      {activeTab === 'basic' && (
        <>
          <Section title={t('models.provider')}>
            <FormRow
              stacked
              label={t('models.providerSelect')}
              control={
                <ProviderSelect
                  value={provider}
                  options={providers}
                  onChange={handleProviderChange}
                />
              }
            />
            {!isLocal && (
              <FormRow
                stacked
                label={
                  <>
                    {t('models.apiKey')}
                    {hasKey && <span className="badge badge-accent label-badge">已配置</span>}
                  </>
                }
                control={
                  <div className="compact-input-row">
                    <input
                      className="compact-input"
                      type={showKey ? 'text' : 'password'}
                      value={apiKey}
                      onChange={e => {
                        setApiKey(e.target.value)
                        setDetectError(null)
                      }}
                      placeholder={
                        hasKey
                          ? '输入新密钥覆盖现有配置'
                          : t('models.inputApiKey', curProvider?.name || '')
                      }
                    />
                    <button className="input-suffix-btn" onClick={() => setShowKey(!showKey)}>
                      {showKey ? t('common.hide') : t('common.show')}
                    </button>
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={detectModels}
                      disabled={detecting || !apiKey.trim()}
                    >
                      {detecting ? '连接中...' : '连接'}
                    </Button>
                    {hasKey && (
                      <button
                        type="button"
                        className="icon-btn-ghost icon-btn-clear"
                        onClick={handleClearKey}
                        disabled={clearingKey}
                        title={t('models.clearKey')}
                        aria-label={t('models.clearKey')}
                      >
                        <IconBrushCleaning size={14} />
                      </button>
                    )}
                  </div>
                }
              />
            )}

            {/* ── 模型筛选(可选) -- 上提到 API Key 块下方, 大王需求 ③ ── */}
            <FormRow
              stacked
              control={
                <input
                  className="compact-input"
                  value={filterInput}
                  onChange={e => setFilterInput(e.target.value)}
                  placeholder={t('models.filterModels')}
                />
              }
              label=""
            />
            {(isCustom || isLocal) && (
              <FormRow
                stacked
                label={t('models.baseUrl')}
                control={
                  <input
                    className="compact-input"
                    value={baseUrl}
                    onChange={e => setBaseUrl(e.target.value)}
                    placeholder={
                      isLocal ? 'http://localhost:11434/v1' : t('models.baseUrlPlaceholder')
                    }
                  />
                }
              />
            )}
          </Section>

          <Section
            title={t('models.modelList')}
            description={currentModel ? t('models.currentModel', currentModel) : undefined}
            actions={
              <button
                className="models-refresh-btn"
                onClick={refreshModels}
                disabled={refreshing}
                title="获取该服务商最新模型列表"
              >
                <IconRefresh size={13} className={refreshing ? 'is-spinning' : ''} />
                {refreshing ? '刷新中...' : '刷新'}
              </button>
            }
          >
            {refreshError && (
              <div className="text-caption" style={{ color: 'var(--error)', marginBottom: 8 }}>
                ⚠ {refreshError}
              </div>
            )}
            {/* ── 可用模型列表（后端已配置 + 本地检测结果并集，实时筛选）+ radio 选择 (大王需求 ②+⑤) ── */}
            {(() => {
              const configured = allModels.filter(m => m.provider === provider).map(m => m.id)
              const display = Array.from(new Set([...detectedModels.map(d => d.id), ...configured]))
              const q = filterInput.trim().toLowerCase()
              const filtered = q ? display.filter(m => m.toLowerCase().includes(q)) : display
              if (filtered.length === 0) {
                return (
                  <div className="detect-status">
                    {q ? `筛选「${filterInput}」无匹配模型` : '未检测到可用模型，请先连接'}
                  </div>
                )
              }
              // 能力数据：检测结果（ProviderModelBrief）优先，list_models（ModelInfo）兜底
              const briefById = new Map<string, ProviderModelBrief>(
                detectedModels.map(d => [d.id, d]),
              )
              const infoById = new Map<string, ModelInfo>(allModels.map(m => [m.id, m]))
              return (
                <div>
                  <div className="detect-status">
                    {display.length} 个可用模型，点击即配置
                    {configured.length > 0 && `（含 ${configured.length} 个已配置）`}
                  </div>
                  <div className="model-list">
                    {filtered.map(name => {
                      const isActive = currentModel === name
                      const brief = briefById.get(name)
                      const info = infoById.get(name)
                      const ctx = brief?.context_window ?? info?.context_window
                      const caps = {
                        vision: brief?.supports_vision || info?.supports_vision || false,
                        audio: brief?.supports_audio || info?.supports_audio || false,
                        image:
                          brief?.supports_image_generation ||
                          info?.supports_image_generation ||
                          false,
                      }
                      return (
                        <div
                          key={name}
                          className={'model-list-item' + (isActive ? ' active' : '')}
                          onClick={async () => {
                            const p = providers.find(x => x.id === provider)
                            if (!p) return
                            try {
                              const effectiveKey = apiKey.trim()
                              const resolvedBaseUrl = baseUrl || p.base_url
                              if (effectiveKey) {
                                await configureLlm(effectiveKey, name, provider, resolvedBaseUrl)
                              } else {
                                await switchModelCmd(
                                  name,
                                  provider,
                                  resolvedBaseUrl,
                                  undefined,
                                  'default',
                                )
                              }
                              setCurrentModel(name)
                              persistCurrentProvider(name)
                              onModelChanged?.()
                              listModels()
                                .then(list => {
                                  if (Array.isArray(list)) setAllModels(list)
                                })
                                .catch(() => {})
                              setFeedback({ ok: true, msg: t('models.switchTo', name) })
                            } catch (e: any) {
                              setFeedback({ ok: false, msg: e?.message || t('models.switchFail') })
                            }
                            setTimeout(() => setFeedback(null), 2500)
                          }}
                        >
                          <div className={'model-radio' + (isActive ? ' selected' : '')} />
                          <div className="model-list-name">{name}</div>
                          {(caps.vision || caps.audio || caps.image || ctx !== undefined) && (
                            <div className="model-list-badges">
                              {caps.vision && (
                                <span className="model-badge" title={t('models.capVision')}>
                                  <IconEye size={12} />
                                </span>
                              )}
                              {caps.audio && (
                                <span className="model-badge" title={t('models.capAudio')}>
                                  <IconMic size={12} />
                                </span>
                              )}
                              {caps.image && (
                                <span className="model-badge" title={t('models.capImageGen')}>
                                  <IconImage size={12} />
                                </span>
                              )}
                              {ctx ? (
                                <span
                                  className="model-badge model-badge--ctx"
                                  title={t('models.capContext')}
                                >
                                  {formatContextWindow(ctx)}
                                </span>
                              ) : (
                                <span
                                  className="model-badge model-badge--ctx model-badge--ctx-unknown"
                                  title={t('models.capContextUnknown')}
                                >
                                  ?
                                </span>
                              )}
                            </div>
                          )}
                          {isActive && <IconCheck size={12} className="icon-accent" />}
                        </div>
                      )
                    })}
                  </div>
                </div>
              )
            })()}

            {/* ── 基础 section 的手动输入已移除(大王需求 ⑤);
            models state+列表仍保留,因为 local models section(L538+)要复用 ── */}
            {provider === 'local' && models.length > 0 && (
              <div className="model-list model-list--spaced">
                {models.map(m => {
                  const isActive = currentModel === m
                  return (
                    <div
                      key={m}
                      className={'model-list-item' + (isActive ? ' active' : '')}
                      onClick={() => switchModel(m)}
                    >
                      <div className="model-list-name">{m}</div>
                      {isActive && <IconCheck size={12} className="icon-accent" />}
                      <button
                        className="icon-btn-ghost"
                        onClick={e => {
                          e.stopPropagation()
                          removeModel(m)
                        }}
                        title={t('models.removeTitle')}
                      >
                        <IconTrash2 size={11} />
                      </button>
                    </div>
                  )
                })}
              </div>
            )}

            {/* 检测失败时显示错误信息 */}
            {detectError && <div className="detect-error">{detectError}</div>}
          </Section>

          {/* ── 高级设置：ExecAgent ── */}
          <Section
            title="高级设置"
            description="ExecAgent（子Agent）由 leader 模式下派发，执行任务所使用的模型。"
          >
            <VisionModelSelect
              value={agentModels.exec}
              models={allModels}
              onChange={m => void saveAgentModel('exec', m)}
              t={t}
              placeholder="跟随 Leader"
            />
            <div className="form-hint agent-models-feedback">
              {agentFeedback ? `${agentFeedback.ok ? '✓' : '⚠'} ${agentFeedback.msg}` : ''}
            </div>
          </Section>

          {/* ── Local models (only when provider=local) ── */}
          {isLocal && (
            <Section title={t('models.localModels')} description={t('models.localModelsDesc')}>
              <div className="segmented segmented-wrap">
                {[
                  { id: 'ollama', label: t('models.ollama'), url: 'http://localhost:11434/v1' },
                  { id: 'lmstudio', label: t('models.lmstudio'), url: 'http://localhost:1234/v1' },
                  { id: 'llamacpp', label: t('models.llamacpp'), url: 'http://localhost:8080/v1' },
                ].map(local => (
                  <button
                    key={local.id}
                    className={`segmented-item ${baseUrl === local.url ? 'active' : ''}`}
                    onClick={() => {
                      setBaseUrl(local.url)
                      setFeedback(null)
                    }}
                  >
                    {local.label}
                  </button>
                ))}
              </div>
              <FormRow
                stacked
                label="Context Window (tokens)"
                control={
                  <input
                    className="compact-input input-num"
                    type="number"
                    value={localCtxWindow}
                    onChange={e => {
                      const v = parseInt(e.target.value) || 128000
                      setLocalCtxWindow(v)
                      localStorage.setItem('nuphus_local_context_window', String(v))
                    }}
                    min={1024}
                    max={10000000}
                    step={1024}
                  />
                }
              />
              <div className="compact-input-row input-row-spaced">
                <input
                  className="compact-input input-flex"
                  value={inputVal}
                  onChange={e => setInputVal(e.target.value)}
                  placeholder={t('models.localModelPlaceholder')}
                  onKeyDown={e => {
                    if (e.key === 'Enter') addModel()
                  }}
                />
                <Button variant="primary" size="sm" onClick={addModel} disabled={!inputVal.trim()}>
                  {t('models.addModel')}
                </Button>
              </div>
              <div className="text-caption hint-text">{t('models.localModelsHint')}</div>
            </Section>
          )}
        </>
      )}

      {activeTab === 'custom' && (
        <>
          {/* ── Vision model ── */}
          <Section title={t('models.visionModel')} description={t('models.visionModelDesc')}>
            <FormRow
              stacked
              label={t('models.visionModelLabel')}
              control={
                <VisionModelSelect
                  value={visionModel}
                  models={allModels}
                  filterCapability="vision"
                  onChange={async modelId => {
                    setVisionSaving(true)
                    setVisionFeedback(null)
                    try {
                      await setCapability('vision', modelId)
                      setVisionModel(modelId)
                      setVisionFeedback({ ok: true, msg: t('models.visionSaved') })
                      setTimeout(() => setVisionFeedback(null), 2000)
                    } catch (e: any) {
                      setVisionFeedback({
                        ok: false,
                        msg: e?.message || t('models.visionSaveFail'),
                      })
                    } finally {
                      setVisionSaving(false)
                    }
                  }}
                  t={t}
                />
              }
            />
          </Section>

          {/* ── 本地视觉模型（OCR / YOLO icon_detect）── 全自动下载：安装后
               首启自动补齐，非技术用户零操作。仅呈现需处理的状态：下载中 /
               失败重试 / 缺文件；就绪时简洁说明。YOLO 缺失不影响 OCR。 ── */}
          <Section
            title="本地视觉模型"
            description="屏幕理解所需的 OCR 文字识别与 UI 元素检测模型，随应用自动下载，无需手动操作。"
          >
            {visionDl.status && (
              <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                <span
                  className={`badge ${visionDl.status.ocrReady ? 'badge-accent' : 'badge-neutral'}`}
                >
                  {visionDl.status.ocrReady ? 'OCR 已就绪' : 'OCR 未就绪'}
                </span>
                <span
                  className={`badge ${visionDl.status.yoloReady ? 'badge-accent' : 'badge-neutral'}`}
                >
                  {visionDl.status.yoloReady ? 'UI 检测已就绪' : 'UI 检测未启用'}
                </span>
              </div>
            )}
            {visionDl.status?.dir && (
              <div className="text-caption hint-text" style={{ marginTop: 6 }}>
                模型目录：{visionDl.status.dir}
              </div>
            )}

            {visionDl.downloading || visionDl.progress || visionDl.status?.downloading ? (
              <>
                {visionDl.progress && (
                  <>
                    <div className="stt-dl-progress">
                      {modelsDownloadProgressPct(visionDl.progress) !== null && (
                        <div
                          className="stt-dl-progress-fill"
                          style={{ width: `${modelsDownloadProgressPct(visionDl.progress)}%` }}
                        />
                      )}
                    </div>
                    <div className="stt-dl-progress-text">
                      {modelsDownloadProgressText(visionDl.progress)}
                    </div>
                  </>
                )}
                <div className="text-caption hint-text" style={{ marginTop: 4 }}>
                  正在后台自动下载，可继续使用应用…
                </div>
              </>
            ) : visionDl.error ? (
              <>
                <div className="text-caption" style={{ color: 'var(--error)', marginTop: 8 }}>
                  下载失败：{visionDl.error}
                </div>
                <Button
                  variant="primary"
                  size="sm"
                  style={{ marginTop: 8 }}
                  onClick={visionDl.retry}
                >
                  重试下载
                </Button>
              </>
            ) : visionDl.status === null ? (
              <div className="text-caption hint-text" style={{ marginTop: 6 }}>
                检测中…
              </div>
            ) : visionDl.status.missing.length > 0 ? (
              <>
                <div className="text-caption hint-text" style={{ marginTop: 6 }}>
                  {visionDl.status.ocrReady
                    ? `缺少 ${visionDl.status.missing.join('、')}（可选，仅影响 UI 元素检测）`
                    : `缺少 ${visionDl.status.missing.length} 个模型文件，下载后即可使用屏幕理解`}
                </div>
                <Button
                  variant="primary"
                  size="sm"
                  style={{ marginTop: 8 }}
                  onClick={visionDl.retry}
                >
                  立即下载
                </Button>
              </>
            ) : (
              <div className="text-caption hint-text" style={{ marginTop: 6 }}>
                屏幕理解（OCR + UI 元素检测）已就绪
              </div>
            )}
          </Section>

          {/* ── 语音输入（STT）── 云端优先路由：配置云端识别模型后走
               /audio/transcriptions，本地 sherpa-onnx 是未配置云端时的默认路径。
               就绪态静默（无信息增量不渲染）；仅呈现需用户处理的状态：无麦克风 /
               缺模型可下载 / 云端已配置时本地缺失一行次要提示 ── */}
          <Section
            title="语音输入"
            description="在输入框用语音转文字。配置云端识别模型后优先使用云端识别；未配置则使用本地离线识别（中文优化，无需联网）。"
          >
            <FormRow
              stacked
              label="云端识别模型"
              hint="配置后优先使用云端识别，清除则回退本地离线识别"
              control={
                <VisionModelSelect
                  value={sttModel}
                  models={allModels}
                  filterCapability="audio"
                  placeholder="未配置（使用本地识别）"
                  showVisionIcons={false}
                  onChange={async modelId => {
                    setSttSaving(true)
                    setSttFeedback(null)
                    try {
                      await setCapability('stt', modelId)
                      setSttModel(modelId)
                      setSttFeedback({ ok: true, msg: '云端识别模型已保存' })
                      setTimeout(() => setSttFeedback(null), 2000)
                      // 引擎路由已变化，刷新状态（engine / available / reason）
                      probeStt()
                    } catch (e: any) {
                      setSttFeedback({ ok: false, msg: e?.message || '保存失败' })
                    } finally {
                      setSttSaving(false)
                    }
                  }}
                  t={t}
                />
              }
            />
            {sttFeedback && (
              <div
                className="text-caption"
                style={{ color: sttFeedback.ok ? 'var(--success)' : 'var(--error)', marginTop: 4 }}
              >
                {sttFeedback.msg}
              </div>
            )}
            {sttLocalStatus &&
              (sttLocalStatus.cloud_configured ? (
                <>
                  {!sttLocalStatus.available && (
                    <div className="text-caption hint-text" style={{ marginTop: 0 }}>
                      <span
                        style={{
                          color: 'var(--warning)',
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: 4,
                        }}
                      >
                        <IconAlertTriangle size={12} /> 未检测到麦克风，连接麦克风后即可使用语音输入
                      </span>
                    </div>
                  )}
                  {/* 本地模型状态降级为次要信息：云端已可用，本地仅离线回退需要 */}
                  {!sttLocalStatus.model_dir && (
                    <div className="text-caption hint-text" style={{ marginTop: 4 }}>
                      本地模型未下载（云端识别已可用，仅离线识别时需要）
                    </div>
                  )}
                </>
              ) : sttLocalStatus.reason === 'no_microphone' ? (
                <div className="text-caption hint-text" style={{ marginTop: 0 }}>
                  <span
                    style={{
                      color: 'var(--warning)',
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: 4,
                    }}
                  >
                    <IconAlertTriangle size={12} /> 未检测到麦克风，连接麦克风后即可使用语音输入
                  </span>
                </div>
              ) : sttLocalStatus.reason?.startsWith('model_missing') ? (
                <div>
                  <div className="text-caption hint-text" style={{ marginTop: 0 }}>
                    下载语音模型（约 250 MB）即可开始语音输入
                  </div>
                  {sttDl.progress && (
                    <>
                      <div className="stt-dl-progress">
                        {sttDownloadProgressPct(sttDl.progress) !== null && (
                          <div
                            className="stt-dl-progress-fill"
                            style={{ width: `${sttDownloadProgressPct(sttDl.progress)}%` }}
                          />
                        )}
                      </div>
                      <div className="stt-dl-progress-text">
                        {sttDownloadProgressText(sttDl.progress)}
                      </div>
                    </>
                  )}
                  {sttDl.error && (
                    <div className="text-caption" style={{ color: 'var(--error)', marginTop: 8 }}>
                      下载失败：{sttDl.error}
                    </div>
                  )}
                  <Button
                    variant="primary"
                    size="sm"
                    style={{ marginTop: 8 }}
                    loading={sttDl.downloading}
                    onClick={sttDl.start}
                  >
                    {sttDl.error ? '重试下载' : '下载'}
                  </Button>
                </div>
              ) : null)}
          </Section>

          {/* ── TTS（文字转语音） ── */}
          <Section
            title="文字转语音（TTS）"
            description="配置文字转语音（TTS）模型，用于 AI 回复的语音朗读，支持 OpenAI 兼容的 TTS API。"
          >
            <FormRow
              stacked
              label="TTS 模型"
              control={
                <VisionModelSelect
                  value={ttsModel}
                  models={allModels}
                  placeholder="请选择 TTS 模型"
                  showVisionIcons={false}
                  onChange={async modelId => {
                    setTtsSaving(true)
                    setTtsFeedback(null)
                    try {
                      await setCapability('tts', modelId)
                      setTtsModel(modelId)
                      setTtsFeedback({ ok: true, msg: 'TTS 模型已保存' })
                      setTimeout(() => setTtsFeedback(null), 2000)
                    } catch (e: any) {
                      setTtsFeedback({ ok: false, msg: e?.message || '保存失败' })
                    } finally {
                      setTtsSaving(false)
                    }
                  }}
                  t={t}
                />
              }
            />
          </Section>

          {/* ── 语音克隆 ── 走云端克隆 API：配置语音克隆模型后，工具页「语音克隆」能力可用 ── */}
          <Section
            title="语音克隆"
            description="配置语音克隆模型，用于克隆音色合成语音（工具页「语音克隆」能力）。走云端 API，需先在基础配置中配置对应提供商的 API Key。"
          >
            <FormRow
              stacked
              label="语音克隆模型"
              hint="配置后可在工具页使用语音克隆；清除则禁用该能力"
              control={
                <VisionModelSelect
                  value={voiceModel}
                  models={allModels}
                  placeholder="请选择语音克隆模型"
                  showVisionIcons={false}
                  onChange={async modelId => {
                    setVoiceSaving(true)
                    setVoiceFeedback(null)
                    try {
                      await setCapability('voice', modelId)
                      setVoiceModel(modelId)
                      setVoiceFeedback({ ok: true, msg: '语音克隆模型已保存' })
                      setTimeout(() => setVoiceFeedback(null), 2000)
                    } catch (e: any) {
                      setVoiceFeedback({ ok: false, msg: e?.message || '保存失败' })
                    } finally {
                      setVoiceSaving(false)
                    }
                  }}
                  t={t}
                />
              }
            />
            {voiceFeedback && (
              <div
                className="text-caption"
                style={{
                  color: voiceFeedback.ok ? 'var(--success)' : 'var(--error)',
                  marginTop: 4,
                }}
              >
                {voiceFeedback.msg}
              </div>
            )}
          </Section>
        </>
      )}

      {/* ── Floating toast ── */}
      {(feedback || visionFeedback || ttsFeedback) &&
        createPortal(
          <div
            className={`feedback-toast ${
              feedback?.ok || visionFeedback?.ok || ttsFeedback?.ok
                ? 'feedback-toast--ok'
                : 'feedback-toast--error'
            }`}
          >
            {feedback?.msg || visionFeedback?.msg || ttsFeedback?.msg || ''}
          </div>,
          document.body,
        )}
    </div>
  )
}
