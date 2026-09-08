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
  addProviderModel,
  clearProviderModels,
  getAgentModels,
  getProviderContext,
  setAgentModel,
  setModelContextWindow,
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
  IconEyeOff,
  IconMic,
  IconImage,
  IconAlertTriangle,
  IconRefresh,
  IconBrushCleaning,
  IconEdit3,
  IconPlug,
} from '../../ui/Icons'
import { Section, FormRow } from '../../ui/PageLayout'
import { Button } from '../../ui/Button'
import { useLanguage } from '../../locales'
import { ProviderIcon, hasProviderIcon } from '../components/ProviderIcon'
import '../../styles/models.css'

// ════════════════════════════════════════════════════════════════
// 文案表（页面专用，硬编码中文；既有多语言字典仅保留仍在用 keys）
// 说明：本页历史上大量 t('models.*') 引用并不存在于字典 → 界面泄漏 key。
// 重构后统一为页内文案，可读中文；通用词条（provider/modelList/…）仍走字典。
// ════════════════════════════════════════════════════════════════
const TXT = {
  providerSelectHint: '选择要配置的服务商。deepseek / kimi / openai 等官方服务与本地兼容端点均可。',
  apiKeyLabel: 'API 密钥',
  keyConfiguredBadge: '已配置',
  keyInputPlaceholder: (name: string) => `输入 ${name} 的 API 密钥`,
  keyOverwritePlaceholder: '输入新密钥覆盖现有配置',
  keyShow: '显示',
  keyHide: '隐藏',
  connectBtn: '连接',
  connecting: '连接中…',
  connectTitle: '用当前密钥探测可用模型',
  clearKeyTitle: '清除已保存的密钥',
  clearKeyConfirm:
    '确定要清除该服务商的 API 密钥吗？\n模型与其它配置会保留，清除后需重新输入才能使用云端服务。',
  keyHelp: '密钥仅保存在本机配置中，用于向服务商发起请求；页面不展示已存密钥原文。',
  filterPlaceholder: '筛选模型…',
  baseUrlLabel: '接口地址',
  baseUrlPlaceholder: '自定义接口地址（可选）',
  baseUrlHelp: '默认使用服务商官方地址；OpenAI 兼容代理或本地网关可在此覆盖。',
  modelListTitle: '可用模型',
  currentModelOf: (name: string) => `（当前使用：${name}）`,
  refreshBtn: '刷新',
  refreshing: '刷新中…',
  refreshTitle: '用已保存密钥重新拉取最新模型列表',
  addModelBtn: '+ 手动添加',
  addModelTitle:
    '手动添加模型代号（/v1/models 未返回的灰度或临时模型，如 deepseek-v4.1-flash-expires-on-0910）',
  addModelPlaceholder: '输入模型代号，如 deepseek-v4.1-flash-expires-on-0910',
  addModelConfirm: '添加',
  addModelCancel: '取消',
  addModelRequired: '请输入模型代号',
  addModelSuccess: (id: string) => `已添加模型「${id}」，可在下方点击切换`,
  addModelFail: '添加失败，请确认该服务商已配置密钥',
  baseUrlChangedWarn:
    '接口地址已变更：旧模型列表可能在新地址下不可用（模型代号不存在，或同名模型能力不同）。建议清理后重新拉取。',
  clearModelsBtn: '清理旧模型',
  clearModelsConfirm:
    '确定清理该服务商的旧模型列表吗？\n仅清空模型条目（名称/地址/密钥保留），清理后请点「刷新」拉取新地址的模型。',
  clearModelsSuccess: (n: number) => `已清理 ${n} 个旧模型`,
  clearModelsNone: '没有可清理的旧模型',
  clearModelsFail: '清理失败',
  emptyFiltered: (q: string) => `没有匹配「${q}」的模型`,
  emptyNeedConnect:
    '尚未检测到模型。填写密钥后点击「连接」，或展开列表用「刷新」同步已保存密钥下的模型。',
  emptyNeedDetect: '尚无模型。请先在上方连接服务商，或点击「刷新」拉取已保存密钥下的最新模型。',
  modelsCount: (n: number) => `${n} 个可用模型`,
  configuredCount: (n: number) => `，含 ${n} 个已配置`,
  capVision: '支持图像理解',
  capAudio: '支持语音',
  capImageGen: '支持图像生成',
  editContext: '设置上下文窗口（K tokens）',
  ctxUnknown: '上下文窗口未知',
  removeModelConfirm: (name: string) => `确定从本地列表移除模型「${name}」吗？`,
  clearKeySuccess: '密钥已清除',
  clearKeyFail: '清除失败',
  savingModel: '切换中…',
  saveOk: '保存成功',
  saveFail: '保存失败',
  apiKeyRequired: '请先输入 API 密钥',
  ctxUnitHint: '单位 K（千 tokens），例如 128 = 128K',
  ctxCap: '上下文窗口',
  visionNone: '未配置（使用默认）',
  downloadReady: '已就绪',
  downloadPaused: '下载已暂停',
}

// ════════════════════════════════════════════════════════════════
// 左侧两模块导航
// ════════════════════════════════════════════════════════════════
type CustomNavKey = 'custom' | 'opencode-go' | 'local' | 'capabilities' | 'agents'

/** 模块二「自定义设置」固定导航项：provider 项（custom/opencode-go/local）路由到对应服务商页 */
const CUSTOM_NAV_ITEMS: { key: CustomNavKey; label: string }[] = [
  { key: 'custom', label: 'Custom（自定义/中转站）' },
  { key: 'opencode-go', label: 'Opencode GO 套餐' },
  { key: 'local', label: '本地模型' },
  { key: 'capabilities', label: '图像音频模型' },
  { key: 'agents', label: '子智能体模型' },
]

/** 归入模块二「自定义设置」的服务商 id（不出现在模块一「模型提供商」分组） */
function isModule2Provider(id: string): boolean {
  return id === 'custom' || id === 'opencode-go' || id === 'local'
}

/**
 * 服务商排序：按展示名 A-Z（与 Rust ProviderRegistry::list_info 返回规则一致）。
 * 后端已保证字母序稳定，此函数仅作前端兜底——即使后端顺序漂移，
 * 左侧导航每次打开/刷新也不跳动；新服务商自动按英文名插入正确位置。
 */
function sortProvidersStable(list: ProviderInfo[]): ProviderInfo[] {
  return [...list].sort((a, b) => {
    if (a.name < b.name) return -1
    if (a.name > b.name) return 1
    return 0
  })
}

