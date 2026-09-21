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
  getProviderBaseUrl,
  addProviderModel,
  clearProviderModels,
  getAgentModels,
  getProviderContext,
  setAgentModel,
  setModelContextWindow,
  setModelSupportsVision,
  setVisionCapability,
  createCustomProvider,
  updateCustomProvider,
  oauthBegin,
  oauthStatus,
  oauthLogout,
  openExternal,
  sttStatus,
} from '../lib/api'
import type {
  ProviderInfo,
  ModelInfo,
  ProviderModelBrief,
  SttStatus,
  AgentModels,
  SyncReport,
  OauthConfigPayload,
  OauthStatusInfo,
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
  IconX,
  IconHardDrive,
} from '../../ui/Icons'
import { Section, FormRow } from '../../ui/PageLayout'
import { Button } from '../../ui/Button'
import { useLanguage } from '../../locales'
import { ProviderIcon, hasProviderIcon } from '../components/ProviderIcon'
import {
  buildCustomInstanceId,
  isCustomProviderId,
  LEGACY_CUSTOM_PROVIDER_ID,
} from '../lib/customProvider'
import { friendlyIpcError } from '../lib/ipcError'
import { selectableModels } from '../lib/modelCapability'
import { JevSettings } from './JevSettings'
import '../../styles/models.css'

// ════════════════════════════════════════════════════════════════
// 文案表（页面专用，硬编码中文；既有多语言字典仅保留仍在用 keys）
// 说明：本页历史上大量 t('models.*') 引用并不存在于字典 → 界面泄漏 key。
// 重构后统一为页内文案，可读中文；通用词条（provider/modelList/…）仍走字典。
// ════════════════════════════════════════════════════════════════
const TXT = {
  // ── 自定义模型四字段（名称 / 提供商 / API Key / API URL）──
  // 表单、实例配置页共用同一套字段名，全流程一个说法，用户不用做同义词翻译。
  nameLabel: '自定义名称',
  namePlaceholder: '例如：公司网关',
  providerTypeLabel: '模型提供商',
  customKeyLabel: '模型 API Key',
  customBaseUrlLabel: '模型 API URL',
  providerTypes: [
    { value: 'custom', label: 'OpenAI 兼容' },
    { value: 'anthropic', label: 'Anthropic 兼容' },
  ],
  keyPlaceholderCustom: '无鉴权端点可留空',
  /** 编辑模式留空 = 不改密钥（不是清空）：密钥已在后端，界面不回显 */
  keyPlaceholderKeep: '留空则保持原密钥不变',
  /** 由名称 slug 化来的段 id 只在 title 里作调试信息，界面不显示 */
  instanceIdTitle: (id: string) => `实例标识：${id}`,
  /** 左栏固定入口（custom 配置界面）显示名走 i18n：models.customEntry */
  /** 创建态/编辑态标题：同一套表单，只有按钮语义不同（「创建」/「保存」） */
  customSectionTitle: '自定义模型',
  createBtn: '创建',
  creating: '创建中…',
  /** 编辑页主按钮（同一套表单，只有按钮语义不同） */
  saveBtn: '保存',
  savingBtn: '保存中…',
  /** 「连接测试」：用当前填写的地址与密钥探测可用模型 */
  testBtn: '连接测试',
  createSuccess: '已创建',
  saveSuccess: '已保存',
  createFail: '创建失败',
  needName: '请填写自定义名称',
  needUrl: '请填写模型 API URL',
  /** Anthropic 协议没有标准模型列表端点：如实告知，并把入口指向唯一的可行路径 */
  anthropicNoModelList: 'Anthropic 协议不支持自动获取模型列表，请手动填写模型名',
  apiKeyLabel: 'API 密钥',
  keyInputPlaceholder: (name: string) => `输入 ${name} 的 API 密钥`,
  keyOverwritePlaceholder: '输入新密钥覆盖现有配置',
  keyShow: '显示',
  keyHide: '隐藏',
  connectBtn: '连接',
  connecting: '连接中…',
  connectTitle: '探测该接口下可用的模型',
  clearKeyTitle: '清除已保存的密钥',
  clearKeyConfirm:
    '确定要清除该服务商的 API 密钥吗？\n模型与其它配置会保留，清除后需重新输入才能使用云端服务。',
  keyHelp: '密钥仅存本机',
  keyHelpLocal:
    '本地服务通常无需密钥，可留空直接连接；若服务启用了鉴权（如 llama-swap、带 key 的网关/反向代理），在此填写后再连接。',
  keyPlaceholderLocal: '可选：本地服务启用鉴权时填写密钥',
  saveKeyBtn: '保存',
  savingKey: '保存中…',
  saveKeyTitle: '仅保存密钥（不检测模型）',
  saveKeyNoModel: '请先点「连接测试」获取模型',
  /** Anthropic 实例没有「连接」按钮，只能手动加模型名 */
  saveKeyNoModelManual: '请先手动添加模型名',
  saveKeySuccess: (m: string) => `密钥已保存（目标模型：${m}）`,
  /** 无鉴权实例（key 留空）保存：只说实际发生了什么（地址与默认模型落盘） */
  saveNoKeySuccess: (m: string) => `已保存（目标模型：${m}）`,
  saveKeyFail: '保存密钥失败',
  filterPlaceholder: '筛选模型…',
  baseUrlLabel: '接口地址',
  baseUrlPlaceholder: 'https://your-relay.com/v1',
  baseUrlHelp: '中转站或网关的接口地址，通常以 /v1 结尾',
  /** 自定义标头编辑区：中转站网关要求的附加请求头，逐字注入每个请求 */
  headerSectionLabel: '自定义标头',
  headerSectionHelp: '部分中转站要求附加请求头；留空则不发送',
  headerNamePlaceholder: '头名称，如 X-Gateway',
  headerValuePlaceholder: '值（可为空）',
  addHeaderBtn: '+ 添加标头',
  removeHeaderTitle: '删除此标头',
  // ── 订阅账号（OAuth，可选路径）──
  // 静态密钥与 OAuth 是同一实例的二选一凭证来源；区块默认折叠，避免给只用密钥的
  // 用户增加六项 OAuth 字段的视觉负担（折叠态只留一行状态摘要）。
  oauthSectionLabel: '订阅账号（OAuth）',
  oauthSummaryOff: '未配置',
  oauthSummaryNotLoggedIn: '已配置 · 未登录',
  oauthSummaryLoggedIn: '已登录',
  oauthSummaryNeedsLogin: '需重新授权',
  /** 编辑态：OAuth 五项随 ProviderInfo.oauth 摘要回显可改；三态语义见 oauthToPayload */
  oauthHelpEdit: '填写则覆盖更新；清空三项必填并保存 = 切回 API Key 模式（移除 OAuth 配置）。',
  oauthHelpCreate: '填写三项必填即为该实例启用 OAuth；创建完成后可在此授权登录。',
  /** 创建态：实例尚未落盘，后端 oauth_begin 找不到段 → 只能先说明可行路径 */
  oauthCreateHint: '实例创建完成后，回到本页即可用「授权登录」绑定订阅账号。',
  oauthAuthorizeLabel: '授权端点',
  oauthAuthorizePlaceholder: 'https://sso.example.com/authorize',
  oauthTokenLabel: '令牌端点',
  oauthTokenPlaceholder: 'https://sso.example.com/token',
  oauthClientIdLabel: 'Client ID',
  oauthClientIdPlaceholder: 'OAuth 客户端标识',
  oauthScopesLabel: 'Scopes',
  oauthScopesPlaceholder: '空格分隔，可留空',
  oauthScopesHelp: '空格分隔；留空则授权请求不带 scope',
  oauthPkceLabel: 'PKCE',
  oauthPkceHelp: '推荐开启（RFC 7636）；仅当授权服务器不支持时关闭',
  oauthPkceOn: '开启',
  oauthPkceOff: '关闭',
  oauthRedirectPortLabel: '回调端口',
  oauthRedirectPortPlaceholder: '留空由系统分配',
  oauthRedirectPortHelp: '本地回调监听端口（http://127.0.0.1:<端口>/callback）',
  oauthPortInvalid: '回调端口需为 1-65535 的整数',
  oauthNeedRequired: '请先填写授权端点 / 令牌端点 / Client ID（三项必填）',
  oauthLoginBtn: '授权登录',
  oauthReauthorizeBtn: '重新授权',
  oauthLogoutBtn: '退出登录',
  oauthLoggingIn: '发起中…',
  oauthLoggedIn: '已登录',
  oauthExpiresAt: (time: string) => `有效期至 ${time}`,
  oauthNotLoggedIn: '尚未登录：点「授权登录」在浏览器完成授权',
  oauthNeedsLogin: '登录已失效：需重新授权',
  oauthBrowserOpened: '已打开浏览器授权页，完成后本页自动更新登录状态',
  oauthLoginSuccess: '授权成功',
  oauthLogoutDone: '已退出登录',
  oauthBeginFail: '发起授权失败',
  oauthOpenFail: '打开浏览器失败',
  oauthLogoutFail: '退出登录失败',
  modelListTitle: '可用模型',
  currentModelOf: (name: string) => `（当前使用：${name}）`,
  refreshBtn: '刷新',
  refreshing: '刷新中…',
  refreshTitle: '用已保存密钥重新拉取最新模型列表',
  addModelBtn: '+ 手动添加',
  /** Anthropic 实例：这是唯一能拿到模型的入口，文案直接说清要填什么 */
  addModelBtnManual: '+ 手动添加模型名',
  addModelTitle: '手动添加模型代号',
  addModelPlaceholder: '输入模型代号',
  addModelConfirm: '添加',
  addModelCancel: '取消',
  addModelRequired: '请输入模型代号',
  addModelSuccess: (id: string) => `已添加模型「${id}」，可在下方点击切换`,
  addModelFail: '添加失败',
  baseUrlChangedWarn:
    '接口地址已变更：旧模型列表可能在新地址下不可用（模型代号不存在，或同名模型能力不同）。建议清理后重新拉取。',
  /** Anthropic 实例无法重新拉取：提示改成手动重加 */
  baseUrlChangedWarnManual:
    '接口地址已变更：旧模型名可能在新地址下不可用。建议清理后手动重新添加模型名。',
  clearModelsBtn: '清理旧模型',
  clearModelsConfirm:
    '确定清理该服务商的旧模型列表吗？\n仅清空模型条目（名称/地址/密钥保留），清理后请点「刷新」拉取新地址的模型。',
  /** Anthropic 实例没有「刷新」：清理后只能手动重加模型名 */
  clearModelsConfirmManual:
    '确定清理该实例的旧模型列表吗？\n仅清空模型条目（名称/地址/密钥保留），清理后请手动重新添加模型名。',
  clearModelsSuccess: (n: number) => `已清理 ${n} 个旧模型`,
  clearModelsNone: '没有可清理的旧模型',
  clearModelsFail: '清理失败',
  emptyFiltered: (q: string) => `没有匹配「${q}」的模型`,
  emptyNeedConnect: '尚未获取模型，点「连接测试」',
  emptyNeedManual: '尚未添加模型，点「+ 手动添加模型名」',
  modelsCount: (n: number) => `${n} 个模型`,
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
  visionNone: '跟随 Leader 模型（需支持图像理解）（推荐）',
  downloadReady: '已就绪',
  downloadPaused: '下载已暂停',
  /** 显式刷新后的同步摘要（新增 / 更新 / 移除，并列出被覆写与被移除的 id） */
  syncSummary: (r: SyncReport) =>
    `已与官方模型清单同步：新增 ${r.added} · 更新 ${r.updated} · 移除 ${r.removed}`,
  syncSummaryUpdated: (ids: string[]) => `能力已更新：${ids.join('、')}`,
  syncSummaryRemoved: (ids: string[]) => `移除：${ids.join('、')}`,
  syncSummaryKeptManual: (n: number) => `保留手动添加 ${n} 条`,
  syncSummaryDismiss: '关闭同步摘要',
  /** 手动添加条目（不在官方 /v1/models 清单内）的行内标注 */
  manualBadge: '官方清单外',
  manualBadgeTitle: '手动添加的模型：不在官方 /v1/models 清单内，刷新时不会被移除',
}

// ════════════════════════════════════════════════════════════════
// 左侧导航三组（按「谁提供模型」划分，而非按「技术上是不是 provider」）
//
// 模块一「模型提供商」：云端远程服务商 + Opencode GO 套餐网关（都是「别人提供的服务」）
// 模块二「自定义模型」：已落盘的自定义中转站实例（动态列表）+「+ 新建」入口
// 模块三「本地模型」：跑在自己机器上的服务（local）——不是云端提供商，单独成组，
//                     否则用户要在云厂商列表里找一个本机服务
// 「图像音频模型 / 子智能体模型」不再占左栏，改为页面右上角工具栏入口。
// ════════════════════════════════════════════════════════════════

/** 本地模型段名（与后端 `LocalProvider::id` 一致）。 */
const LOCAL_PROVIDER_ID = 'local'

/** 实例条目的显示名：用户填写的名称优先，老配置（无 display_name）回退段名 */
function instanceLabel(p: ProviderInfo): string {
  return (p.display_name || p.name || p.id).trim() || p.id
}

/** 表单标头行 → invoke 二元组数组（全空行已在表单 submit 时过滤，这里只做形态转换） */
function headersToTuples(headers: CustomHeaderRow[]): Array<[string, string]> {
  return headers.map(h => [h.name, h.value])
}

/**
 * 表单内的 OAuth 配置值（五项）。回调端口以**文本**持有 —— 输入框天然是字符串，
 * 空串 = 未指定（提交时转 null，由后端分配空闲端口）。
 */
export interface OauthFormValues {
  authorizeUrl: string
  tokenUrl: string
  clientId: string
  scopes: string
  usePkce: boolean
  /** 回调端口文本；空串 = 未指定 */
  redirectPort: string
}

/** OAuth 配置字段的空值（区块展开时字段全空；三项必填全空 = 该实例不启用 OAuth） */
const EMPTY_OAUTH_VALUES: OauthFormValues = {
  authorizeUrl: '',
  tokenUrl: '',
  clientId: '',
  scopes: '',
  usePkce: true,
  redirectPort: '',
}

/**
 * 后端 OAuth 摘要（snake_case）→ 表单值：编辑态回显已存配置。
 *
 * 摘要缺配置项（旧段只登录过、或后端未带）→ undefined，表单回落到空值：
 * 空值提交 = 不动已存配置，与「配置躺在磁盘里」的既有语义一致。
 */
function oauthSummaryToForm(s: OauthStatusInfo | null | undefined): OauthFormValues | undefined {
  if (!s) return undefined
  return {
    authorizeUrl: s.authorize_url ?? '',
    tokenUrl: s.token_url ?? '',
    clientId: s.client_id ?? '',
    scopes: s.scopes ?? '',
    usePkce: s.use_pkce ?? true,
    redirectPort: s.redirect_port == null ? '' : String(s.redirect_port),
  }
}

/**
 * 表单 OAuth 值 → 落盘 DTO（三态语义，与后端 `update_custom_provider_segment` 对齐）：
 *
 * - `null`/`undefined`（表单层） → `null`：**不启用 / 不动**已存 OAuth 配置；
 * - 三项必填全空的对象 → 全空 DTO：**清除** OAuth 配置（切回静态 API Key 模式）——
 *   段里留着 oauth 表会让凭证解析一直走 OAuth 分支，必须有这条路出去；
 * - 齐全 → 正常 DTO；部分填写由后端/表单给出「三项必填」可读错误。
 */
function oauthToPayload(v: OauthFormValues | null | undefined): OauthConfigPayload | null {
  if (!v) return null
  const port = v.redirectPort.trim()
  return {
    authorizeUrl: v.authorizeUrl.trim(),
    tokenUrl: v.tokenUrl.trim(),
    clientId: v.clientId.trim(),
    scopes: v.scopes.trim(),
    usePkce: v.usePkce,
    redirectPort: port === '' ? null : Number(port),
  }
}

/** 除三项必填外，区块内可选项是否被动过（Scopes / PKCE / 端口） */
function hasOauthAuxInput(v: OauthFormValues): boolean {
  return v.scopes.trim() !== '' || !v.usePkce || v.redirectPort.trim() !== ''
}

/** 回调端口合法性（后端按 u16 解析；非法值必须在表单拦下，而不是让后端报 JSON 解析错） */
function isValidRedirectPort(port: number): boolean {
  return Number.isInteger(port) && port >= 1 && port <= 65535
}