// ════════════════════════════════════════════════════════════════
// 能力模型选择（vision/audio 等，模块级组件）
// 注：服务商选择已从下拉升级为整页左侧导航列（见下方 models-rail），
// 原 ProviderSelect 组件随之移除——服务商在整页中保持显式可见。
// ════════════════════════════════════════════════════════════════
function VisionModelSelect({
  value,
  models,
  onChange,
  t,
  placeholder,
  showVisionIcons,
  filterCapability,
  menuUp = false,
}: {
  value: string
  models: ModelInfo[]
  onChange: (id: string) => void
  t: (key: string, ...args: string[]) => string
  placeholder?: string
  showVisionIcons?: boolean
  filterCapability?: 'vision' | 'audio'
  /** true = 菜单向上展开（接近页面底部时避免溢出）；默认向下 */
  menuUp?: boolean
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

  // Sort: matching capability first
  const filtered = Array.isArray(models)
    ? (() => {
        let list = [...models]
        if (filterCapability === 'vision') {
          list = list.filter(m => m.supports_vision)
        } else if (filterCapability === 'audio') {
          list = list.filter(m => m.supports_audio)
        }
        if (filterCapability === 'audio') {
          list.sort((a, b) => (b.supports_audio ? 1 : 0) - (a.supports_audio ? 1 : 0))
        } else {
          list.sort((a, b) => (b.supports_vision ? 1 : 0) - (a.supports_vision ? 1 : 0))
        }
        return list
      })()
    : []
  const selected = filtered.find(m => m.id === value)
  const emptyText = placeholder || TXT.visionNone

  return (
    <div className="models-select compact-select-wrap" ref={ref}>
      <div
        className="compact-select-trigger"
        tabIndex={0}
        role="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen(v => !v)}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            setOpen(v => !v)
          }
        }}
      >
        <span className={`${value ? '' : 'select-placeholder'}`}>
          {selected ? `${selected.id} (${selected.provider})` : emptyText}
        </span>
        <span className={`compact-select-arrow ${open ? 'open' : ''}`}>▾</span>
      </div>
      {open && (
        <div className={`compact-select-menu${menuUp ? ' select-menu--up' : ''}`}>
          {/* Clear option */}
          <div
            className={`compact-select-option ${value === '' ? 'active' : ''}`}
            onClick={() => {
              onChange('')
              close()
            }}
          >
            <span className="select-option-name">{emptyText}</span>
          </div>
          {filtered.length === 0 && (
            <div className="compact-select-empty">暂无可选模型（先在上方连接并选择服务商）</div>
          )}
          {filtered.map(m => (
            <div
              key={m.id}
              role="option"
              aria-selected={value === m.id}
              className={`compact-select-option ${value === m.id ? 'active' : ''}`}
              onClick={() => {
                onChange(m.id)
                close()
              }}
            >
              {showVisionIcons !== false && m.supports_vision && (
                <IconEye size={11} className="icon-prefix" />
              )}
              {showVisionIcons !== false && filterCapability === 'audio' && m.supports_audio && (
                <IconMic size={11} className="icon-prefix" />
              )}
              <span className="select-option-name">{m.id}</span>
              <span className="select-option-provider">({m.provider})</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ════════════════════════════════════════════════════════════════
// 本地缓存 / 工具函数
// ════════════════════════════════════════════════════════════════
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

/** 模型行内 Context Window 编辑：未编辑时显示 ctx badge + 铅笔入口；编辑中输入框 */
function RowCtxEditor({
  ctx,
  isEditing,
  value,
  onValueChange,
  onStart,
  onCommit,
  onCancel,
  t,
}: {
  ctx?: number
  isEditing: boolean
  value: string
  onValueChange: (v: string) => void
  onStart: () => void
  onCommit: (name: string) => void
  onCancel: () => void
  t: (key: string, ...args: string[]) => string
}) {
  if (isEditing) {
    return (
      <span className="ctx-inline-wrap" onClick={e => e.stopPropagation()}>
        <input
          autoFocus
          type="number"
          className="ctx-inline-input input-num"
          min={0.1}
          max={10000}
          step={0.001}
          value={value}
          onChange={e => onValueChange(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter') {
              e.stopPropagation()
              onCommit(value)
            } else if (e.key === 'Escape') {
              e.stopPropagation()
              onCancel()
            }
          }}
          onBlur={() => onCommit(value)}
        />
        <span className="ctx-unit">K</span>
      </span>
    )
  }
  return (
    <>
      {ctx && ctx > 0 ? (
        <span className="model-badge model-badge--ctx" title={TXT.ctxCap}>
          {formatContextWindow(ctx)}
        </span>
      ) : (
        <span className="model-badge model-badge--ctx model-badge--ctx-unknown" title={TXT.ctxCap}>
          ?
        </span>
      )}
      <button
        type="button"
        className="icon-btn-ghost model-ctx-edit-btn"
        title={TXT.editContext}
        aria-label={TXT.editContext}
        onClick={e => {
          e.stopPropagation()
          onStart()
        }}
      >
        <IconEdit3 size={11} />
      </button>
    </>
  )
}

// ════════════════════════════════════════════════════════════════
// 页面主体
// ════════════════════════════════════════════════════════════════
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
  // 本地 sherpa-onnx STT 状态（进入 custom tab 时一次性探测）
  const [sttLocalStatus, setSttLocalStatus] = useState<SttStatus | null>(null)
  const [sttModel, setSttModel] = useState('')
  const [sttSaving, setSttSaving] = useState(false)
  const [sttFeedback, setSttFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [ttsModel, setTtsModel] = useState('')
  const [ttsSaving, setTtsSaving] = useState(false)
  const [ttsFeedback, setTtsFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [voiceModel, setVoiceModel] = useState('')
  const [voiceSaving, setVoiceSaving] = useState(false)
  const [voiceFeedback, setVoiceFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [allModels, setAllModels] = useState<ModelInfo[]>([])
  const [agentModels, setAgentModels] = useState<AgentModels>({
    leader: '',
    workflow: '',
    exec: '',
    custom: '',
  })
  const [agentSaving, setAgentSaving] = useState(false)
  const [agentFeedback, setAgentFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [activeView, setActiveView] = useState<'provider' | 'capabilities' | 'agents'>('provider')
  const [hasKey, setHasKey] = useState(false)
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([])
  const [detecting, setDetecting] = useState(false)
  const [clearingKey, setClearingKey] = useState(false)
  const [detectedModels, setDetectedModels] = useState<ProviderModelBrief[]>([])
  const [filterInput, setFilterInput] = useState('')
  const [detectError, setDetectError] = useState<string | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [addOpen, setAddOpen] = useState(false)
  const [addInput, setAddInput] = useState('')
  const [addSaving, setAddSaving] = useState(false)
  const [addError, setAddError] = useState<string | null>(null)
  /** 后端已保存的接口地址（用于检测用户是否改动了 base_url → 提示旧模型可能失效） */
  const [loadedBaseUrl, setLoadedBaseUrl] = useState('')
  const [clearingModels, setClearingModels] = useState(false)

  // 本地 STT 探测（一次性，不轮询；调用失败静默降级）
  const probeStt = useCallback(() => {
    sttStatus()
      .then(s => setSttLocalStatus(s))
      .catch(() => {})
  }, [])

  // STT / 视觉模型下载（事件驱动）
  const sttDl = useSttModelDownload(probeStt)
  const visionDl = useVisionModelDownload()

  // 进入「图像音频模型」页时探测本地 STT 状态 + 刷新视觉模型状态
  useEffect(() => {
    if (activeView !== 'capabilities') return
    probeStt()
    visionDl.refresh()
  }, [activeView, probeStt, visionDl.refresh])

  // Load specified provider state (baseUrl + model list only, key kept on backend)
  const loadProviderState = (id: string) => {
    setBaseUrl('')
    setLoadedBaseUrl('')
    setModels(loadModels(id))
  }

  useEffect(() => {
    Promise.all([
      getSupportedProviders()
        .then(list => {
          if (Array.isArray(list)) setProviders(sortProvidersStable(list))
        })
        .catch(() => {}),
      // provider 归属用 mode 感知的 get_provider_context（后端权威）：同 id 跨段
      // （官方 deepseek vs opencode-go）时 getCurrentConfig 的 provider 可能落在
      // 文件顺序第一段，直接写 localStorage 会把生效模型记到错误 provider 键下。
      Promise.all([
        getCurrentConfig().catch(() => null),
        getProviderContext('leader').catch(() => null),
      ]).then(([cfg, pctx]) => {
        if (cfg) {
          setCurrentModel(cfg.model || '')
          setApiKey('')
          setHasKey(!!cfg.has_key)
          setBaseUrl(cfg.base_url || '')
          setLoadedBaseUrl(cfg.base_url || '')
          if (cfg.configured_providers) setConfiguredProviders(cfg.configured_providers)
        }
        const prov = pctx?.provider || cfg?.provider || 'deepseek'
        try {
          if (cfg?.model) {
            localStorage.setItem(`nuphus_current_model_${prov}`, cfg.model)
          }
        } catch {
          /* localStorage 写入失败不阻塞 UI */
        }
        if (cfg || pctx) setProvider(prov)
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

  // Load provider-local state on provider change
  useEffect(() => {
    setModels(loadModels(provider))
    setDetectedModels(loadDetectedModels(provider))
    setBaseUrl('')
    setLoadedBaseUrl('')
    setFilterInput('')
    setCtxOverrides({})
    setEditingCtxModel(null)
    editingCtxRef.current = null
    try {
      const saved = localStorage.getItem(`nuphus_current_model_${provider}`)
      if (saved) setCurrentModel(saved)
    } catch {
      setCurrentModel('')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider])

  // ── 进入远程服务商页自动同步网关模型列表（每次挂载每 provider 一次，静默）──
  // 后端 refresh_provider_models = 拉 /v1/models + upsert_provider_models 持久化进
  // providers.toml 段：使网关实际在售的新模型（如 opencode-go 的 deepseek 系列）
  // 无需手动点「刷新」即出现在本页列表与弹窗 hover（list_models 数据源）。
  // 网络失败静默降级（保留 localStorage 检测缓存 + 磁盘段）；本地段跳过（无远程）。
  const autoSyncedRef = useRef<Set<string>>(new Set())
  const providerRef = useRef(provider)
  useEffect(() => {
    providerRef.current = provider
  }, [provider])
  useEffect(() => {
    if (activeView !== 'provider') return
    if (!provider || provider === 'local' || provider === 'custom') return
    if (!configuredProviders.includes(provider)) return
    if (autoSyncedRef.current.has(provider)) return
    autoSyncedRef.current.add(provider)
    const target = provider
    const p = providers.find(x => x.id === target)
    refreshProviderModels(target, p?.base_url || undefined)
      .then(models => {
        if (providerRef.current !== target || !Array.isArray(models)) return
        setDetectedModels(models)
        saveDetectedModels(target, models)
        return listModels()
      })
      .then(list => {
        if (Array.isArray(list)) setAllModels(list)
      })
      .catch(() => {
        /* 静默：失败不打扰，手动「刷新」入口仍在 */
      })
  }, [activeView, provider, configuredProviders, providers])

  // local 默认上下文（仅在用户显式设置过时作为新模型默认值）
  const [localCtxWindow, setLocalCtxWindow] = useState<number | null>(() => {
    try {
      const raw = localStorage.getItem('nuphus_local_context_window')
      if (!raw) return null
      const v = parseInt(raw, 10)
      return Number.isInteger(v) && v > 0 ? v : null
    } catch {
      return null
    }
  })
  const [ctxOverrides, setCtxOverrides] = useState<Record<string, number>>({})
  const [editingCtxModel, setEditingCtxModel] = useState<string | null>(null)
  const [editingCtxValue, setEditingCtxValue] = useState('')
  const editingCtxRef = useRef<string | null>(null)

  const curProvider = providers.find(p => p.id === provider)
  const isCustom = provider === 'custom'
  const isLocal = provider === 'local'
  /** 模块一「模型提供商」= 官方远程服务商（排除归入模块二的 custom / opencode-go / local） */
  const module1Providers = providers.filter(p => !isModule2Provider(p.id))

  // 持久化当前 provider + current model，供快捷切换弹窗读取
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
    setApiKey('')
    setFeedback(null)
    setDetectError(null)
    setRefreshError(null)
    setDetecting(false)
    setHasKey(configuredProviders.includes(id))
  }

  /** 左侧导航选择某个服务商：切到 provider 内容页并加载该服务商状态 */
  const openProviderView = (id: string) => {
    setActiveView('provider')
    handleProviderChange(id)
  }

  /** 模块二「自定义设置」选中态：custom/opencode-go/local 跟随对应服务商页 */
  const isCustomNavActive = (key: CustomNavKey): boolean => {
    if (key === 'capabilities') return activeView === 'capabilities'
    if (key === 'agents') return activeView === 'agents'
    return activeView === 'provider' && provider === key
  }

  /** 通过 /v1/models 检测 API key 并列出可用模型 */
  const detectModels = async () => {
    const key = apiKey.trim()
    if (!key) {
      setDetectError(TXT.apiKeyRequired)
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
      listModels()
        .then(list => {
          if (Array.isArray(list)) setAllModels(list)
        })
        .catch(() => {})
    } catch (e: any) {
      setRefreshError(e?.message || '刷新失败，请检查密钥与网络')
    } finally {
      setRefreshing(false)
    }
  }

  /** 手动添加模型代号到服务商配置（灰度/临时模型；不依赖 /v1/models 返回） */
  const handleAddModel = async () => {
    const id = addInput.trim()
    if (!id) {
      setAddError(TXT.addModelRequired)
      return
    }
    setAddSaving(true)
    setAddError(null)
    try {
      await addProviderModel(provider, id)
      setAddOpen(false)
      setAddInput('')
      setFeedback({ ok: true, msg: TXT.addModelSuccess(id) })
      setTimeout(() => setFeedback(null), 2500)
      // 后端已并入 config.toml → list_models 重新读取即含新条目（configured 并集）
      listModels()
        .then(list => {
          if (Array.isArray(list)) setAllModels(list)
        })
        .catch(() => {})
    } catch (e: any) {
      setAddError(e?.message || TXT.addModelFail)
    } finally {
      setAddSaving(false)
    }
  }

  /** 清理该服务商旧模型列表（接口地址变更后调用；清空 config.toml models + 本地检测缓存） */
  const handleClearModels = async () => {
    if (!window.confirm(TXT.clearModelsConfirm)) return
    setClearingModels(true)
    try {
      const n = (await clearProviderModels(provider)) ?? 0
      // 本地检测缓存同清，避免并集里残留旧地址模型
      setDetectedModels([])
      saveDetectedModels(provider, [])
      listModels()
        .then(list => {
          if (Array.isArray(list)) setAllModels(list)
        })
        .catch(() => {})
      setFeedback({ ok: true, msg: n > 0 ? TXT.clearModelsSuccess(n) : TXT.clearModelsNone })
      // 清理后自动尝试拉取新地址的模型列表（key 有效时一步到位，失败由刷新区提示）
      refreshModels()
    } catch (e: any) {
      setFeedback({ ok: false, msg: e?.message || TXT.clearModelsFail })
    } finally {
      setClearingModels(false)
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  /** 清除当前 provider 已存储的 API Key（仅清 key，保留 provider/model 配置） */
  const handleClearKey = async () => {
    if (!window.confirm(TXT.clearKeyConfirm)) return
    setClearingKey(true)
    try {
      await clearProviderApiKey(provider)
      const cfg = await getCurrentConfig()
      if (cfg) {
        setApiKey('')
        setBaseUrl(cfg.base_url || '')
        setLoadedBaseUrl(cfg.base_url || '')
        if (cfg.configured_providers) {
          setConfiguredProviders(cfg.configured_providers)
          setHasKey(cfg.configured_providers.includes(provider))
        }
      }
      setFeedback({ ok: true, msg: TXT.clearKeySuccess })
    } catch (e: any) {
      setFeedback({ ok: false, msg: e?.message || TXT.clearKeyFail })
    } finally {
      setClearingKey(false)
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  // ── 手动添加模型（仅 local 列表使用；basic 模型的真正添加走「连接后点击模型」）──
  const addModel = async () => {
    const name = inputVal.trim()
    if (!name) return
    const list = loadModels(provider)
    if (list.includes(name)) {
      setFeedback({ ok: false, msg: `模型「${name}」已在列表中` })
      setTimeout(() => setFeedback(null), 1800)
      return
    }
    list.push(name)
    saveModels(provider, list)
    setModels(list)
    setInputVal('')

    const effectiveKey = apiKey.trim() ? apiKey : ''
    if (!isLocal && !effectiveKey.trim() && !hasKey) {
      setFeedback({ ok: false, msg: TXT.apiKeyRequired })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    const p = providers.find(x => x.id === provider)
    if (p) {
      const resolvedBaseUrl = baseUrl || p.base_url
      const addCtxArg =
        isLocal &&
        localCtxWindow != null &&
        ctxOverrides[name] === undefined &&
        !allModels.some(
          m =>
            m.provider === provider &&
            m.id === name &&
            m.context_window != null &&
            m.context_window > 0,
        )
          ? localCtxWindow
          : undefined
      try {
        if (effectiveKey) {
          await configureLlm(effectiveKey, name, provider, resolvedBaseUrl, addCtxArg)
        } else {
          await switchModelCmd(name, provider, resolvedBaseUrl, addCtxArg, 'global')
        }
        setCurrentModel(name)
        persistCurrentProvider(name)
        onModelChanged?.()
        listModels()
          .then(list => {
            if (Array.isArray(list)) setAllModels(list)
          })
          .catch(() => {})
        setFeedback({ ok: true, msg: `已切换到 ${name}` })
      } catch (e: any) {
        setFeedback({ ok: false, msg: e?.message || '切换失败' })
      }
      setTimeout(() => setFeedback(null), 2500)
    }
  }

  // Agent 级模型保存（高级设置）：空串 = 清除（跟随默认模型）
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

  /** 从本地列表移除模型（不删除后端已配置项） */
  const removeModel = (name: string) => {
    if (!window.confirm(TXT.removeModelConfirm(name))) return
    const list = loadModels(provider).filter(m => m !== name)
    saveModels(provider, list)
    setModels(list)
    if (currentModel === name) {
      setCurrentModel('')
    }
  }

  /** 模型当前生效 context_window（行内已保存覆盖 > 检测 brief > list_models info） */
  const rowCtx = (name: string): number | undefined => {
    const ov = ctxOverrides[name]
    if (ov !== undefined) return ov
    const brief = detectedModels.find(d => d.id === name)
    if (brief?.context_window !== undefined && brief?.context_window !== null) {
      return brief.context_window
    }
    const info = allModels.find(m => m.provider === provider && m.id === name)
    return info?.context_window
  }

  /** 该模型是否已有 per-model 显式 context_window */
  const hasExplicitCtx = (name: string): boolean =>
    ctxOverrides[name] !== undefined ||
    detectedModels.some(d => d.id === name && d.context_window != null && d.context_window > 0) ||
    allModels.some(
      m =>
        m.provider === provider &&
        m.id === name &&
        m.context_window != null &&
        m.context_window > 0,
    )

  const startCtxEdit = (name: string) => {
    if (editingCtxRef.current !== null) return
    const cur = rowCtx(name)
    editingCtxRef.current = name
    setEditingCtxModel(name)
    setEditingCtxValue(cur !== undefined && cur > 0 ? String(cur / 1000) : '')
  }

  const cancelCtxEdit = () => {
    if (editingCtxRef.current === null) return
    editingCtxRef.current = null
    setEditingCtxModel(null)
    setEditingCtxValue('')
  }

  const commitCtxEdit = async (name: string, rawValue: string) => {
    if (editingCtxRef.current !== name) return
    editingCtxRef.current = null
    setEditingCtxModel(null)
    const raw = String(rawValue ?? '').trim()
    if (raw === '') return
    const k = Number(raw)
    if (!Number.isFinite(k) || k <= 0) {
      setFeedback({ ok: false, msg: '上下文窗口需为大于 0 的数字（单位 K）' })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    const v = Math.round(k * 1000)
    if (v < 1 || v > 10000000) {
      setFeedback({ ok: false, msg: '上下文窗口需在 1 ~ 10,000,000K 之间' })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    const p = providers.find(x => x.id === provider)
    if (!p) return
    try {
      await setModelContextWindow(provider, name, v)
      setCtxOverrides(prev => ({ ...prev, [name]: v }))
      setFeedback({ ok: true, msg: `上下文窗口已保存（${formatContextWindow(v)}）` })
      listModels()
        .then(list => {
          if (Array.isArray(list)) setAllModels(list)
        })
        .catch(() => {})
    } catch (e: any) {
      setFeedback({ ok: false, msg: e?.message || '保存失败' })
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  const switchModel = async (name: string) => {
    if (providersLoading || providers.length === 0) {
      setFeedback({ ok: false, msg: '服务商列表尚未加载完成' })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    const p = providers.find(x => x.id === provider)
    if (!p) {
      setFeedback({ ok: false, msg: '切换失败：未找到服务商' })
      setTimeout(() => setFeedback(null), 2500)
      return
    }
    setFeedback(null)
    const resolvedBaseUrl = baseUrl || p.base_url
    const ctxArg =
      isLocal && localCtxWindow != null && !hasExplicitCtx(name) ? localCtxWindow : undefined
    try {
      await switchModelCmd(name, provider, resolvedBaseUrl, ctxArg, 'default')
      setCurrentModel(name)
      persistCurrentProvider(name)
      onModelChanged?.()
      setFeedback({ ok: true, msg: `已切换到 ${name}` })
    } catch (e: any) {
      setFeedback({ ok: false, msg: e?.message || '切换失败' })
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  // ── 页面渲染 ──
  const loadingView = providersLoading ? (
    <div className="models-page-loading">正在加载服务商列表…</div>
  ) : null

  return (
    <div className="models-page-layout">
      {/* ── 左侧：两模块分组导航（模型提供商 / 自定义设置） ── */}
      <aside className="models-rail">
        <div className="models-rail-scroll">
          <div className="models-rail-group">
            <div className="models-rail-group-title">模型提供商</div>
            {providersLoading ? (
              <div className="models-rail-note">正在加载服务商…</div>
            ) : (
              <div className="models-rail-list">
                {module1Providers.map(p => {
                  const isActive = activeView === 'provider' && p.id === provider
                  const isConfigured = configuredProviders.includes(p.id)
                  return (
                    <button
                      type="button"
                      key={p.id}
                      className={[
                        'models-rail-item',
                        isActive ? 'active' : '',
                        isConfigured ? 'configured' : '',
                      ]
                        .filter(Boolean)
                        .join(' ')}
                      onClick={() => openProviderView(p.id)}
                      title={isConfigured ? `${p.name}（已配置密钥）` : `${p.name}（未配置密钥）`}
                    >
                      {hasProviderIcon(p.id) ? (
                        <ProviderIcon provider={p.id} size={16} />
                      ) : (
                        /* 无图标 provider（custom / local）保留等宽占位，保证各行图标位对齐 */
                        <span className="provider-icon" style={{ width: 16, height: 16 }} />
                      )}
                      <span className="models-rail-name">{p.name}</span>
                      {isConfigured && (
                        <span className="model-badge models-rail-badge">已配置</span>
                      )}
                    </button>
                  )
                })}
              </div>
            )}
          </div>
          <div className="models-rail-group">
            <div className="models-rail-group-title">自定义设置</div>
            <div className="models-rail-list">
              {CUSTOM_NAV_ITEMS.map(item => (
                <button
                  type="button"
                  key={item.key}
                  className={[
                    'models-rail-item',
                    'models-rail-item--sub',
                    isCustomNavActive(item.key) ? 'active' : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                  onClick={() => {
                    if (item.key === 'capabilities' || item.key === 'agents') {
                      setActiveView(item.key)
                    } else {
                      openProviderView(item.key)
                    }
                  }}
                >
                  <span className="models-rail-name">{item.label}</span>
                </button>
              ))}
            </div>
          </div>
        </div>
      </aside>

      {/* ── 右侧：按左侧所选显示对应内容页 ── */}
      <div className="models-main">
        {loadingView}
        {!providersLoading && (
          <>
            <div className="models-main-scroll">
              {/* ═══════════ 模型服务商 + 可用模型（模块一 / Custom / Opencode GO / 本地模型共用） ═══════════ */}
              {activeView === 'provider' && (
                <>
                  {/* ── 服务商与连接：选择入口在左侧导航列 ── */}
                  <Section
                    title="模型服务商"
                    description="在左侧导航中选择服务商后配置；以下为该服务商访问密钥与可用模型。"
                  >
                    <div className="models-provider-current">
                      {hasProviderIcon(provider) && <ProviderIcon provider={provider} size={18} />}
                      <span className="models-provider-current-name">
                        {curProvider?.name || provider}
                      </span>
                      <span className="models-provider-current-id">{provider}</span>
                      {hasKey ? (
                        <span className="model-badge label-badge">已配置</span>
                      ) : (
                        <span className="models-provider-current-hint">尚未配置密钥</span>
                      )}
                    </div>

                    {!isLocal && (
                      <FormRow
                        stacked
                        label={
                          <span className="models-field-label">
                            <IconPlug size={12} className="icon-prefix" />
                            {TXT.apiKeyLabel}
                            {hasKey && <span className="model-badge label-badge">已配置</span>}
                          </span>
                        }
                        hint={TXT.keyHelp}
                        control={
                          <div className="models-key-row">
                            <div className="models-key-field">
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
                                    ? TXT.keyOverwritePlaceholder
                                    : TXT.keyInputPlaceholder(curProvider?.name || provider)
                                }
                              />
                              <button
                                type="button"
                                className="models-key-eye"
                                onClick={() => setShowKey(v => !v)}
                                tabIndex={-1}
                                title={showKey ? TXT.keyHide : TXT.keyShow}
                                aria-label={showKey ? TXT.keyHide : TXT.keyShow}
                              >
                                {showKey ? <IconEyeOff size={14} /> : <IconEye size={14} />}
                              </button>
                            </div>
                            <Button
                              variant="primary"
                              size="sm"
                              onClick={detectModels}
                              disabled={detecting || !apiKey.trim()}
                              title={TXT.connectTitle}
                            >
                              {detecting ? TXT.connecting : TXT.connectBtn}
                            </Button>
                            {hasKey && (
                              <button
                                type="button"
                                className="models-key-clear"
                                onClick={handleClearKey}
                                disabled={clearingKey}
                                title={TXT.clearKeyTitle}
                                aria-label={TXT.clearKeyTitle}
                              >
                                <IconBrushCleaning size={13} />
                              </button>
                            )}
                          </div>
                        }
                      />
                    )}

                    {detectError && <div className="detect-error">{detectError}</div>}

                    {(isCustom || isLocal) && (
                      <FormRow
                        stacked
                        label="接口地址"
                        hint={TXT.baseUrlHelp}
                        control={
                          <input
                            className="compact-input"
                            value={baseUrl}
                            onChange={e => setBaseUrl(e.target.value)}
                            placeholder={
                              isLocal ? 'http://localhost:11434/v1' : TXT.baseUrlPlaceholder
                            }
                          />
                        }
                      />
                    )}

                    {(isCustom || isLocal) && baseUrl.trim() !== loadedBaseUrl.trim() && (
                      <div className="models-baseurl-warn" role="alert">
                        <span className="models-baseurl-warn-text">{TXT.baseUrlChangedWarn}</span>
                        <button
                          type="button"
                          className="models-add-submit"
                          onClick={handleClearModels}
                          disabled={clearingModels}
                        >
                          {clearingModels ? `${TXT.clearModelsBtn}…` : TXT.clearModelsBtn}
                        </button>
                      </div>
                    )}

                    {isLocal && (
                      <div className="models-local-presets">
                        {[
                          { id: 'ollama', label: 'Ollama', url: 'http://localhost:11434/v1' },
                          { id: 'lmstudio', label: 'LM Studio', url: 'http://localhost:1234/v1' },
                          { id: 'llamacpp', label: 'llama.cpp', url: 'http://localhost:8080/v1' },
                        ].map(local => (
                          <button
                            key={local.id}
                            type="button"
                            className={`models-preset-btn ${baseUrl === local.url ? 'active' : ''}`}
                            onClick={() => {
                              setBaseUrl(local.url)
                              setFeedback(null)
                            }}
                          >
                            {local.label}
                          </button>
                        ))}
                      </div>
                    )}
                  </Section>

                  {/* ── 可用模型列表（后端已配置 + 本地检测结果并集）── */}
                  <Section
                    title={TXT.modelListTitle}
                    description={
                      currentModel
                        ? TXT.currentModelOf(currentModel)
                        : '选择一个模型作为默认使用（点击行即可切换）。'
                    }
                    actions={
                      <>
                        <button
                          type="button"
                          className="models-refresh-btn"
                          onClick={() => {
                            setAddOpen(v => !v)
                            setAddError(null)
                          }}
                          title={TXT.addModelTitle}
                        >
                          {TXT.addModelBtn}
                        </button>
                        <button
                          type="button"
                          className="models-refresh-btn"
                          onClick={refreshModels}
                          disabled={refreshing}
                          title={TXT.refreshTitle}
                        >
                          <IconRefresh size={13} className={refreshing ? 'is-spinning' : ''} />
                          {refreshing ? TXT.refreshing : TXT.refreshBtn}
                        </button>
                      </>
                    }
                  >
                    <input
                      className="compact-input models-filter"
                      value={filterInput}
                      onChange={e => setFilterInput(e.target.value)}
                      placeholder={TXT.filterPlaceholder}
                    />
                    {addOpen && (
                      <div className="models-add-row">
                        <input
                          className="compact-input models-add-input"
                          autoFocus
                          value={addInput}
                          onChange={e => setAddInput(e.target.value)}
                          onKeyDown={e => {
                            if (e.key === 'Enter') handleAddModel()
                            if (e.key === 'Escape') {
                              setAddOpen(false)
                              setAddError(null)
                            }
                          }}
                          placeholder={TXT.addModelPlaceholder}
                          aria-label={TXT.addModelPlaceholder}
                        />
                        <button
                          type="button"
                          className="models-add-submit"
                          onClick={handleAddModel}
                          disabled={addSaving}
                        >
                          {addSaving ? `${TXT.addModelConfirm}…` : TXT.addModelConfirm}
                        </button>
                        <button
                          type="button"
                          className="models-add-cancel"
                          onClick={() => {
                            setAddOpen(false)
                            setAddError(null)
                          }}
                        >
                          {TXT.addModelCancel}
                        </button>
                      </div>
                    )}
                    {addError && <div className="detect-error">{addError}</div>}
                    {refreshError && <div className="detect-error">{refreshError}</div>}

                    {(() => {
                      const configured = allModels
                        .filter(m => m.provider === provider)
                        .map(m => m.id)
                      const display = Array.from(
                        new Set([...detectedModels.map(d => d.id), ...configured]),
                      )
                      const q = filterInput.trim().toLowerCase()
                      const filtered = q
                        ? display.filter(m => m.toLowerCase().includes(q))
                        : display
                      if (detectedModels.length === 0 && configured.length === 0 && !detecting) {
                        return (
                          <div className="models-empty">
                            <div className="models-empty-title">
                              {q ? TXT.emptyFiltered(filterInput) : TXT.emptyNeedConnect}
                            </div>
                            <div className="models-empty-hint">
                              {hasKey
                                ? '已保存密钥可直接点击右上角「刷新」同步模型。'
                                : '密钥仅保存在本机；配置后点击「连接」探测可用模型。'}
                            </div>
                          </div>
                        )
                      }
                      if (detecting) {
                        return <div className="models-empty">正在连接并获取模型列表…</div>
                      }
                      if (filtered.length === 0) {
                        return (
                          <div className="models-empty">
                            {q ? TXT.emptyFiltered(filterInput) : TXT.emptyNeedDetect}
                          </div>
                        )
                      }
                      const briefById = new Map<string, ProviderModelBrief>(
                        detectedModels.map(d => [d.id, d]),
                      )
                      const infoById = new Map<string, ModelInfo>(allModels.map(m => [m.id, m]))
                      return (
                        <div className="models-list-wrap">
                          <div className="detect-status">
                            {TXT.modelsCount(display.length)}
                            {configured.length > 0 && TXT.configuredCount(configured.length)}
                            <span className="detect-status-hint">· 点击行切换为默认模型</span>
                          </div>
                          <div className="model-list">
                            {filtered.map(name => {
                              const isActive = currentModel === name
                              const brief = briefById.get(name)
                              const info = infoById.get(name)
                              const ctx =
                                ctxOverrides[name] ?? brief?.context_window ?? info?.context_window
                              const caps = {
                                vision: brief?.supports_vision || info?.supports_vision || false,
                                audio: brief?.supports_audio || info?.supports_audio || false,
                                image:
                                  brief?.supports_image_generation ||
                                  info?.supports_image_generation ||
                                  false,
                              }
                              const rowCtxArg =
                                isLocal && localCtxWindow != null && !hasExplicitCtx(name)
                                  ? localCtxWindow
                                  : undefined
                              return (
                                <div
                                  key={name}
                                  className={'model-list-item' + (isActive ? ' active' : '')}
                                  role="button"
                                  tabIndex={0}
                                  onClick={async () => {
                                    if (editingCtxModel === name) return
                                    const p = providers.find(x => x.id === provider)
                                    if (!p) return
                                    try {
                                      const effectiveKey = apiKey.trim()
                                      const resolvedBaseUrl = baseUrl || p.base_url
                                      if (effectiveKey) {
                                        await configureLlm(
                                          effectiveKey,
                                          name,
                                          provider,
                                          resolvedBaseUrl,
                                        )
                                      } else {
                                        await switchModelCmd(
                                          name,
                                          provider,
                                          resolvedBaseUrl,
                                          rowCtxArg,
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
                                      setFeedback({ ok: true, msg: `已切换到 ${name}` })
                                    } catch (e: any) {
                                      setFeedback({
                                        ok: false,
                                        msg: e?.message || '切换失败',
                                      })
                                    }
                                    setTimeout(() => setFeedback(null), 2500)
                                  }}
                                  onKeyDown={e => {
                                    if (e.key === 'Enter' || e.key === ' ') {
                                      e.preventDefault()
                                      ;(e.currentTarget as HTMLElement).click()
                                    }
                                  }}
                                >
                                  <div className={'model-radio' + (isActive ? ' selected' : '')} />
                                  <div className="model-list-name">
                                    {name}
                                    {provider === 'opencode-go' && (
                                      <span className="model-go-badge" title="OpenCode Go 网关">
                                        GO
                                      </span>
                                    )}
                                  </div>
                                  <div className="model-list-badges">
                                    {caps.vision && (
                                      <span className="model-badge" title="支持图像理解">
                                        <IconEye size={12} />
                                      </span>
                                    )}
                                    {caps.audio && (
                                      <span className="model-badge" title="支持语音">
                                        <IconMic size={12} />
                                      </span>
                                    )}
                                    {caps.image && (
                                      <span className="model-badge" title="支持图像生成">
                                        <IconImage size={12} />
                                      </span>
                                    )}
                                    <RowCtxEditor
                                      ctx={ctx}
                                      isEditing={editingCtxModel === name}
                                      value={editingCtxValue}
                                      onValueChange={setEditingCtxValue}
                                      onStart={() => startCtxEdit(name)}
                                      onCommit={(raw: string) => commitCtxEdit(name, raw)}
                                      onCancel={cancelCtxEdit}
                                      t={t}
                                    />
                                  </div>
                                  {isActive && <IconCheck size={13} className="model-list-check" />}
                                </div>
                              )
                            })}
                          </div>
                        </div>
                      )
                    })()}

                    {/* local：手动模型列表与默认上下文 */}
                    {provider === 'local' && models.length > 0 && (
                      <div className="model-list model-list--spaced">
                        {models.map(m => {
                          const isActive = currentModel === m
                          const mctx = ctxOverrides[m] ?? rowCtx(m)
                          return (
                            <div
                              key={m}
                              className={'model-list-item' + (isActive ? ' active' : '')}
                              role="button"
                              tabIndex={0}
                              onClick={() => {
                                if (editingCtxModel === m) return
                                void switchModel(m)
                              }}
                            >
                              <div className="model-list-name">{m}</div>
                              <div className="model-list-badges">
                                <RowCtxEditor
                                  ctx={mctx}
                                  isEditing={editingCtxModel === m}
                                  value={editingCtxValue}
                                  onValueChange={setEditingCtxValue}
                                  onStart={() => startCtxEdit(m)}
                                  onCommit={(raw: string) => commitCtxEdit(m, raw)}
                                  onCancel={cancelCtxEdit}
                                  t={t}
                                />
                              </div>
                              {isActive && <IconCheck size={13} className="model-list-check" />}
                              <button
                                type="button"
                                className="icon-btn-ghost icon-btn-clear"
                                onClick={e => {
                                  e.stopPropagation()
                                  removeModel(m)
                                }}
                                title="从本地列表移除"
                                aria-label="从本地列表移除"
                              >
                                <IconTrash2 size={12} />
                              </button>
                            </div>
                          )
                        })}
                      </div>
                    )}

                    {provider === 'local' && (
                      <div className="models-local-ctx">
                        <FormRow
                          stacked
                          label="本地模型默认上下文（K tokens）"
                          hint="为空则每个本地模型按需单独设置；填写后切换新模型时自动应用。"
                          control={
                            <input
                              className="compact-input input-num"
                              type="number"
                              value={localCtxWindow ?? ''}
                              onChange={e => {
                                const raw = e.target.value.trim()
                                if (raw === '') {
                                  setLocalCtxWindow(null)
                                  localStorage.removeItem('nuphus_local_context_window')
                                  return
                                }
                                const v = parseInt(raw, 10)
                                if (!Number.isInteger(v) || v <= 0) return
                                setLocalCtxWindow(v)
                                localStorage.setItem('nuphus_local_context_window', String(v))
                              }}
                              min={1024}
                              max={10000000}
                              step={1024}
                              placeholder="例如 128"
                            />
                          }
                        />
                      </div>
                    )}
                  </Section>

                  {/* ── 高级：ExecAgent 子模型 已迁移至左侧「自定义设置 → 子智能体模型」页 ── */}
                </>
              )}

              {/* local：添加自定义模型（手动录入，用于本地网关模型列表外补充） */}
              {activeView === 'provider' && provider === 'local' && (
                <div className="models-local-add">
                  <FormRow
                    stacked
                    label="添加本地模型"
                    hint="手动输入模型名称并回车，随后会在上方列表出现（可设置上下文并切换）。"
                    control={
                      <div className="compact-input-row input-row-spaced">
                        <input
                          className="compact-input input-flex"
                          value={inputVal}
                          onChange={e => setInputVal(e.target.value)}
                          placeholder="例如 qwen2.5:7b"
                          onKeyDown={e => {
                            if (e.key === 'Enter') addModel()
                          }}
                        />
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={addModel}
                          disabled={!inputVal.trim()}
                        >
                          添加
                        </Button>
                      </div>
                    }
                  />
                </div>
              )}

              {/* ═══════════ 图像音频模型：视觉 / 语音 / 朗读（原「自定义能力」tab 内容） ═══════════ */}
              {activeView === 'capabilities' && (
                <>
                  {/* ── 云端图像理解模型 ── */}
                  <Section
                    title="图像理解"
                    description="配置图像理解模型后，对话中的截图 / 图片可被自动识别（OCR 与界面描述）。留空表示使用默认模型。"
                  >
                    <FormRow
                      stacked
                      className="models-form-row--dedup"
                      label="图像理解模型"
                      hint="需模型本身支持视觉输入（列表中会以图标标注）。"
                      control={
                        <VisionModelSelect
                          value={visionModel}
                          models={allModels}
                          filterCapability="vision"
                          placeholder="未配置（使用默认模型）"
                          onChange={async modelId => {
                            setVisionSaving(true)
                            setVisionFeedback(null)
                            try {
                              await setCapability('vision', modelId)
                              setVisionModel(modelId)
                              setVisionFeedback({ ok: true, msg: '图像理解模型已保存' })
                              setTimeout(() => setVisionFeedback(null), 2000)
                            } catch (e: any) {
                              setVisionFeedback({
                                ok: false,
                                msg: e?.message || '保存失败',
                              })
                            } finally {
                              setVisionSaving(false)
                            }
                          }}
                          t={t}
                        />
                      }
                    />
                    {visionFeedback && (
                      <div
                        className={`text-caption${visionFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {visionFeedback.msg}
                      </div>
                    )}
                  </Section>

                  {/* ── 本地视觉模型（OCR / UI 元素检测）：随应用自动下载 ── */}
                  <Section
                    title="本地视觉模型（OCR / UI 检测）"
                    description="屏幕理解所需的本地 OCR 与界面元素检测模型。随应用自动下载，无需手动操作；仅当缺少文件时需要处理。"
                  >
                    {visionDl.status && (
                      <div className="models-dl-badges">
                        <span
                          className={`model-badge ${visionDl.status.ocrReady ? 'model-badge--ok' : ''}`}
                        >
                          {visionDl.status.ocrReady ? 'OCR 已就绪' : 'OCR 未就绪'}
                        </span>
                        <span
                          className={`model-badge ${visionDl.status.yoloReady ? 'model-badge--ok' : ''}`}
                        >
                          {visionDl.status.yoloReady ? 'UI 检测已就绪' : 'UI 检测未启用'}
                        </span>
                      </div>
                    )}
                    {visionDl.status?.dir && (
                      <div className="text-caption hint-text">模型目录：{visionDl.status.dir}</div>
                    )}

                    {visionDl.downloading || visionDl.progress || visionDl.status?.downloading ? (
                      <>
                        {visionDl.progress && (
                          <>
                            <div className="stt-dl-progress">
                              {modelsDownloadProgressPct(visionDl.progress) !== null && (
                                <div
                                  className="stt-dl-progress-fill"
                                  style={{
                                    width: `${modelsDownloadProgressPct(visionDl.progress)}%`,
                                  }}
                                />
                              )}
                            </div>
                            <div className="stt-dl-progress-text">
                              {modelsDownloadProgressText(visionDl.progress)}
                            </div>
                          </>
                        )}
                        <div className="text-caption hint-text">
                          正在后台自动下载，下载完成即可使用屏幕理解…
                        </div>
                      </>
                    ) : visionDl.error ? (
                      <>
                        <div className="detect-error">下载失败：{visionDl.error}</div>
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
                      <div className="text-caption hint-text">检测中…</div>
                    ) : visionDl.status.missing.length > 0 ? (
                      <>
                        <div className="text-caption hint-text">
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
                      <div className="text-caption hint-text">
                        屏幕理解（OCR + UI 元素检测）已就绪
                      </div>
                    )}
                  </Section>

                  {/* ── 语音输入（STT）：云端优先，本地 sherpa-onnx 兜底 ── */}
                  <Section
                    title="语音输入"
                    description="在输入框用语音转文字。配置云端识别模型后优先使用云端识别；未配置则使用本地离线识别（中文优化，无需联网）。"
                  >
                    <FormRow
                      stacked
                      label="云端识别模型"
                      hint="配置后优先使用云端识别，清除则回退本地离线识别。"
                      control={
                        <VisionModelSelect
                          value={sttModel}
                          models={allModels}
                          filterCapability="audio"
                          placeholder="未配置（使用本地识别）"
                          showVisionIcons={false}
                          menuUp
                          onChange={async modelId => {
                            setSttSaving(true)
                            setSttFeedback(null)
                            try {
                              await setCapability('stt', modelId)
                              setSttModel(modelId)
                              setSttFeedback({ ok: true, msg: '云端识别模型已保存' })
                              setTimeout(() => setSttFeedback(null), 2000)
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
                        className={`text-caption${sttFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {sttFeedback.msg}
                      </div>
                    )}
                    {sttLocalStatus &&
                      (sttLocalStatus.cloud_configured ? (
                        <>
                          {!sttLocalStatus.available && (
                            <div className="text-caption hint-text models-warn">
                              <IconAlertTriangle size={12} className="icon-prefix" />
                              未检测到麦克风，连接麦克风后即可使用语音输入
                            </div>
                          )}
                          {!sttLocalStatus.model_dir && (
                            <div className="text-caption hint-text">
                              本地模型未下载（云端识别已可用，仅离线识别时需要）
                            </div>
                          )}
                        </>
                      ) : sttLocalStatus.reason === 'no_microphone' ? (
                        <div className="text-caption hint-text models-warn">
                          <IconAlertTriangle size={12} className="icon-prefix" />
                          未检测到麦克风，连接麦克风后即可使用语音输入
                        </div>
                      ) : sttLocalStatus.reason?.startsWith('model_missing') ? (
                        <div>
                          <div className="text-caption hint-text">
                            下载语音模型（约 250 MB）即可开始本地语音输入
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
                            <div className="detect-error">下载失败：{sttDl.error}</div>
                          )}
                          <Button
                            variant="primary"
                            size="sm"
                            style={{ marginTop: 8 }}
                            loading={sttDl.downloading}
                            onClick={sttDl.start}
                          >
                            {sttDl.error ? '重试下载' : '下载语音模型'}
                          </Button>
                        </div>
                      ) : null)}
                  </Section>

                  {/* ── 文字转语音（TTS） ── */}
                  <Section
                    title="文字转语音（TTS）"
                    description="配置文字转语音模型，用于 AI 回复的语音朗读，支持 OpenAI 兼容的 TTS 服务。"
                  >
                    <FormRow
                      stacked
                      label="TTS 模型"
                      hint="留空表示不使用朗读功能。"
                      control={
                        <VisionModelSelect
                          value={ttsModel}
                          models={allModels}
                          placeholder="未配置（不使用朗读）"
                          showVisionIcons={false}
                          menuUp
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
                    {ttsFeedback && (
                      <div
                        className={`text-caption${ttsFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {ttsFeedback.msg}
                      </div>
                    )}
                  </Section>

                  {/* ── 语音克隆 ── */}
                  <Section
                    title="语音克隆"
                    description="配置语音克隆模型（云端克隆 API），配置后语音克隆工具可用。"
                  >
                    <FormRow
                      stacked
                      label="语音克隆模型"
                      hint="留空表示不使用语音克隆。"
                      control={
                        <VisionModelSelect
                          value={voiceModel}
                          models={allModels}
                          placeholder="未配置（不使用）"
                          showVisionIcons={false}
                          menuUp
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
                        className={`text-caption${voiceFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {voiceFeedback.msg}
                      </div>
                    )}
                  </Section>
                </>
              )}

              {/* ═══════════ 子智能体模型：ExecAgent 子任务模型（原详情区 Exec 配置块迁移至此） ═══════════ */}
              {activeView === 'agents' && (
                <>
                  <Section
                    title="子任务执行模型（Exec）"
                    description="ExecAgent 由 Leader 模式下派发、执行子任务时使用的模型。留空则跟随全局默认模型。"
                  >
                    <FormRow
                      stacked
                      className="models-form-row--dedup"
                      label="Exec 模型"
                      hint="留空表示跟随全局默认模型。"
                      control={
                        <VisionModelSelect
                          value={agentModels.exec}
                          models={allModels}
                          onChange={m => void saveAgentModel('exec', m)}
                          t={t}
                          placeholder="跟随默认模型"
                        />
                      }
                    />
                    {agentFeedback && (
                      <div
                        className={`text-caption${agentFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {agentFeedback.msg}
                      </div>
                    )}
                  </Section>
                </>
              )}
            </div>
          </>
        )}

        {/* ── 全局反馈 Toast ── */}
        {(feedback ||
          visionFeedback ||
          sttFeedback ||
          ttsFeedback ||
          voiceFeedback ||
          agentFeedback) &&
          createPortal(
            <div
              className={`feedback-toast ${
                feedback?.ok ||
                visionFeedback?.ok ||
                sttFeedback?.ok ||
                ttsFeedback?.ok ||
                voiceFeedback?.ok ||
                agentFeedback?.ok
                  ? 'feedback-toast--ok'
                  : 'feedback-toast--error'
              }`}
            >
              {feedback?.msg ||
                visionFeedback?.msg ||
                sttFeedback?.msg ||
                ttsFeedback?.msg ||
                voiceFeedback?.msg ||
                agentFeedback?.msg ||
                ''}
            </div>,
            document.body,
          )}
      </div>
    </div>
  )
}