/** OAuth 到期时刻（unix 秒）→ 本地时间文本；无到期时间返回空串（只展示「已登录」） */
function oauthExpiryText(expiresAt: number | null | undefined): string {
  if (expiresAt == null) return ''
  return TXT.oauthExpiresAt(new Date(expiresAt * 1000).toLocaleString())
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
  provider,
  models,
  onChange,
  t,
  placeholder,
  showVisionIcons,
  filterCapability,
  menuUp = false,
}: {
  value: string
  provider?: string
  models: ModelInfo[]
  /** provider 与被选 model 同源（同 id 跨服务商时不可只用 id 消歧） */
  onChange: (id: string, provider: string) => void
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

  // 能力即过滤条件（见 lib/modelCapability.ts 的取舍说明）：视觉列表只列
  // supports_vision=true 的模型，能力由用户在模型行内显式声明。
  const filtered = selectableModels(models, filterCapability)
  // 触发器上的已保存值必须**脱离候选集**解析：若该模型的视觉开关当前是关的，
  // 它已不在候选集里，但用户实际配置就是它——显示成「未配置」会误导。
  const selected = Array.isArray(models)
    ? models.find(m => m.id === value && (!provider || m.provider === provider))
    : undefined
  const selectedMissing = !!value && !!selected && !filtered.includes(selected)
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
              onChange('', '')
              close()
            }}
          >
            <span className="select-option-name">{emptyText}</span>
          </div>
          {filtered.length === 0 && (
            <div className="compact-select-empty">
              {filterCapability === 'vision'
                ? '暂无支持视觉的模型：在左侧服务商的模型列表里，为可输入图片的模型打开「视觉输入」'
                : '暂无可选模型（先在上方连接并选择服务商）'}
            </div>
          )}
          {selectedMissing && (
            <div className="compact-select-empty" role="status">
              {`当前配置的 ${value} 未开启视觉能力，已不在候选列表中；如需继续使用，请先在模型列表中打开它的「视觉输入」。`}
            </div>
          )}
          {filtered.map(m => (
            <div
              key={`${m.provider}:${m.id}`}
              role="option"
              aria-selected={selected === m}
              className={`compact-select-option ${selected === m ? 'active' : ''}`}
              onClick={() => {
                onChange(m.id, m.provider)
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
// 自定义模型表单（新建 / 编辑共用同一套模板）
//
// 一个组件、一套字段顺序与一套校验：新建（左栏「+ 新建」）与编辑（选中某个自定义
// 模型）渲染出的就是同一份 JSX。历史上两处各写一遍 —— 新建页四个可编辑字段、
// 配置页把同两个字段又做成只读行 —— 于是同一个名字一屏出现两次、且都改不了；
// 改一处字段得同时改两处，迟早漂移。
//
// 段 id（custom-xxx）不出现在表单里：它是模型路由依据（同名模型靠「段 id + 模型 ID」
// 精确路由），重命名只改 display_name，由后端 update_custom_provider 保证。
// ════════════════════════════════════════════════════════════════

/** 一行自定义请求头（key/value 成对编辑；全空行在提交前被过滤） */
export interface CustomHeaderRow {
  name: string
  value: string
}

export interface CustomModelFormValues {
  displayName: string
  providerType: string
  baseUrl: string
  apiKey: string
  /** 段级自定义请求头（部分中转站网关要求），随创建/更新全量上报 */
  headers: CustomHeaderRow[]
  /**
   * OAuth 订阅配置（五项）。null = 该实例不启用 OAuth（三项必填全空）；
   * update 提交 null = 不动已存 OAuth 配置（后端契约）。
   */
  oauth: OauthFormValues | null
}

/** edit 模式的登录区数据源与动作（create 模式不传 —— 实例尚未落盘） */
export interface OauthLoginSection {
  /** 当前实例登录态（父组件经 oauth_status 拉取）；null = 尚未读到，按未登录展示 */
  status: OauthStatusInfo | null
  /** 发起授权 / 退出登录进行中：按钮 loading 且不可重复点击 */
  busy: boolean
  /** 最近一次交互结果（失败时是后端原文） */
  feedback: { ok: boolean; msg: string } | null
  onLogin: () => void
  onLogout: () => void
}

export function CustomModelForm({
  mode,
  initial,
  hasKey,
  saving,
  error,
  onSubmit,
  onValuesChange,
  onTest,
  testing,
  onClearKey,
  clearingKey,
  oauthLogin,
}: {
  mode: 'create' | 'edit'
  /** edit 模式：该实例当前已存值（create 模式不传，四个字段从空开始） */
  initial?: {
    displayName: string
    providerType: string
    baseUrl: string
    /** 已存标头回显（extra_headers 的数组形态）；缺省 = 空列表 */
    headers?: CustomHeaderRow[]
    /** 已存 OAuth 配置回显（ProviderInfo.oauth 摘要 → 表单值）；缺省 = 空配置 */
    oauth?: OauthFormValues
  }
  /** edit 模式：该实例是否已配置密钥（仅用于「留空 = 保持原密钥」的占位提示） */
  hasKey?: boolean
  saving: boolean
  /** 落盘失败文案（IPC 错误经 friendlyIpcError 转换后由调用方传入） */
  error: string | null
  onSubmit: (v: CustomModelFormValues) => void
  /**
   * 字段变化上报（可选）。调用方的「连接测试」「刷新」「地址已变更」提示要用到用户**当前**
   * 填入的地址与密钥；表单只做单向上报，不回灌，避免打字被打断。
   */
  onValuesChange?: (v: CustomModelFormValues) => void
  /** 「连接测试」：用当前填写的地址与密钥探测可用模型。不传则不渲染（Anthropic 协议无 /v1/models） */
  onTest?: () => void
  testing?: boolean
  /** 清除已保存的密钥（仅编辑态且已配置时传入）。不传则不渲染——清空输入框=保持原密钥，不是删除 */
  onClearKey?: () => void
  clearingKey?: boolean
  /**
   * 订阅账号登录区（仅编辑态传入）。传了才渲染「授权登录 / 退出登录」——
   * create 模式下实例尚未落盘，后端 oauth_begin 找不到配置段，只能先填配置。
   */
  oauthLogin?: OauthLoginSection
}) {
  const [displayName, setDisplayName] = useState(initial?.displayName ?? '')
  const [providerType, setProviderType] = useState(initial?.providerType || 'custom')
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? '')
  const [apiKey, setApiKey] = useState('')
  /** 自定义标头行（key/value 成对）；空列表 = 不写标头（后端缺键语义不变） */
  const [headers, setHeaders] = useState<CustomHeaderRow[]>(initial?.headers ?? [])
  /**
   * 订阅账号（OAuth）配置。编辑态从 ProviderInfo.oauth 摘要回显已存五项
   *（与标头同一约定：配置可见可改）；创建态为空。
   */
  const [oauth, setOauth] = useState<OauthFormValues>(initial?.oauth ?? EMPTY_OAUTH_VALUES)
  /** 订阅账号区块展开态：默认折叠（只用静态密钥的用户不该被六项 OAuth 字段挡视线） */
  const [oauthOpen, setOauthOpen] = useState(false)
  /** 用户手动开合过区块 → 不再由「已配 oauth / 需重新登录」自动展开（不抢用户的收放动作） */
  const [oauthTouched, setOauthTouched] = useState(false)
  const [showKey, setShowKey] = useState(false)
  /** 字段级校验（名称为空 / 地址为空）——与落盘错误分开显示，谁先发生谁先提示 */
  const [fieldError, setFieldError] = useState('')

  // 跟随「当前实例」的已存值：切换实例、或落盘后列表刷新时同步一次。
  // 依赖的是已存值（不是受控 value），所以打字过程不会被重置。
  // headers / oauth 依赖用序列化内容：父组件每次渲染重建对象，按引用比较会每个键击都重置表单。
  const initialHeadersKey = JSON.stringify(initial?.headers ?? [])
  const initialOauthKey = JSON.stringify(initial?.oauth ?? null)
  useEffect(() => {
    setDisplayName(initial?.displayName ?? '')
    setProviderType(initial?.providerType || 'custom')
    setBaseUrl(initial?.baseUrl ?? '')
    setApiKey('')
    setHeaders(initial?.headers ?? [])
    // OAuth 配置随实例回显（编辑态可见可改）；空值提交 = 不动后端已存 OAuth 配置。
    setOauth(initial?.oauth ?? EMPTY_OAUTH_VALUES)
    setOauthTouched(false)
    setShowKey(false)
    setFieldError('')
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 依赖已存值的形态而非 initial 对象引用
  }, [
    mode,
    initial?.displayName,
    initial?.providerType,
    initial?.baseUrl,
    initialHeadersKey,
    initialOauthKey,
  ])

  /**
   * 已配 oauth / 需重新登录 → 自动展开（登录态是异步读回来的，故用 effect 跟随）。
   * 用户手动开合过（oauthTouched）后不再自动展开 —— 收放动作由用户说了算。
   */
  const oauthNeedsAttention = !!oauthLogin?.status?.configured || !!oauthLogin?.status?.needs_login
  useEffect(() => {
    if (oauthNeedsAttention && !oauthTouched) setOauthOpen(true)
  }, [oauthNeedsAttention, oauthTouched])

  const emit = (patch: Partial<CustomModelFormValues>) => {
    onValuesChange?.({
      displayName: patch.displayName ?? displayName,
      providerType: patch.providerType ?? providerType,
      baseUrl: patch.baseUrl ?? baseUrl,
      apiKey: patch.apiKey ?? apiKey,
      headers: patch.headers ?? headers,
      oauth: patch.oauth !== undefined ? patch.oauth : oauth,
    })
  }

  /** OAuth 字段变更：整块替换（字段少、以完整值上报，避免漏字段） */
  const setOauthField = (patch: Partial<OauthFormValues>) => {
    const next = { ...oauth, ...patch }
    setOauth(next)
    emit({ oauth: next })
  }

  const submit = () => {
    const name = displayName.trim()
    if (!name) {
      setFieldError(TXT.needName)
      return
    }
    const url = baseUrl.trim()
    if (!url) {
      setFieldError(TXT.needUrl)
      return
    }
    // 「三项必填是否填了任意一项」= 用户是否真的想启用 OAuth（比对 DTO 判空更直接：
    // 全空对象的 DTO 也非 null，不能用来判断「不启用」）
    const oauthComplete = !!(
      oauth.authorizeUrl.trim() ||
      oauth.tokenUrl.trim() ||
      oauth.clientId.trim()
    )
    // 三项必填留空却动了可选项（Scopes / PKCE / 端口）= 半配置状态：当场说明并拦下，
    // 避免「填了却没生效」的静默丢弃；三项全空且可选项未动 = 该实例确实不启用 OAuth。
    if (!oauthComplete && hasOauthAuxInput(oauth)) {
      setFieldError(TXT.oauthNeedRequired)
      return
    }
    // 端口只在「确实要提交 OAuth 配置」时才校验：整块留空 = 不启用，填了端口也无意义
    if (oauthComplete) {
      const portText = oauth.redirectPort.trim()
      if (portText !== '' && !isValidRedirectPort(Number(portText))) {
        setFieldError(TXT.oauthPortInvalid)
        return
      }
    }
    // 清除语义：编辑态原本配了 OAuth、用户把三项必填清空 → 提交全空值，
    // 后端据此移除 oauth 表（切回静态密钥）。未配过 OAuth 的全空 = 不启用（传 null）。
    const hadOauthConfig =
      !!initial?.oauth &&
      !!(initial.oauth.authorizeUrl || initial.oauth.tokenUrl || initial.oauth.clientId)
    const clearingOauth = !oauthComplete && hadOauthConfig
    setFieldError('')
    onSubmit({
      displayName: name,
      providerType,
      baseUrl: url,
      apiKey: apiKey.trim(),
      // name 与 value 均为空的行是编辑残渣，不上报；其余原样（value 可为空串）
      headers: headers.filter(h => h.name.trim() !== '' || h.value.trim() !== ''),
      // 三态：填写 = 覆盖；原配过且清空 = 全空对象（清除）；其余全空 = null（不启用/不动）
      oauth: oauthComplete ? oauth : clearingOauth ? EMPTY_OAUTH_VALUES : null,
    })
  }

  /** 标头行变更：改字段 / 增行 / 删行都走这里，同步上报给「连接测试」等消费方 */
  const setHeaderRow = (idx: number, patch: Partial<CustomHeaderRow>) => {
    const next = headers.map((h, i) => (i === idx ? { ...h, ...patch } : h))
    setHeaders(next)
    emit({ headers: next })
  }
  const addHeaderRow = () => {
    const next = [...headers, { name: '', value: '' }]
    setHeaders(next)
    emit({ headers: next })
  }
  const removeHeaderRow = (idx: number) => {
    const next = headers.filter((_, i) => i !== idx)
    setHeaders(next)
    emit({ headers: next })
  }

  const shownError = fieldError || error

  const oauthStatus = oauthLogin?.status ?? null
  const oauthExpiry = oauthExpiryText(oauthStatus?.expires_at)
  /** 折叠态摘要：一眼看清该实例是否走 OAuth / 是否已登录（未配置时视觉负担最小） */
  const oauthSummary = oauthStatus?.logged_in
    ? TXT.oauthSummaryLoggedIn
    : oauthStatus?.needs_login
      ? TXT.oauthSummaryNeedsLogin
      : oauthStatus?.configured
        ? TXT.oauthSummaryNotLoggedIn
        : TXT.oauthSummaryOff

  return (
    <>
      <FormRow
        stacked
        label={TXT.nameLabel}
        control={
          <input
            className="compact-input"
            autoFocus={mode === 'create'}
            value={displayName}
            onChange={e => {
              setDisplayName(e.target.value)
              setFieldError('')
              emit({ displayName: e.target.value })
            }}
            placeholder={TXT.namePlaceholder}
            aria-label={TXT.nameLabel}
          />
        }
      />
      <FormRow
        stacked
        label={TXT.providerTypeLabel}
        control={
          <select
            className="compact-input"
            value={providerType}
            onChange={e => {
              setProviderType(e.target.value)
              emit({ providerType: e.target.value })
            }}
            aria-label={TXT.providerTypeLabel}
          >
            {TXT.providerTypes.map(o => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        }
      />
      <FormRow
        stacked
        label={TXT.customKeyLabel}
        control={
          /* 结构对齐官方服务商密钥行：.models-key-field 是 position:relative 的输入框容器
             （内含 100% 宽 input + 绝对定位的眼睛图标），.models-key-clear 是流式方块，
             必须与 field 平级放在 .models-key-row 里 —— 塞进 field 内部会把输入框挤到换行、
             眼睛图标也会被顶出输入框。 */
          <div className="models-key-row">
            <div className="models-key-field">
              <input
                className="compact-input"
                type={showKey ? 'text' : 'password'}
                value={apiKey}
                onChange={e => {
                  setApiKey(e.target.value)
                  setFieldError('')
                  emit({ apiKey: e.target.value })
                }}
                placeholder={
                  mode === 'edit' && hasKey ? TXT.keyPlaceholderKeep : TXT.keyPlaceholderCustom
                }
                aria-label={TXT.customKeyLabel}
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
            {/* 清空输入框 = 保持原密钥（不是删除）：删除已存密钥需要独立入口，放在密钥行右侧 */}
            {onClearKey && (
              <button
                type="button"
                className="models-key-clear"
                onClick={onClearKey}
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
      <FormRow
        stacked
        label={TXT.customBaseUrlLabel}
        control={
          <input
            className="compact-input"
            value={baseUrl}
            onChange={e => {
              setBaseUrl(e.target.value)
              setFieldError('')
              emit({ baseUrl: e.target.value })
            }}
            placeholder={TXT.baseUrlPlaceholder}
            aria-label={TXT.customBaseUrlLabel}
          />
        }
      />
      {/* ═══ 自定义标头：key/value 成对行，可增删（中转站网关要求的附加请求头） ═══ */}
      <FormRow
        stacked
        label={TXT.headerSectionLabel}
        control={
          <div className="custom-headers-editor">
            <div className="models-empty">{TXT.headerSectionHelp}</div>
            {headers.map((h, idx) => (
              <div className="custom-header-row" key={idx}>
                <input
                  className="compact-input"
                  value={h.name}
                  onChange={e => setHeaderRow(idx, { name: e.target.value })}
                  placeholder={TXT.headerNamePlaceholder}
                  aria-label={`${TXT.headerSectionLabel} ${idx + 1} 名称`}
                />
                <input
                  className="compact-input"
                  value={h.value}
                  onChange={e => setHeaderRow(idx, { value: e.target.value })}
                  placeholder={TXT.headerValuePlaceholder}
                  aria-label={`${TXT.headerSectionLabel} ${idx + 1} 值`}
                />
                <button
                  type="button"
                  className="models-key-clear"
                  onClick={() => removeHeaderRow(idx)}
                  title={TXT.removeHeaderTitle}
                  aria-label={`${TXT.removeHeaderTitle} ${idx + 1}`}
                >
                  ×
                </button>
              </div>
            ))}
            <div>
              <Button variant="default" size="sm" onClick={addHeaderRow}>
                {TXT.addHeaderBtn}
              </Button>
            </div>
          </div>
        }
      />
      {/* ═══ 订阅账号（OAuth）：可选路径，默认折叠 ═══
          折叠态只有一行状态摘要 —— 只配静态密钥的用户不该被六项 OAuth 字段挡住视线；
          已配 oauth / 需重新登录时自动展开（登录态异步到达，见 oauthNeedsAttention）。
          标题不用 FormRow：它不是单个字段，而是一组字段的开关（同「高级设置」式折叠）。 */}
      <div className="custom-oauth">
        <button
          type="button"
          className="custom-oauth-toggle"
          aria-expanded={oauthOpen}
          onClick={() => {
            setOauthTouched(true)
            setOauthOpen(v => !v)
          }}
        >
          <span className="custom-oauth-caret">{oauthOpen ? '▾' : '▸'}</span>
          <span className="custom-oauth-title">{TXT.oauthSectionLabel}</span>
          <span className="custom-oauth-summary">{oauthSummary}</span>
        </button>
        {oauthOpen && (
          <div className="custom-oauth-body">
            <div className="models-empty">
              {mode === 'create' ? TXT.oauthHelpCreate : TXT.oauthHelpEdit}
            </div>
            <FormRow
              stacked
              label={TXT.oauthAuthorizeLabel}
              control={
                <input
                  className="compact-input"
                  value={oauth.authorizeUrl}
                  onChange={e => setOauthField({ authorizeUrl: e.target.value })}
                  placeholder={TXT.oauthAuthorizePlaceholder}
                  aria-label={TXT.oauthAuthorizeLabel}
                />
              }
            />
            <FormRow
              stacked
              label={TXT.oauthTokenLabel}
              control={
                <input
                  className="compact-input"
                  value={oauth.tokenUrl}
                  onChange={e => setOauthField({ tokenUrl: e.target.value })}
                  placeholder={TXT.oauthTokenPlaceholder}
                  aria-label={TXT.oauthTokenLabel}
                />
              }
            />
            <FormRow
              stacked
              label={TXT.oauthClientIdLabel}
              control={
                <input
                  className="compact-input"
                  value={oauth.clientId}
                  onChange={e => setOauthField({ clientId: e.target.value })}
                  placeholder={TXT.oauthClientIdPlaceholder}
                  aria-label={TXT.oauthClientIdLabel}
                />
              }
            />
            <FormRow
              stacked
              label={TXT.oauthScopesLabel}
              hint={TXT.oauthScopesHelp}
              control={
                <input
                  className="compact-input"
                  value={oauth.scopes}
                  onChange={e => setOauthField({ scopes: e.target.value })}
                  placeholder={TXT.oauthScopesPlaceholder}
                  aria-label={TXT.oauthScopesLabel}
                />
              }
            />
            <FormRow
              stacked
              label={TXT.oauthPkceLabel}
              hint={TXT.oauthPkceHelp}
              control={
                <label className="custom-oauth-check">
                  <input
                    type="checkbox"
                    checked={oauth.usePkce}
                    onChange={e => setOauthField({ usePkce: e.target.checked })}
                    aria-label={TXT.oauthPkceLabel}
                  />
                  <span>{oauth.usePkce ? TXT.oauthPkceOn : TXT.oauthPkceOff}</span>
                </label>
              }
            />
            <FormRow
              stacked
              label={TXT.oauthRedirectPortLabel}
              hint={TXT.oauthRedirectPortHelp}
              control={
                <input
                  className="compact-input"
                  value={oauth.redirectPort}
                  onChange={e => setOauthField({ redirectPort: e.target.value })}
                  placeholder={TXT.oauthRedirectPortPlaceholder}
                  aria-label={TXT.oauthRedirectPortLabel}
                />
              }
            />
            {/* 登录区：只有编辑态才有实例可授权（create 态给一行可操作指引） */}
            {oauthLogin ? (
              <div className="custom-oauth-login">
                <div
                  className={
                    oauthStatus?.logged_in ? 'custom-oauth-status is-on' : 'custom-oauth-status'
                  }
                >
                  <span
                    className={
                      oauthStatus?.logged_in ? 'custom-oauth-dot is-on' : 'custom-oauth-dot'
                    }
                  />
                  <span className="custom-oauth-status-text">
                    {oauthStatus?.logged_in
                      ? `${TXT.oauthLoggedIn}${oauthExpiry ? ` · ${oauthExpiry}` : ''}`
                      : oauthStatus?.needs_login
                        ? TXT.oauthNeedsLogin
                        : TXT.oauthNotLoggedIn}
                  </span>
                </div>
                <div className="custom-oauth-actions">
                  <Button
                    variant="default"
                    size="sm"
                    onClick={oauthLogin.onLogin}
                    disabled={oauthLogin.busy}
                  >
                    {oauthLogin.busy
                      ? TXT.oauthLoggingIn
                      : oauthStatus?.logged_in
                        ? TXT.oauthReauthorizeBtn
                        : TXT.oauthLoginBtn}
                  </Button>
                  {/* 退出登录只对「有令牌可清」的实例有意义 */}
                  {oauthStatus?.logged_in && (
                    <Button
                      variant="default"
                      size="sm"
                      onClick={oauthLogin.onLogout}
                      disabled={oauthLogin.busy}
                    >
                      {TXT.oauthLogoutBtn}
                    </Button>
                  )}
                </div>
                {oauthLogin.feedback && (
                  <div
                    className={
                      oauthLogin.feedback.ok
                        ? 'custom-oauth-feedback is-ok'
                        : 'custom-oauth-feedback is-err'
                    }
                    role="status"
                  >
                    {oauthLogin.feedback.msg}
                  </div>
                )}
              </div>
            ) : (
              <div className="custom-oauth-hint">{TXT.oauthCreateHint}</div>
            )}
          </div>
        )}
      </div>
      {shownError && <div className="detect-error">{shownError}</div>}
      <div className="models-form-actions">
        <Button variant="primary" size="sm" onClick={submit} disabled={saving}>
          {saving
            ? mode === 'create'
              ? TXT.creating
              : TXT.savingBtn
            : mode === 'create'
              ? TXT.createBtn
              : TXT.saveBtn}
        </Button>
        {/* 连接测试与保存并排：两者都作用于「当前填写的地址与密钥」，
            放在同一个动作区，用户一眼看到两条出路（存下来 / 先测通） */}
        {onTest && (
          <Button variant="default" size="sm" onClick={onTest} disabled={testing}>
            {testing ? TXT.connecting : TXT.testBtn}
          </Button>
        )}
      </div>
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
  // ── 自定义模型四字段表单（新建 / 编辑共用 CustomModelForm，字段状态由表单自己持有）──
  // 只有这四个输入项，没有第五个；新建 = create_custom_provider，编辑 = update_custom_provider。
  const [formOpen, setFormOpen] = useState(false)
  const [formError, setFormError] = useState('')
  const [formSaving, setFormSaving] = useState(false)
  /** 编辑（保存）态的独立反馈：与新建互不影响 */
  const [editError, setEditError] = useState('')
  const [editSaving, setEditSaving] = useState(false)
  // ── 订阅账号（OAuth）登录区（仅编辑自定义实例时使用）──
  /** 当前实例登录态（oauth_status 权威读盘结果，不含令牌）；非自定义实例恒 null */
  const [oauthState, setOauthState] = useState<OauthStatusInfo | null>(null)
  /** 发起授权 / 退出登录进行中：按钮 loading，禁止重复点击 */
  const [oauthBusy, setOauthBusy] = useState(false)
  /** 最近一次登录交互结果（失败时是后端原文） */
  const [oauthFeedback, setOauthFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [apiKey, setApiKey] = useState('')
  const [showKey, setShowKey] = useState(false)
  const [inputVal, setInputVal] = useState('')
  const [models, setModels] = useState<string[]>([])
  const [baseUrl, setBaseUrl] = useState('')
  const [feedback, setFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [visionModel, setVisionModel] = useState('')
  const [visionProvider, setVisionProvider] = useState('')
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
    leader_provider: '',
    workflow: '',
    workflow_provider: '',
    exec: '',
    exec_provider: '',
    custom: '',
    custom_provider: '',
  })
  const [agentSaving, setAgentSaving] = useState(false)
  const [agentFeedback, setAgentFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  // ── 图像理解：默认「跟随 Leader」的可见性 ──
  // 未显式配置时图像由 Leader 模型直接理解（backend resolve_vision_strategy 同口径），
  // 这里把「跟随谁 / 它是否真能看图」如实呈现，用户不必猜"默认"指什么。
  const leaderVisionModelId = agentModels.leader || currentModel
  const leaderSupportsVision = !!allModels.find(
    m =>
      m.id === leaderVisionModelId &&
      (!agentModels.leader_provider || m.provider === agentModels.leader_provider),
  )?.supports_vision
  const [activeView, setActiveView] = useState<'provider' | 'capabilities' | 'agents' | 'jev'>(
    'provider',
  )
  const [hasKey, setHasKey] = useState(false)
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([])
  const [detecting, setDetecting] = useState(false)
  const [clearingKey, setClearingKey] = useState(false)
  const [savingKey, setSavingKey] = useState(false)
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
  /** 显式「刷新」后的落盘同步摘要（新增/更新/移除）；null = 不展示 */
  const [syncSummary, setSyncSummary] = useState<SyncReport | null>(null)

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
          setVisionProvider(m.vision_provider || '')
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
    // 回填该服务商已保存的接口地址：留空会让用户以为配置丢了，检测/刷新也会拿着
    // 空值去回落内置默认（自定义端点的内置默认只是文档占位示例）。
    getProviderBaseUrl(provider)
      .then(url => {
        if (providerRef.current !== provider) return
        const saved = (url || '').trim()
        if (!saved) return
        setBaseUrl(saved)
        setLoadedBaseUrl(saved)
      })
      .catch(() => {})
    setFilterInput('')
    setSyncSummary(null)
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
  // 后端 refresh_provider_models(sync=false) = 拉 /v1/models + 增量同步进
  // providers.toml 段（只新增 + 覆写能力，**绝不删除**条目）：使网关实际在售的
  // 新模型（如 opencode-go 的 deepseek 系列）无需手动点「刷新」即出现在本页列表
  // 与弹窗 hover（list_models 数据源）。删除仅由页内显式「刷新」触发。
  // 网络失败静默降级（保留 localStorage 检测缓存 + 磁盘段）；本地段跳过（无远程）。
  const autoSyncedRef = useRef<Set<string>>(new Set())
  const providerRef = useRef(provider)
  useEffect(() => {
    providerRef.current = provider
  }, [provider])
  useEffect(() => {
    if (activeView !== 'provider') return
    if (!provider || provider === 'local') return
    // 自定义中转站不自动拉取（与既有 custom 行为一致）：网关模型目录往往远大于
    // 实际可用集合，自动 upsert 会污染模型列表；用户在页内手动「刷新」即可。
    if (isCustomProviderId(provider)) return
    if (!configuredProviders.includes(provider)) return
    if (autoSyncedRef.current.has(provider)) return
    autoSyncedRef.current.add(provider)
    const target = provider
    // 不传内置默认地址：自建/中转端点须用 config.toml 已存真实地址，内置默认
    // （自定义端点为文档占位示例）由后端在缺省时自行回落。
    // sync=false：静默模式不删除任何条目，也不展示摘要。
    refreshProviderModels(target, undefined, false)
      .then(res => {
        if (providerRef.current !== target) return
        const models = res?.models
        if (!Array.isArray(models)) return
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

  // ── 订阅账号（OAuth）：登录态读取 + 授权结果事件 ──
  //
  // 登录态的唯一真值在磁盘（providers.toml 的 [providers.<段>.oauth]）：前端永远只
  // 拿得到状态，拿不到令牌。授权结果不由 oauth_begin 返回 —— 浏览器回调到达时后端
  // 推 `oauth-login-result` 事件，前端按 provider 匹配当前实例再刷新状态。

  /** 重读当前实例登录态（切实例 / 授权成功 / 退出登录后调用） */
  const reloadOauthStatus = useCallback(async (id: string) => {
    if (!isCustomProviderId(id)) {
      setOauthState(null)
      return
    }
    try {
      setOauthState(await oauthStatus(id))
    } catch {
      // 读盘失败（配置缺失等）不打断页面：按「未登录」展示，用户仍可点授权登录
      setOauthState(null)
    }
  }, [])

  useEffect(() => {
    setOauthFeedback(null)
    setOauthBusy(false)
    // 先清空再拉取：否则切换实例的瞬间会显示上一个实例的登录态（跨实例串状态会误导操作）
    setOauthState(null)
    void reloadOauthStatus(provider)
  }, [provider, reloadOauthStatus])

  // 授权结果事件：只认当前实例（同一事件通道上可能有别的实例在登录）。
  // listen 用动态 import：本模块被多个测试/页面引用，不在顶层引入 Tauri 事件 API。
  useEffect(() => {
    if (!isCustomProviderId(provider)) return
    const target = provider
    let unlisten: (() => void) | null = null
    let disposed = false
    void import('@tauri-apps/api/event')
      .then(({ listen }) =>
        listen<{ provider: string; ok: boolean; error: string | null }>('oauth-login-result', e => {
          const p = e.payload
          if (!p || p.provider !== target) return
          setOauthBusy(false)
          if (p.ok) {
            setOauthFeedback({ ok: true, msg: TXT.oauthLoginSuccess })
            void reloadOauthStatus(target)
          } else {
            setOauthFeedback({ ok: false, msg: p.error || TXT.oauthBeginFail })
          }
        }),
      )
      .then(un => {
        // 订阅是异步完成的：组件已卸载/已切实例则立即退订，避免监听泄漏
        if (disposed) un()
        else unlisten = un
      })
      .catch(() => {
        /* 事件 API 不可用（非 Tauri 环境）→ 交互仍可用，只是拿不到回调通知 */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [provider, reloadOauthStatus])

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
  /** 正在保存视觉能力开关的模型名（行内 loading 态，避免重复点击） */
  const [visionToggling, setVisionToggling] = useState('')
  const [editingCtxModel, setEditingCtxModel] = useState<string | null>(null)
  const [editingCtxValue, setEditingCtxValue] = useState('')
  const editingCtxRef = useRef<string | null>(null)

  const curProvider = providers.find(p => p.id === provider)
  const currentProviderInfo = curProvider
  const isCustom = isCustomProviderId(provider)
  const isLocal = provider === 'local'
  /** 该实例的协议类型：custom（OpenAI 兼容）/ anthropic（Anthropic 兼容） */
  const providerType = currentProviderInfo?.provider_type || ''
  /** 界面上该服务商/实例的显示名（自定义实例 = 用户填的名称；官方 = 内置展示名） */
  const providerLabel = curProvider ? instanceLabel(curProvider) : provider
  /** Anthropic 兼容实例没有 /v1/models：不展示「连接」，走手动填模型名 */
  const isAnthropicInstance = isCustom && providerType === 'anthropic'
  /**
   * 左栏「自定义模型」分组的具名实例列表 = custom-xxx 已落盘实例。
   * 旧版 `custom` 段**不在此列**：它是迁移前的老配置，界面上只以「Custom」配置
   * 入口的预填数据出现（创建实例时由后端自动接管迁移），不再单独占一个条目。
   */
  const customInstances = providers.filter(
    p => isCustomProviderId(p.id) && p.id !== LEGACY_CUSTOM_PROVIDER_ID,
  )
  /**
   * 旧版 `custom` 段（迁移源）：存在时作为创建态表单的预填数据（base_url /
   * provider_type——地址是用户最不想重打的字段；display_name 留空由用户填）。
   */
  const legacyCustom = providers.find(p => p.id === LEGACY_CUSTOM_PROVIDER_ID)
  /**
   * 左栏「模型提供商」= 云端远程服务商 + Opencode GO 套餐网关。
   * 本地模型（local）不在此列：它不是「云端提供商」，而是跑在自己机器上的服务。
   */
  const railProviders = providers.filter(
    p => !isCustomProviderId(p.id) && p.id !== LOCAL_PROVIDER_ID,
  )
  /** 左栏「本地模型」= 本机运行的服务（Ollama / llama.cpp / vLLM 等） */
  const localProviders = providers.filter(p => p.id === LOCAL_PROVIDER_ID)

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

  /**
   * 切换左侧导航选中的服务商 / 实例。
   *
   * `configuredOverride`：刚创建完实例时传入最新一次配置读取结果——`configuredProviders`
   * 状态更新是异步的，直接读会拿到旧值，把「刚填了密钥的实例」显示成未配置。
   */
  const handleProviderChange = (id: string, configuredOverride?: string[]) => {
    setProvider(id)
    loadProviderState(id)
    setInputVal('')
    setApiKey('')
    setFeedback(null)
    setDetectError(null)
    setRefreshError(null)
    setDetecting(false)
    setHasKey((configuredOverride ?? configuredProviders).includes(id))
  }

  /** 左侧导航选择某个服务商 / 实例：切到其配置页并收起「新建」表单 */
  const openProviderView = (id: string, configuredOverride?: string[]) => {
    setActiveView('provider')
    setFormOpen(false)
    setFormError('')
    handleProviderChange(id, configuredOverride)
  }

  /** 「Custom」配置入口：展开创建态表单（旧 custom 段存在时预填其地址与协议） */
  const openCreateForm = () => {
    setActiveView('provider')
    setFormOpen(true)
    setFormError('')
  }

  /**
   * 新建保存（CustomModelForm mode="create" 的提交口）→ 落盘 → 立刻进入该实例。
   *
   * 段 id 由「自定义名称」slug 化而来（纯中文名退化为 custom-<时间戳>）：
   * 界面上永远只显示用户填的名称，段 id 只出现在 title 里作调试信息。
   * 字段级校验（名称/地址必填）由表单统一负责，这里只管落盘；
   * 落盘失败（重名 / 非法地址 / 写盘错误）展示后端原文，不掩盖。
   */
  const createInstance = async (v: CustomModelFormValues) => {
    setFormSaving(true)
    setFormError('')
    try {
      const id = buildCustomInstanceId(
        v.displayName,
        providers.map(p => p.id),
      )
      const created = await createCustomProvider(
        id,
        v.displayName,
        v.providerType,
        v.baseUrl,
        v.apiKey,
        headersToTuples(v.headers),
        // OAuth 配置（可选）：三项必填全空 = null = 该实例走静态密钥
        oauthToPayload(v.oauth),
      )
      // 落盘是唯一真值：重新拉列表（新实例立即出现在左栏）与配置状态
      const [list, cfg] = await Promise.all([
        getSupportedProviders().catch(() => null),
        getCurrentConfig().catch(() => null),
      ])
      if (Array.isArray(list)) setProviders(sortProvidersStable(list))
      const configured = cfg?.configured_providers
      if (configured) setConfiguredProviders(configured)
      // 表单字段无需手工清空：进入实例后表单卸载（formOpen=false），下次展开是全新一份
      setFeedback({ ok: true, msg: TXT.createSuccess })
      setTimeout(() => setFeedback(null), 2500)
      openProviderView(created?.id || id, configured)
    } catch (e: any) {
      setFormError(friendlyIpcError(e, TXT.createFail))
    } finally {
      setFormSaving(false)
    }
  }

  /**
   * 编辑保存（CustomModelForm mode="edit" 的提交口）：重命名 / 换协议 / 改地址 / 换密钥。
   *
   * 段 id（custom-xxx）是模型路由依据，**不随重命名变化**：后端只改 `display_name`，
   * 因此这里传的是当前段 id（`provider`），不是用户填的名称。
   * API Key 留空 = 保持原密钥（后端契约：空串不覆盖），界面不回显原密钥。
   * 落盘成功后重拉服务商列表 —— 左栏立刻显示新名称（重启后从 providers.toml 读回同一值）。
   */
  const saveCustomInstance = async (v: CustomModelFormValues) => {
    setEditSaving(true)
    setEditError('')
    try {
      await updateCustomProvider(
        provider,
        v.displayName,
        v.providerType,
        v.baseUrl,
        v.apiKey,
        headersToTuples(v.headers),
        // OAuth 配置：null = 不动已存配置（编辑表单不回显五项，留空即「不修改」）；
        // 五项填全 = 覆盖写配置，既有令牌由后端保留（清令牌只能走「退出登录」）
        oauthToPayload(v.oauth),
      )
      const [list, cfg, savedUrl] = await Promise.all([
        getSupportedProviders().catch(() => null),
        getCurrentConfig().catch(() => null),
        getProviderBaseUrl(provider).catch(() => null),
      ])
      if (Array.isArray(list)) setProviders(sortProvidersStable(list))
      if (cfg?.configured_providers) {
        setConfiguredProviders(cfg.configured_providers)
        setHasKey(cfg.configured_providers.includes(provider))
      }
      // 回填已存地址：与「保存密钥」同一约定，地址变更提示随之归零
      const savedBase = (savedUrl || '').trim()
      if (savedBase) {
        setBaseUrl(savedBase)
        setLoadedBaseUrl(savedBase)
      }
      // 本次保存可能刚写入 / 覆盖了 OAuth 配置五项 → 登录区状态与摘要立即对齐磁盘
      // （不刷新的话摘要会一直停在保存前的「未配置」，直到用户切换实例）
      void reloadOauthStatus(provider)
      setFeedback({ ok: true, msg: TXT.saveSuccess })
    } catch (e: any) {
      setEditError(friendlyIpcError(e, TXT.saveFail))
    } finally {
      setEditSaving(false)
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  /**
   * 「授权登录」：后端起本地回调 server → 拿到授权 URL → 系统浏览器打开。
   *
   * 授权结果**不由这里返回**：浏览器回调到达后由后端推 `oauth-login-result`
   * 事件（见上方监听），成功再刷新登录态。这里只管「发起 + 打开 + 如实反馈」：
   * 段未配 oauth / 配置不完整时后端 reject 可读中文错误，原样展示不掩盖。
   * 重复点击是安全的（后端顶替旧会话），但请求期间仍锁按钮避免连点炸出一堆会话。
   */
  const startOauthLogin = async () => {
    if (oauthBusy) return
    setOauthBusy(true)
    setOauthFeedback(null)
    try {
      // invoke 在通道异常时回 null（命令本身没报错也会拿到 null）→ 按失败处理，
      // 不能拿一个空地址去开浏览器
      const authorizeUrl = (await oauthBegin(provider))?.authorize_url ?? ''
      if (!authorizeUrl) {
        setOauthFeedback({ ok: false, msg: TXT.oauthBeginFail })
        return
      }
      try {
        await openExternal(authorizeUrl)
      } catch (e) {
        setOauthFeedback({ ok: false, msg: friendlyIpcError(e, TXT.oauthOpenFail) })
        return
      }
      setOauthFeedback({ ok: true, msg: TXT.oauthBrowserOpened })
    } catch (e) {
      setOauthFeedback({ ok: false, msg: friendlyIpcError(e, TXT.oauthBeginFail) })
    } finally {
      setOauthBusy(false)
    }
  }

  /** 「退出登录」：清空令牌（配置五项保留）→ 重读状态（界面立刻回到未登录） */
  const logoutOauth = async () => {
    if (oauthBusy) return
    setOauthBusy(true)
    setOauthFeedback(null)
    try {
      await oauthLogout(provider)
      await reloadOauthStatus(provider)
      setOauthFeedback({ ok: true, msg: TXT.oauthLogoutDone })
    } catch (e) {
      setOauthFeedback({ ok: false, msg: friendlyIpcError(e, TXT.oauthLogoutFail) })
    } finally {
      setOauthBusy(false)
    }
  }

  /**
   * 「连接」：通过 /v1/models 列出可用模型。
   *
   * 判据与后端 fetch_provider_models 对齐 —— 只对官方远程服务商要求 key；
   * 本地服务（默认无鉴权）与自定义中转站（地址用户自己填，可能本就无鉴权，
   * 如 Ollama / llama-swap / 无 key 中转）允许空 key 直连探测。
   */
  const detectModels = async () => {
    // 空 key 直接交给后端：fetch_provider_models 空 key 时回落该段已存密钥
    // （编辑表单不回显 key，改地址/标��后点「连接」不该被要求重贴密钥）；
    // 段里也无 key 且该端点要求鉴权时，由后端返回「API Key 不能为空」。
    setDetecting(true)
    setDetectError(null)
    setDetectedModels([])
    try {
      // 只下发用户填写的地址；空值交给后端按「显式参数 → 已存配置 → 内置默认」解析，
      // 避免内置默认（自定义端点为文档占位示例）被当作有效地址。
      const models = await listProviderModels(apiKey.trim(), provider, baseUrl.trim() || undefined)
      setDetectedModels(models ?? [])
      saveDetectedModels(provider, models ?? [])
    } catch (e: any) {
      setDetectError(friendlyIpcError(e, '检测失败'))
    } finally {
      setDetecting(false)
    }
  }

  /**
   * 显式「刷新」：复用 config.toml 已存 key（不暴露 key 本身），把该服务商段
   * **同步为官方 /v1/models 集合**——移除官方清单外的 auto 条目、覆写能力元数据，
   * 并展示同步摘要（含被移除的 id，供用户知情与重加）。
   */
  const refreshModels = async () => {
    if (refreshing) return
    setRefreshing(true)
    setRefreshError(null)
    try {
      // sync=true 显式刷新；同上不下发内置默认地址，空值交给后端解析已存配置
      const res = await refreshProviderModels(provider, baseUrl.trim() || undefined, true)
      const models = res?.models
      setDetectedModels(models ?? [])
      saveDetectedModels(provider, models ?? [])
      setSyncSummary(res?.report ?? null)
      listModels()
        .then(list => {
          if (Array.isArray(list)) setAllModels(list)
        })
        .catch(() => {})
    } catch (e: any) {
      setRefreshError(friendlyIpcError(e, '刷新失败，请检查密钥与网络'))
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
      setAddError(friendlyIpcError(e, TXT.addModelFail))
    } finally {
      setAddSaving(false)
    }
  }

  /** 清理该服务商旧模型列表（接口地址变更后调用；清空 config.toml models + 本地检测缓存） */
  const handleClearModels = async () => {
    if (
      !window.confirm(isAnthropicInstance ? TXT.clearModelsConfirmManual : TXT.clearModelsConfirm)
    ) {
      return
    }
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
      // 清理后自动尝试拉取新地址的模型列表（key 有效时一步到位，失败由刷新区提示）。
      // Anthropic 协议没有 /v1/models：不发起必然失败的拉取（该实例也不展示「刷新」）。
      if (!isAnthropicInstance) refreshModels()
    } catch (e: any) {
      setFeedback({ ok: false, msg: friendlyIpcError(e, TXT.clearModelsFail) })
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
      const [cfg, savedUrl] = await Promise.all([
        getCurrentConfig().catch(() => null),
        getProviderBaseUrl(provider).catch(() => null),
      ])
      const savedBase = (savedUrl || '').trim()
      if (savedBase) {
        setBaseUrl(savedBase)
        setLoadedBaseUrl(savedBase)
      }
      setApiKey('')
      if (cfg?.configured_providers) {
        setConfiguredProviders(cfg.configured_providers)
        setHasKey(cfg.configured_providers.includes(provider))
      }
      setFeedback({ ok: true, msg: TXT.clearKeySuccess })
    } catch (e: any) {
      setFeedback({ ok: false, msg: friendlyIpcError(e, TXT.clearKeyFail) })
    } finally {
      setClearingKey(false)
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  /**
   * 仅保存密钥：不依赖「连接」是否成功，也不依赖可用模型列表里已有目标模型。
   *
   * 背景：key 原本只在「点击可用模型」那一步经 configureLlm 落盘（或走「添加本地模型」），
   * 于是服务端鉴权失败、可用模型列表为空时，用户「填了 key 却找不到保存入口」。
   * 这里补一个显式入口：以该服务商当前模型（否则已配置的首个模型）作为落盘目标。
   */
  const saveKey = async () => {
    const key = apiKey.trim()
    // 自定义中转站允许空 key 保存（无鉴权端点：地址与密钥都由用户自己填）
    if (!key && !isCustom) {
      setDetectError(TXT.apiKeyRequired)
      return
    }
    const own = allModels.filter(m => m.provider === provider).map(m => m.id)
    const target = (own.includes(currentModel) ? currentModel : '') || own[0] || ''
    if (!target) {
      // 提示必须指向本页真实存在的入口：Anthropic 实例没有「连接」按钮
      setDetectError(isAnthropicInstance ? TXT.saveKeyNoModelManual : TXT.saveKeyNoModel)
      return
    }
    setSavingKey(true)
    setDetectError(null)
    try {
      // 只落盘用户填写的地址；空值由后端解析已存配置/内置默认，
      // 避免把自定义端点的文档占位地址写进配置覆盖真实地址。
      await configureLlm(key, target, provider, baseUrl.trim() || undefined)
      // 回填「该服务商」的已存地址（cfg.base_url 是当前生效 provider 的地址，
      // 在保存其它 provider 时会把无关地址显示到输入框）。
      const [cfg, savedUrl] = await Promise.all([
        getCurrentConfig().catch(() => null),
        getProviderBaseUrl(provider).catch(() => null),
      ])
      const savedBase = (savedUrl || '').trim()
      if (savedBase) {
        setBaseUrl(savedBase)
        setLoadedBaseUrl(savedBase)
      }
      if (cfg?.configured_providers) {
        setConfiguredProviders(cfg.configured_providers)
        setHasKey(cfg.configured_providers.includes(provider))
      }
      setApiKey('')
      // key 留空 = 无鉴权实例：别说「密钥已保存」（什么都没存）
      setFeedback({
        ok: true,
        msg: key ? TXT.saveKeySuccess(target) : TXT.saveNoKeySuccess(target),
      })
      onModelChanged?.()
    } catch (e: any) {
      setDetectError(friendlyIpcError(e, TXT.saveKeyFail))
    } finally {
      setSavingKey(false)
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
      // 只下发用户填写的地址；内置默认（自定义端点为文档占位示例）不可当作覆盖
      const resolvedBaseUrl = baseUrl.trim() || undefined
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
        setFeedback({ ok: false, msg: friendlyIpcError(e, '切换失败') })
      }
      setTimeout(() => setFeedback(null), 2500)
    }
  }

  // Agent 级模型保存（高级设置）：空串 = 清除（跟随默认模型）
  // provider 与 model 同源传入（后端成对落盘；只给 model 时多候选会报错拒绝）
  const saveAgentModel = async (agent: string, model: string, provider?: string) => {
    setAgentSaving(true)
    setAgentFeedback(null)
    try {
      await setAgentModel(agent, model, provider)
      setAgentModels(prev => ({
        ...prev,
        [agent]: model,
        [`${agent}_provider`]: provider ?? '',
      }))
      setAgentFeedback({ ok: true, msg: `${agent} 模型已保存${model ? '' : '（跟随默认模型）'}` })
    } catch (e: any) {
      setAgentFeedback({ ok: false, msg: friendlyIpcError(e, '保存失败') })
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

  /** 模型当前生效的视觉能力（list_models 已按 provider+id 同源解析） */
  const rowVision = (name: string): boolean => {
    const brief = detectedModels.find(d => d.id === name)
    if (brief) return brief.supports_vision
    return allModels.find(m => m.provider === provider && m.id === name)?.supports_vision ?? false
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
      setFeedback({ ok: false, msg: friendlyIpcError(e, '保存失败') })
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  /**
   * 行内切换模型的视觉（多模态）能力 —— 与「上下文窗口」同构的第二类模型元数据编辑。
   *
   * 这个开关是图像理解模型列表的来源：只有打开它的模型才会出现在视觉模型候选里。
   * 落盘时会标记来源为 user，此后自动探测不再覆盖（否则今天开、下次连接又被关掉）。
   */
  const toggleModelVision = async (name: string, next: boolean) => {
    setVisionToggling(name)
    try {
      await setModelSupportsVision(provider, name, next)
      setFeedback({
        ok: true,
        msg: next ? `${name} 已标记为支持视觉输入` : `${name} 已标记为不支持视觉输入`,
      })
      const list = await listModels().catch(() => null)
      if (Array.isArray(list)) setAllModels(list)
    } catch (e: any) {
      setFeedback({ ok: false, msg: friendlyIpcError(e, '保存失败') })
    } finally {
      setVisionToggling('')
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
    // 不下发内置默认地址（自定义端点为文档占位示例），空值交给后端解析已存配置
    const resolvedBaseUrl = baseUrl.trim() || undefined
    const ctxArg =
      isLocal && localCtxWindow != null && !hasExplicitCtx(name) ? localCtxWindow : undefined
    try {
      await switchModelCmd(name, provider, resolvedBaseUrl, ctxArg, 'default')
      setCurrentModel(name)
      persistCurrentProvider(name)
      onModelChanged?.()
      setFeedback({ ok: true, msg: `已切换到 ${name}` })
    } catch (e: any) {
      setFeedback({ ok: false, msg: friendlyIpcError(e, '切换失败') })
    }
    setTimeout(() => setFeedback(null), 2500)
  }

  // ── 页面渲染 ──
  const loadingView = providersLoading ? (
    <div className="models-page-loading">正在加载服务商列表…</div>
  ) : null

  return (
    <div className="models-page-layout">
      {/* ── 左侧：两模块分组导航（模型提供商 / 自定义模型） ── */}
      <aside className="models-rail">
        <div className="models-rail-scroll">
          <div className="models-rail-group">
            <div className="models-rail-group-title">模型提供商</div>
            {providersLoading ? (
              <div className="models-rail-note">正在加载服务商…</div>
            ) : (
              <div className="models-rail-list">
                {railProviders.map(p => {
                  const isActive = activeView === 'provider' && p.id === provider
                  return (
                    <button
                      type="button"
                      key={p.id}
                      className={['models-rail-item', isActive ? 'active' : '']
                        .filter(Boolean)
                        .join(' ')}
                      onClick={() => openProviderView(p.id)}
                      title={p.name}
                    >
                      {hasProviderIcon(p.id) ? (
                        <ProviderIcon provider={p.id} size={16} />
                      ) : (
                        /* 无图标 provider（local 等）保留等宽占位，保证各行图标位对齐 */
                        <span className="provider-icon" style={{ width: 16, height: 16 }} />
                      )}
                      <span className="models-rail-name">{p.name}</span>
                    </button>
                  )
                })}
              </div>
            )}
          </div>

          {/* 模块二「自定义模型」：固定 Custom 配置入口（创建态表单）+ 已落盘具名实例。
              旧版 `custom` 段不单独占条目：创建实例时后端自动接管迁移，其配置作为
              创建态预填数据出现。条目只显示用户填写的名称（段 id 只作 title 调试信息）。 */}
          <div className="models-rail-group">
            <div className="models-rail-group-title">自定义模型</div>
            <div className="models-rail-list">
              <button
                type="button"
                className={[
                  'models-rail-item',
                  activeView === 'provider' && formOpen ? 'active' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                onClick={openCreateForm}
                title={t('models.customEntryTitle')}
                aria-expanded={formOpen}
              >
                <span className={`models-rail-dot${formOpen ? ' is-on' : ''}`} aria-hidden="true" />
                <span className="models-rail-name">{t('models.customEntry')}</span>
              </button>
              {customInstances.map(p => {
                const isActive = activeView === 'provider' && !formOpen && p.id === provider
                const isConfigured = configuredProviders.includes(p.id)
                return (
                  <button
                    type="button"
                    key={p.id}
                    className={['models-rail-item', isActive ? 'active' : '']
                      .filter(Boolean)
                      .join(' ')}
                    onClick={() => openProviderView(p.id)}
                    title={TXT.instanceIdTitle(p.id)}
                  >
                    <span
                      className={`models-rail-dot${isConfigured ? ' is-on' : ''}`}
                      title={isConfigured ? '已配置密钥' : '未配置密钥（无鉴权端点可留空）'}
                      aria-hidden="true"
                    />
                    <span className="models-rail-name">{instanceLabel(p)}</span>
                  </button>
                )
              })}
            </div>
          </div>

          {/* 模块三「本地模型」：跑在自己机器上的服务，与云端提供商分开放，
              避免用户被引导到云厂商列表里找一个本机服务。 */}
          {!providersLoading && localProviders.length > 0 && (
            <div className="models-rail-group">
              <div className="models-rail-group-title">本地模型</div>
              <div className="models-rail-list">
                {localProviders.map(p => {
                  const isActive = activeView === 'provider' && p.id === provider
                  return (
                    <button
                      type="button"
                      key={p.id}
                      className={['models-rail-item', isActive ? 'active' : '']
                        .filter(Boolean)
                        .join(' ')}
                      onClick={() => openProviderView(p.id)}
                      title="本机运行的服务（Ollama / llama.cpp / vLLM 等），默认地址 http://localhost:11434/v1"
                    >
                      {/* 本地服务图标：与「模型提供商」组的品牌图标同宽同位，
                          不再留一块空白（曾表现为与其它分组不一致的缺口）。
                          用「硬盘」表达「跑在自己机器上」，避免与云端品牌图标混淆。 */}
                      <span
                        className="provider-icon"
                        style={{ width: 16, height: 16 }}
                        aria-hidden="true"
                      >
                        <IconHardDrive size={16} />
                      </span>
                      <span className="models-rail-name">本地模型</span>
                    </button>
                  )
                })}
              </div>
            </div>
          )}

          <div className="models-rail-group">
            <div className="models-rail-group-title">自动化增强</div>
            <div className="models-rail-list">
              <button
                type="button"
                className={[
                  'models-rail-item',
                  'models-rail-item--sub',
                  activeView === 'jev' ? 'active' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                onClick={() => setActiveView('jev')}
              >
                <span className="models-rail-name">Jev 增强判断</span>
              </button>
            </div>
          </div>
        </div>
      </aside>

      {/* ── 右侧：按左侧所选显示对应内容页 ── */}
      <div className="models-main">
        {/* 右上角工具栏：图像/语音模型与子智能体模型（不占左栏） */}
        <div className="models-main-toolbar">
          <button
            type="button"
            className={['models-toolbar-btn', activeView === 'capabilities' ? 'is-active' : '']
              .filter(Boolean)
              .join(' ')}
            onClick={() => setActiveView('capabilities')}
          >
            图像音频模型
          </button>
          <button
            type="button"
            className={['models-toolbar-btn', activeView === 'agents' ? 'is-active' : '']
              .filter(Boolean)
              .join(' ')}
            onClick={() => setActiveView('agents')}
          >
            子智能体模型
          </button>
        </div>
        {loadingView}
        {!providersLoading && (
          <>
            <div className="models-main-scroll">
              {/* ═══════════ 自定义模型配置（Custom 入口）：创建态表单，落盘即新实例 ═══════════ */}
              {activeView === 'provider' && formOpen && (
                <Section title={TXT.customSectionTitle}>
                  <CustomModelForm
                    mode="create"
                    /* 旧 `custom` 段存在时预填其地址与协议（display_name 留空由用户填）：
                        迁移由后端在创建时自动完成，前端只把用户最不想重打的字段带上 */
                    initial={{
                      displayName: '',
                      providerType: legacyCustom?.provider_type || 'custom',
                      baseUrl: legacyCustom?.base_url || '',
                    }}
                    saving={formSaving}
                    error={formError}
                    onSubmit={createInstance}
                  />
                </Section>
              )}

              {/* ═══════════ 服务商配置 + 可用模型（官方服务商 / 自定义实例 / 本地模型共用） ═══════════ */}
              {activeView === 'provider' && !formOpen && (
                <>
                  {/* 自定义实例：名称 / 协议 / 密钥 / 地址由同一套表单编辑（CustomModelForm）；
                      官方与本地服务商保持原有只读展示与密钥栏 */}
                  <Section title={isCustom ? TXT.customSectionTitle : '模型服务商'}>
                    {/* 自定义模型：这一整行状态条不存在——名称/协议由下方表单承担（可编辑），
                        密钥状态由密钥字段自身的占位提示表达，动作（连接测试 / 清除密钥）也各自
                        归位到表单里。官方与本地服务商保留原有的「当前服务商」标题条。 */}
                    {!isCustom && (
                      <div className="models-provider-current">
                        {hasProviderIcon(provider) && (
                          <ProviderIcon provider={provider} size={18} />
                        )}
                        <span className="models-provider-current-name">{providerLabel}</span>
                        <span className="models-provider-current-hint">
                          {hasKey ? '已配置密钥' : '尚未配置密钥'}
                        </span>
                      </div>
                    )}

                    {isCustom ? (
                      <CustomModelForm
                        key={provider}
                        mode="edit"
                        initial={{
                          displayName: providerLabel,
                          providerType: providerType || 'custom',
                          baseUrl: curProvider?.base_url || '',
                          // 已存标头回显：对象 → 行数组（插入序 = 键名字典序，与落盘一致）
                          headers: Object.entries(curProvider?.extra_headers ?? {}).map(
                            ([name, value]) => ({ name, value }),
                          ),
                          // 已存 OAuth 配置回显（摘要带配置五项；无 oauth 段 → undefined）
                          oauth: oauthSummaryToForm(curProvider?.oauth),
                        }}
                        hasKey={hasKey}
                        saving={editSaving}
                        error={editError}
                        onSubmit={saveCustomInstance}
                        onValuesChange={v => {
                          setApiKey(v.apiKey)
                          setBaseUrl(v.baseUrl)
                        }}
                        /* Anthropic 协议没有 /v1/models：不提供「连接测试」（不给必然失败的入口） */
                        onTest={isAnthropicInstance ? undefined : detectModels}
                        testing={detecting}
                        onClearKey={hasKey ? handleClearKey : undefined}
                        clearingKey={clearingKey}
                        /* 订阅账号登录区：登录态 / 登录 / 退出都由本页持有（表单只管展示与转发） */
                        oauthLogin={{
                          status: oauthState,
                          busy: oauthBusy,
                          feedback: oauthFeedback,
                          onLogin: () => void startOauthLogin(),
                          onLogout: () => void logoutOauth(),
                        }}
                      />
                    ) : (
                      <>
                        {/* 密钥栏：官方远程服务商必填；本地服务可选（llama-swap 等启用鉴权时需要）。 */}
                        <FormRow
                          stacked
                          label={
                            <span className="models-field-label">
                              <IconPlug size={12} className="icon-prefix" />
                              {TXT.apiKeyLabel}
                            </span>
                          }
                          hint={isLocal ? TXT.keyHelpLocal : TXT.keyHelp}
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
                                      : isLocal
                                        ? TXT.keyPlaceholderLocal
                                        : TXT.keyInputPlaceholder(providerLabel)
                                  }
                                  aria-label={TXT.apiKeyLabel}
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
                              {/* Anthropic 协议没有 /v1/models：不摆一个必然失败的按钮 */}
                              {!isAnthropicInstance && (
                                <Button
                                  variant="primary"
                                  size="sm"
                                  onClick={detectModels}
                                  disabled={detecting || (!isLocal && !apiKey.trim())}
                                  title={TXT.connectTitle}
                                >
                                  {detecting ? TXT.connecting : TXT.connectBtn}
                                </Button>
                              )}
                              {/* 显式保存入口：key 原本只在「点击可用模型」时才落盘，
                                    鉴权失败（可用模型列表为空）时用户找不到任何保存按钮 */}
                              <Button
                                variant="default"
                                size="sm"
                                onClick={saveKey}
                                disabled={savingKey || !apiKey.trim()}
                                title={TXT.saveKeyTitle}
                              >
                                {savingKey ? TXT.savingKey : TXT.saveKeyBtn}
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

                        {/* 本地服务可选填地址（自定义实例的地址在表单里） */}
                        {isLocal && (
                          <FormRow
                            stacked
                            label={TXT.baseUrlLabel}
                            hint={TXT.baseUrlHelp}
                            control={
                              <input
                                className="compact-input"
                                value={baseUrl}
                                onChange={e => setBaseUrl(e.target.value)}
                                placeholder="http://localhost:11434/v1"
                                aria-label={TXT.baseUrlLabel}
                              />
                            }
                          />
                        )}
                      </>
                    )}

                    {detectError && <div className="detect-error">{detectError}</div>}

                    {isAnthropicInstance && (
                      <div className="text-caption hint-text">{TXT.anthropicNoModelList}</div>
                    )}

                    {(isCustom || isLocal) && baseUrl.trim() !== loadedBaseUrl.trim() && (
                      <div className="models-baseurl-warn" role="alert">
                        <span className="models-baseurl-warn-text">
                          {isAnthropicInstance
                            ? TXT.baseUrlChangedWarnManual
                            : TXT.baseUrlChangedWarn}
                        </span>
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
                        {/* Anthropic 实例只能手动填模型名：这条入口是本页唯一的获取途径，
                            因此用强调态呈现，文案也直说「模型名」 */}
                        <button
                          type="button"
                          className={[
                            'models-refresh-btn',
                            isAnthropicInstance ? 'is-emphasis' : '',
                          ]
                            .filter(Boolean)
                            .join(' ')}
                          onClick={() => {
                            setAddOpen(v => !v)
                            setAddError(null)
                          }}
                          title={TXT.addModelTitle}
                        >
                          {isAnthropicInstance ? TXT.addModelBtnManual : TXT.addModelBtn}
                        </button>
                        {!isAnthropicInstance && (
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
                        )}
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
                    {/* 显式刷新的落盘摘要：新增/更新/移除 + 被覆写/被移除的 id（供追溯与重加） */}
                    {syncSummary && (
                      <div className="models-sync-summary" role="status">
                        <span className="models-sync-summary-text">
                          {TXT.syncSummary(syncSummary)}
                          {syncSummary.updated_ids.length > 0 && (
                            <>
                              <br />
                              {TXT.syncSummaryUpdated(syncSummary.updated_ids)}
                            </>
                          )}
                          {syncSummary.removed_ids.length > 0 && (
                            <>
                              <br />
                              {TXT.syncSummaryRemoved(syncSummary.removed_ids)}
                            </>
                          )}
                          {syncSummary.kept_manual > 0 && (
                            <>
                              {' · '}
                              {TXT.syncSummaryKeptManual(syncSummary.kept_manual)}
                            </>
                          )}
                        </span>
                        <button
                          type="button"
                          className="icon-btn-ghost"
                          onClick={() => setSyncSummary(null)}
                          title={TXT.syncSummaryDismiss}
                          aria-label={TXT.syncSummaryDismiss}
                        >
                          <IconX size={12} />
                        </button>
                      </div>
                    )}

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
                              {q
                                ? TXT.emptyFiltered(filterInput)
                                : isAnthropicInstance
                                  ? TXT.emptyNeedManual
                                  : TXT.emptyNeedConnect}
                            </div>
                          </div>
                        )
                      }
                      if (detecting) {
                        return <div className="models-empty">正在连接并获取模型列表…</div>
                      }
                      if (filtered.length === 0) {
                        return <div className="models-empty">{TXT.emptyFiltered(filterInput)}</div>
                      }
                      const briefById = new Map<string, ProviderModelBrief>(
                        detectedModels.map(d => [d.id, d]),
                      )
                      const infoById = new Map<string, ModelInfo>(allModels.map(m => [m.id, m]))
                      return (
                        <div className="models-list-wrap">
                          <div className="detect-status">{TXT.modelsCount(display.length)}</div>
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
                                      // 不下发内置默认地址：自定义端点为文档占位示例，
                                      // 空值交给后端解析已存配置/内置默认。
                                      const resolvedBaseUrl = baseUrl.trim() || undefined
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
                                        msg: friendlyIpcError(e, '切换失败'),
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
                                    {/* 官方清单外：手动添加的模型，刷新时不会被移除 */}
                                    {info?.source === 'manual' && (
                                      <span
                                        className="model-manual-badge"
                                        title={TXT.manualBadgeTitle}
                                      >
                                        {TXT.manualBadge}
                                      </span>
                                    )}
                                  </div>
                                  <div className="model-list-badges">
                                    {/* 视觉能力开关：与「上下文窗口」同为行内模型元数据编辑，
                                        开关本身即状态（关闭态 = 该模型不进图像理解候选列表）。 */}
                                    <button
                                      type="button"
                                      className={`model-vision-toggle${caps.vision ? ' is-on' : ''}`}
                                      aria-pressed={caps.vision}
                                      aria-label={`视觉输入能力：${caps.vision ? '已开启' : '未开启'}`}
                                      title={
                                        caps.vision
                                          ? '已支持视觉输入（点击关闭后，该模型不再出现在图像理解模型列表）'
                                          : '点击标记为支持视觉输入（支持图片的模型才会出现在图像理解模型列表）'
                                      }
                                      disabled={visionToggling === name}
                                      onClick={e => {
                                        e.stopPropagation()
                                        void toggleModelVision(name, !caps.vision)
                                      }}
                                    >
                                      <IconEye size={12} />
                                    </button>
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
                                {/* 本地端点也可能是多模态（本地视觉模型）：同样可标记，
                                    否则「图像理解模型」列表对本地服务永久为空。 */}
                                <button
                                  type="button"
                                  className={`model-vision-toggle${rowVision(m) ? ' is-on' : ''}`}
                                  aria-pressed={rowVision(m)}
                                  aria-label={`视觉输入能力：${rowVision(m) ? '已开启' : '未开启'}`}
                                  title={
                                    rowVision(m)
                                      ? '已支持视觉输入（点击关闭后，该模型不再出现在图像理解模型列表）'
                                      : '点击标记为支持视觉输入（支持图片的模型才会出现在图像理解模型列表）'
                                  }
                                  disabled={visionToggling === m}
                                  onClick={e => {
                                    e.stopPropagation()
                                    void toggleModelVision(m, !rowVision(m))
                                  }}
                                >
                                  <IconEye size={12} />
                                </button>
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

                  {/* ── 高级：ExecAgent 子模型 已迁移至右上角「子智能体模型」入口 ── */}
                </>
              )}

              {/* local：添加自定义模型（手动录入，用于本地网关模型列表外补充） */}
              {activeView === 'provider' && !formOpen && provider === 'local' && (
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
                      hint="已确认支持视觉输入的模型会显示图标；自定义/中转模型即使未探测到能力，也可以手动选择。"
                      control={
                        <VisionModelSelect
                          value={visionModel}
                          provider={visionProvider}
                          models={allModels}
                          filterCapability="vision"
                          placeholder={TXT.visionNone}
                          onChange={async (modelId, selectedProvider) => {
                            setVisionSaving(true)
                            setVisionFeedback(null)
                            try {
                              // 原子写入：model 与 provider 一起落盘，杜绝
                              // 「新 model + 旧 provider」的半绑定中间态。
                              await setVisionCapability(modelId, selectedProvider)
                              setVisionModel(modelId)
                              setVisionProvider(selectedProvider)
                              setVisionFeedback({ ok: true, msg: '图像理解模型已保存' })
                              setTimeout(() => setVisionFeedback(null), 2000)
                            } catch (e: any) {
                              setVisionFeedback({
                                ok: false,
                                msg: friendlyIpcError(e, '保存失败'),
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
                    {/* 图像理解生效路径如实呈现：默认「跟随 Leader」时讲清由谁处理，
                        Leader 不支持视觉时明确警示——否则用户无从知道截图识别已不可用
                        （视觉不可用时桌面操作会真实报错，这里提前把原因摆在配置处）。 */}
                    <div
                      className={`text-caption${!visionModel && leaderVisionModelId && !leaderSupportsVision ? ' text-danger' : ''}`}
                    >
                      {visionModel
                        ? `已显式指定：图像统一由 ${visionModel} 理解（不再跟随 Leader；换 Leader 不会自动改这里）`
                        : leaderVisionModelId
                          ? leaderSupportsVision
                            ? `跟随 Leader：图像由当前 Leader 模型 ${leaderVisionModelId} 直接理解，无需额外配置`
                            : `⚠ 当前 Leader 模型 ${leaderVisionModelId} 不支持图像理解 —— 截图/图像识别类操作将不可用。请在此指定一个图像理解模型，或把 Leader 换成支持视觉的模型。`
                          : '跟随 Leader：图像由当前 Leader 模型直接理解（尚未检测到 Leader 模型信息）'}
                    </div>
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
                              setSttFeedback({ ok: false, msg: friendlyIpcError(e, '保存失败') })
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
                              setTtsFeedback({ ok: false, msg: friendlyIpcError(e, '保存失败') })
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
                              setVoiceFeedback({ ok: false, msg: friendlyIpcError(e, '保存失败') })
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
                          onChange={(m, provider) => void saveAgentModel('exec', m, provider)}
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

              {activeView === 'jev' && <JevSettings />}
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
