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
  setModelSupportsImageGeneration,
  setModelSupportsVision,
  setCapabilityBinding,
  createCustomProvider,
  updateCustomProvider,
  removeCustomProvider,
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
  IconStar,
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
import { HOT_MODELS } from '../lib/hotModels'
import {
  getDefaultModel,
  getPickedModels,
  setDefaultModel,
  setModelVisible,
} from '../lib/modelPrefs'
import { JevSettings } from './JevSettings'
// 页内反馈与 island 共用同一套胶囊视觉（组件自带共享层 app-pill.css）
import { AppPill } from '../../ui/AppPill'
import '../../styles/models.css'

// ════════════════════════════════════════════════════════════════
// 文案表：makeTXT(t) 按当前语言生成，键名与结构保持稳定。
// 标题与按钮走 i18n 字典（models.* 命名空间，zh/en 双侧齐备）；
// 尚未迁移的条目暂留中文兜底，后续按同一方式逐批迁移。
// ════════════════════════════════════════════════════════════════
type TFunc = (key: string, ...args: string[]) => string

/**
 * 词条取值：字典命中 → 用字典文案；未命中 → 用中文兜底。
 *
 * 兜底存在的意义：本页词条分批迁移期，未补进 zh/en 字典的条目仍显示可读中文，
 * 而不是泄漏 key。`t()` 本身在 miss 时返回原 key，故此处显式判等。
 */
const tr = (t: TFunc, key: string, fallback: string): string => {
  const v = t(key)
  return v === key ? fallback : v
}

function makeTXT(t: TFunc) {
  return {
    // ── 自定义模型四字段（名称 / 提供商 / API Key / API URL）──
    // 表单、实例配置页共用同一套字段名，全流程一个说法，用户不用做同义词翻译。
    nameLabel: tr(t, 'models.nameLabel', '自定义名称'),
    namePlaceholder: tr(t, 'models.namePlaceholder', '例如：公司网关'),
    providerTypeLabel: tr(t, 'models.providerTypeLabel', '模型提供商'),
    customKeyLabel: tr(t, 'models.customKeyLabel', '模型 API Key'),
    customBaseUrlLabel: tr(t, 'models.customBaseUrlLabel', '模型 API URL'),
    providerTypes: [
      { value: 'custom', label: tr(t, 'models.providerTypeOpenai', 'OpenAI 兼容') },
      { value: 'anthropic', label: tr(t, 'models.providerTypeAnthropic', 'Anthropic 兼容') },
    ],
    keyPlaceholderCustom: tr(t, 'models.keyPlaceholderCustom', '无鉴权端点可留空'),
    /** 编辑模式留空 = 不改密钥（不是清空）：密钥已在后端，界面不回显 */
    keyPlaceholderKeep: tr(t, 'models.keyPlaceholderKeep', '留空则保持原密钥不变'),
    /** 由名称 slug 化来的段 id 只在 title 里作调试信息，界面不显示 */
    instanceIdTitle: (id: string) => `实例标识：${id}`,
    /** 左栏固定入口（custom 配置界面）显示名走 i18n：models.customEntry */
    /** 创建态/编辑态标题：同一套表单，只有按钮语义不同（「创建」/「保存」） */
    customSectionTitle: tr(t, 'models.customSectionTitle', '自定义模型'),
    createBtn: tr(t, 'models.createBtn', '创建'),
    creating: tr(t, 'models.creating', '创建中…'),
    /** 编辑页主按钮（同一套表单，只有按钮语义不同） */
    saveBtn: tr(t, 'models.saveBtn', '保存'),
    savingBtn: tr(t, 'models.savingBtn', '保存中…'),
    /** 「连接测试」：用当前填写的地址与密钥探测可用模型 */
    testBtn: tr(t, 'models.testBtn', '连接测试'),
    createSuccess: tr(t, 'models.createSuccess', '已创建'),
    saveSuccess: tr(t, 'models.saveSuccess', '已保存'),
    createFail: tr(t, 'models.createFail', '创建失败'),
    needName: tr(t, 'models.needName', '请填写自定义名称'),
    needUrl: tr(t, 'models.needUrl', '请填写模型 API URL'),
    /**
     * Anthropic 兼容中转的模型列表说明。
     *
     * 历史上这里写「Anthropic 协议不支持自动获取模型列表，请手动填写模型名」，
     * 并把「刷新」按钮对 anthropic 实例整体隐藏——该说法对中转站是假的：
     * 后端 `fetch_provider_models` 与协议无关（GET {base}/models + 按段类型发
     * x-api-key），实测主流中转两条路由均返回 200。保留手动添加作为兜底路径，
     * 但入口必须给全，否则用户只能逐个手敲模型 id。
     */
    anthropicModelListHint: tr(
      t,
      'models.anthropicModelListHint',
      '点「刷新」从中转站拉取模型列表；若该服务商不返回列表，再用「+ 手动添加模型名」。',
    ),
    apiKeyLabel: tr(t, 'models.apiKeyLabel', 'API 密钥'),
    keyInputPlaceholder: (name: string) => t('models.keyInputPlaceholder', name),
    keyOverwritePlaceholder: tr(t, 'models.keyOverwritePlaceholder', '输入新密钥覆盖现有配置'),
    keyShow: tr(t, 'models.keyShow', '显示'),
    keyHide: tr(t, 'models.keyHide', '隐藏'),
    connectBtn: tr(t, 'models.connectBtn', '连接'),
    connecting: tr(t, 'models.connecting', '连接中…'),
    connectTitle: tr(t, 'models.connectTitle', '探测该接口下可用的模型'),
    clearKeyTitle: tr(t, 'models.clearKeyTitle', '清除已保存的密钥'),
    clearKeyConfirm: tr(
      t,
      'models.clearKeyConfirm',
      '确定要清除该服务商的 API 密钥吗？\n模型与其它配置会保留，清除后需重新输入才能使用云端服务。',
    ),
    keyHelp: tr(t, 'models.keyHelp', '密钥仅存本机'),
    keyHelpLocal: tr(
      t,
      'models.keyHelpLocal',
      '本地服务通常无需密钥，可留空直接连接；若服务启用了鉴权（如 llama-swap、带 key 的网关/反向代理），在此填写后再连接。',
    ),
    keyPlaceholderLocal: tr(t, 'models.keyPlaceholderLocal', '可选：本地服务启用鉴权时填写密钥'),
    saveKeyBtn: tr(t, 'models.saveKeyBtn', '保存'),
    savingKey: tr(t, 'models.savingKey', '保存中…'),
    saveKeyTitle: tr(t, 'models.saveKeyTitle', '仅保存密钥（不检测模型）'),
    saveKeyNoModel: tr(t, 'models.saveKeyNoModel', '请先点「连接测试」获取模型'),
    /** Anthropic 实例没有「连接」按钮，只能手动加模型名 */
    saveKeyNoModelManual: tr(t, 'models.saveKeyNoModelManual', '请先手动添加模型名'),
    saveKeySuccess: (m: string) => t('models.saveKeySuccess', m),
    /** 无鉴权实例（key 留空）保存：只说实际发生了什么（地址与默认模型落盘） */
    saveNoKeySuccess: (m: string) => t('models.saveNoKeySuccess', m),
    saveKeyFail: tr(t, 'models.saveKeyFail', '保存密钥失败'),
    filterPlaceholder: tr(t, 'models.filterPlaceholder', '筛选模型…'),
    baseUrlLabel: tr(t, 'models.baseUrlLabel', '接口地址'),
    baseUrlPlaceholder: 'https://your-relay.com/v1',
    baseUrlHelp: tr(t, 'models.baseUrlHelp', '中转站或网关的接口地址，通常以 /v1 结尾'),
    /** 自定义标头编辑区：中转站网关要求的附加请求头，逐字注入每个请求 */
    headerSectionLabel: tr(t, 'models.headerSectionLabel', '自定义标头'),
    headerSectionHelp: tr(t, 'models.headerSectionHelp', '部分中转站要求附加请求头；留空则不发送'),
    headerNamePlaceholder: tr(t, 'models.headerNamePlaceholder', '头名称，如 X-Gateway'),
    headerValuePlaceholder: tr(t, 'models.headerValuePlaceholder', '值（可为空）'),
    addHeaderBtn: tr(t, 'models.addHeaderBtn', '+ 添加标头'),
    removeHeaderTitle: tr(t, 'models.removeHeaderTitle', '删除此标头'),
    // ── 订阅账号（OAuth，可选路径）──
    // 静态密钥与 OAuth 是同一实例的二选一凭证来源；区块默认折叠，避免给只用密钥的
    // 用户增加六项 OAuth 字段的视觉负担（折叠态只留一行状态摘要）。
    oauthSectionLabel: tr(t, 'models.oauthSectionLabel', '订阅账号（OAuth）'),
    oauthSummaryOff: tr(t, 'models.oauthSummaryOff', '未配置'),
    oauthSummaryNotLoggedIn: tr(t, 'models.oauthSummaryNotLoggedIn', '已配置 · 未登录'),
    oauthSummaryLoggedIn: tr(t, 'models.oauthSummaryLoggedIn', '已登录'),
    oauthSummaryNeedsLogin: tr(t, 'models.oauthSummaryNeedsLogin', '需重新授权'),
    /** 编辑态：OAuth 五项随 ProviderInfo.oauth 摘要回显可改；三态语义见 oauthToPayload */
    oauthHelpEdit: tr(
      t,
      'models.oauthHelpEdit',
      '填写则覆盖更新；清空三项必填并保存 = 切回 API Key 模式（移除 OAuth 配置）。',
    ),
    oauthHelpCreate: tr(
      t,
      'models.oauthHelpCreate',
      '填写三项必填即为该实例启用 OAuth；创建完成后可在此授权登录。',
    ),
    /** 创建态：实例尚未落盘，后端 oauth_begin 找不到段 → 只能先说明可行路径 */
    oauthCreateHint: tr(
      t,
      'models.oauthCreateHint',
      '实例创建完成后，回到本页即可用「授权登录」绑定订阅账号。',
    ),
    oauthAuthorizeLabel: tr(t, 'models.oauthAuthorizeLabel', '授权端点'),
    oauthAuthorizePlaceholder: 'https://sso.example.com/authorize',
    oauthTokenLabel: tr(t, 'models.oauthTokenLabel', '令牌端点'),
    oauthTokenPlaceholder: 'https://sso.example.com/token',
    oauthClientIdLabel: tr(t, 'models.oauthClientIdLabel', 'Client ID'),
    oauthClientIdPlaceholder: tr(t, 'models.oauthClientIdPlaceholder', 'OAuth 客户端标识'),
    oauthScopesLabel: tr(t, 'models.oauthScopesLabel', 'Scopes'),
    oauthScopesPlaceholder: tr(t, 'models.oauthScopesPlaceholder', '空格分隔，可留空'),
    oauthScopesHelp: tr(t, 'models.oauthScopesHelp', '空格分隔；留空则授权请求不带 scope'),
    oauthPkceLabel: tr(t, 'models.oauthPkceLabel', 'PKCE'),
    oauthPkceHelp: tr(
      t,
      'models.oauthPkceHelp',
      '推荐开启（RFC 7636）；仅当授权服务器不支持时关闭',
    ),
    oauthPkceOn: tr(t, 'models.oauthPkceOn', '开启'),
    oauthPkceOff: tr(t, 'models.oauthPkceOff', '关闭'),
    oauthRedirectPortLabel: tr(t, 'models.oauthRedirectPortLabel', '回调端口'),
    oauthRedirectPortPlaceholder: tr(t, 'models.oauthRedirectPortPlaceholder', '留空由系统分配'),
    oauthRedirectPortHelp: tr(
      t,
      'models.oauthRedirectPortHelp',
      '本地回调监听端口（http://127.0.0.1:<端口>/callback）',
    ),
    oauthPortInvalid: tr(t, 'models.oauthPortInvalid', '回调端口需为 1-65535 的整数'),
    oauthNeedRequired: tr(
      t,
      'models.oauthNeedRequired',
      '请先填写授权端点 / 令牌端点 / Client ID（三项必填）',
    ),
    oauthLoginBtn: tr(t, 'models.oauthLoginBtn', '授权登录'),
    oauthReauthorizeBtn: tr(t, 'models.oauthReauthorizeBtn', '重新授权'),
    oauthLogoutBtn: tr(t, 'models.oauthLogoutBtn', '退出登录'),
    oauthLoggingIn: tr(t, 'models.oauthLoggingIn', '发起中…'),
    oauthLoggedIn: tr(t, 'models.oauthLoggedIn', '已登录'),
    oauthExpiresAt: (time: string) => {
      const k = 'models.oauthExpiresAtLabel'
      return t(k, time) === k ? `有效期至 ${time}` : t(k, time)
    },
    oauthNotLoggedIn: tr(t, 'models.oauthNotLoggedIn', '尚未登录：点「授权登录」在浏览器完成授权'),
    oauthNeedsLogin: tr(t, 'models.oauthNeedsLogin', '登录已失效：需重新授权'),
    oauthBrowserOpened: tr(
      t,
      'models.oauthBrowserOpened',
      '已打开浏览器授权页，完成后本页自动更新登录状态',
    ),
    oauthLoginSuccess: tr(t, 'models.oauthLoginSuccess', '授权成功'),
    oauthLogoutDone: tr(t, 'models.oauthLogoutDone', '已退出登录'),
    oauthBeginFail: tr(t, 'models.oauthBeginFail', '发起授权失败'),
    oauthOpenFail: tr(t, 'models.oauthOpenFail', '打开浏览器失败'),
    oauthLogoutFail: tr(t, 'models.oauthLogoutFail', '退出登录失败'),
    modelListTitle: tr(t, 'models.modelListTitle', '可用模型'),
    currentModelOf: (name: string) => t('models.currentModelOf', name),
    refreshBtn: tr(t, 'models.refreshBtn', '刷新'),
    refreshing: tr(t, 'models.refreshing', '刷新中…'),
    refreshTitle: tr(t, 'models.refreshTitle', '用已保存密钥重新拉取最新模型列表'),
    addModelBtn: tr(t, 'models.addModelBtn', '+ 手动添加'),
    /** Anthropic 实例：这是唯一能拿到模型的入口，文案直接说清要填什么 */
    addModelBtnManual: tr(t, 'models.addModelBtnManual', '+ 手动添加模型名'),
    addModelTitle: tr(t, 'models.addModelTitle', '手动添加模型代号'),
    addModelPlaceholder: tr(t, 'models.addModelPlaceholder', '输入模型代号'),
    addModelConfirm: tr(t, 'models.addModelConfirm', '添加'),
    addModelCancel: tr(t, 'models.addModelCancel', '取消'),
    addModelRequired: tr(t, 'models.addModelRequired', '请输入模型代号'),
    addModelSuccess: (id: string) => t('models.addModelSuccess', id),
    addModelFail: tr(t, 'models.addModelFail', '添加失败'),
    baseUrlChangedWarn: tr(
      t,
      'models.baseUrlChangedWarn',
      '接口地址已变更：旧模型列表可能在新地址下不可用（模型代号不存在，或同名模型能力不同）。建议清理后重新拉取。',
    ),
    /** Anthropic 实例同样可刷新，仅文案区别（原为「无法重新拉取」） */
    baseUrlChangedWarnManual: tr(
      t,
      'models.baseUrlChangedWarnManual',
      '接口地址已变更：旧模型名可能在新地址下不可用。可点「刷新」重新拉取，或清理后手动添加模型名。',
    ),
    clearModelsBtn: tr(t, 'models.clearModelsBtn', '清理旧模型'),
    clearModelsConfirm: tr(
      t,
      'models.clearModelsConfirm',
      '确定清理该服务商的旧模型列表吗？\n仅清空模型条目（名称/地址/密钥保留），清理后请点「刷新」拉取新地址的模型。',
    ),
    clearModelsConfirmManual: tr(
      t,
      'models.clearModelsConfirmManual',
      '确定清理该实例的旧模型列表吗？\n仅清空模型条目（名称/地址/密钥保留），清理后可点「刷新」重新拉取，或手动添加模型名。',
    ),
    clearModelsSuccess: (n: number) => t('models.clearModelsSuccess', String(n)),
    clearModelsNone: tr(t, 'models.clearModelsNone', '没有可清理的旧模型'),
    clearModelsFail: tr(t, 'models.clearModelsFail', '清理失败'),
    emptyFiltered: (q: string) => t('models.emptyFiltered', q),
    emptyNeedConnect: tr(t, 'models.emptyNeedConnect', '尚未获取模型，点「连接测试」'),
    emptyNeedManual: tr(t, 'models.emptyNeedManual', '尚未添加模型，点「+ 手动添加模型名」'),
    modelsCount: (n: number) => t('models.modelsCount', String(n)),
    capVision: tr(t, 'models.capVisionLabel', '支持图像理解'),
    capAudio: tr(t, 'models.capAudioLabel', '支持语音'),
    capImageGen: tr(t, 'models.capImageGenLabel', '支持图像生成'),
    editContext: tr(t, 'models.editContext', '设置上下文窗口（K tokens）'),
    ctxUnknown: tr(t, 'models.ctxUnknown', '上下文窗口未知'),
    removeModelConfirm: (name: string) => t('models.removeModelConfirm', name),
    /** 删除自定义实例的二次确认：讲清后果（整段配置移除、不可撤销）。 */
    removeInstanceConfirm: (name: string) => t('models.removeInstanceConfirm', name),
    removeInstanceTitle: tr(t, 'models.removeInstanceTitle', '删除自定义模型'),
    removeSuccess: (name: string) => t('models.removeSuccess', name),
    removeFail: tr(t, 'models.removeFail', '删除失败'),
    removing: tr(t, 'models.removing', '删除中…'),
    cancel: tr(t, 'models.cancel', '取消'),
    confirmDelete: tr(t, 'models.confirmDelete', '删除'),
    clearKeySuccess: tr(t, 'models.clearKeySuccess', '密钥已清除'),
    clearKeyFail: tr(t, 'models.clearKeyFail', '清除失败'),
    savingModel: tr(t, 'models.savingModel', '切换中…'),
    saveOk: tr(t, 'models.saveOk', '保存成功'),
    saveFail: tr(t, 'models.saveFail', '保存失败'),
    apiKeyRequired: tr(t, 'models.apiKeyRequired', '请先输入 API 密钥'),
    ctxUnitHint: tr(t, 'models.ctxUnitHint', '单位 K（千 tokens），例如 128 = 128K'),
    ctxCap: tr(t, 'models.ctxCap', '上下文窗口'),
    visionNone: tr(t, 'models.visionNone', '跟随 Leader 模型（需支持图像理解）（推荐）'),
    downloadReady: tr(t, 'models.downloadReady', '已就绪'),
    downloadPaused: tr(t, 'models.downloadPaused', '下载已暂停'),
    /** 显式刷新后的同步摘要（新增 / 更新 / 移除，并列出被覆写与被移除的 id） */
    syncSummary: (r: SyncReport) =>
      t('models.syncSummary', String(r.added), String(r.updated), String(r.removed)) ===
      'models.syncSummary'
        ? `已与官方模型清单同步：新增 ${r.added} · 更新 ${r.updated} · 移除 ${r.removed}`
        : t('models.syncSummary', String(r.added), String(r.updated), String(r.removed)),
    syncSummaryUpdated: (ids: string[]) =>
      t('models.syncSummaryUpdated', ids.join(', ')) === 'models.syncSummaryUpdated'
        ? `能力已更新：${ids.join('、')}`
        : t('models.syncSummaryUpdated', ids.join(', ')),
    syncSummaryRemoved: (ids: string[]) =>
      t('models.syncSummaryRemoved', ids.join(', ')) === 'models.syncSummaryRemoved'
        ? `移除：${ids.join('、')}`
        : t('models.syncSummaryRemoved', ids.join(', ')),
    syncSummaryKeptManual: (n: number) =>
      t('models.syncSummaryKeptManual', String(n)) === 'models.syncSummaryKeptManual'
        ? `保留手动添加 ${n} 条`
        : t('models.syncSummaryKeptManual', String(n)),
    syncSummaryDismiss: tr(t, 'models.syncSummaryDismiss', '关闭同步摘要'),
    /** 手动添加条目（不在官方 /v1/models 清单内）的行内标注 */
    manualBadge: tr(t, 'models.manualBadge', '官方清单外'),
    manualBadgeTitle: tr(
      t,
      'models.manualBadgeTitle',
      '手动添加的模型：不在官方 /v1/models 清单内，刷新时不会被移除',
    ),
    providerSectionTitle: tr(t, 'models.providerSectionTitle', '模型服务商'),
    // ── 左侧栏分组标题 ──
    railGroupProviders: tr(t, 'models.railGroupProviders', '模型提供商'),
    railGroupCustom: tr(t, 'models.railGroupCustom', '自定义模型'),
    railGroupLocal: tr(t, 'models.railGroupLocal', '本地模型'),
    railGroupAutomation: tr(t, 'models.railGroupAutomation', '自动化增强'),
    railLoadingProviders: tr(t, 'models.railLoadingProviders', '正在加载服务商…'),
    // ── 右上角工具栏 ──
    toolbarCapabilities: tr(t, 'models.toolbarCapabilities', '图像音频模型'),
    toolbarAgents: tr(t, 'models.toolbarAgents', '子智能体模型'),
    // ── 图像理解 ──
    visionSection: tr(t, 'models.visionSection', '图像理解'),
    visionSectionDesc: tr(
      t,
      'models.visionSectionDesc',
      '配置图像理解模型后，对话中的截图 / 图片可被自动识别（OCR 与界面描述）。留空表示使用默认模型；已确认支持视觉输入的模型会显示图标。中转模型的能力多由自动探测写入，探测没认出来时，用下方的「标记为支持视觉输入」把当前模型补进候选列表。',
    ),
    visionSaved: tr(t, 'models.visionSaved', '已保存'),
    visionMarkHint: (m: string) =>
      t('models.visionMarkHint', m) === 'models.visionMarkHint'
        ? `当前模型 ${m} 未标记支持图像输入，不在上方候选列表里`
        : t('models.visionMarkHint', m),
    visionMarkBtn: tr(t, 'models.visionMarkBtn', '标记为支持视觉输入'),
    visionMarkDone: (m: string) =>
      t('models.visionMarkDone', m) === 'models.visionMarkDone'
        ? `已将 ${m} 标记为支持视觉输入`
        : t('models.visionMarkDone', m),
    saveFailed: tr(t, 'models.visionSaveFail', '保存失败'),
    visionExplicitPath: (m: string) =>
      t('models.visionExplicitPath', m) === 'models.visionExplicitPath'
        ? `已显式指定：图像统一由 ${m} 理解（不再跟随 Leader；换 Leader 不会自动改这里）`
        : t('models.visionExplicitPath', m),
    visionFollowOk: (m: string) =>
      t('models.visionFollowOk', m) === 'models.visionFollowOk'
        ? `跟随 Leader：图像由当前 Leader 模型 ${m} 直接理解，无需额外配置`
        : t('models.visionFollowOk', m),
    visionFollowWarn: (m: string) =>
      t('models.visionFollowWarn', m) === 'models.visionFollowWarn'
        ? `⚠ 当前 Leader 模型 ${m} 不支持图像理解 —— 截图/图像识别类操作将不可用。请在此指定一个图像理解模型，或把 Leader 换成支持视觉的模型。`
        : t('models.visionFollowWarn', m),
    visionFollowUnknown: tr(
      t,
      'models.visionFollowUnknown',
      '跟随 Leader：图像由当前 Leader 模型直接理解（尚未检测到 Leader 模型信息）',
    ),
    // ── 子智能体模型 ──
    execSection: tr(t, 'models.execSection', '子任务执行模型（Exec）'),
    execSectionDesc: tr(
      t,
      'models.execSectionDesc',
      'ExecAgent 由 Leader 模式下派发、执行子任务时使用的模型。留空则跟随全局默认模型。',
    ),
    execFollowDefault: tr(t, 'models.execFollowDefault', '跟随默认模型'),
    // ── 左栏：增强判断模型 ──
    railGroupEnhancedJudge: tr(t, 'models.railGroupEnhancedJudge', '增强判断模型'),
    // ── 本地视觉模型（OCR / UI 检测）──
    localVisionSection: tr(t, 'models.localVisionSection', '本地视觉模型（OCR / UI 检测）'),
    localVisionSectionDesc: tr(
      t,
      'models.localVisionSectionDesc',
      '屏幕理解所需的本地 OCR 与界面元素检测模型。随应用自动下载，无需手动操作；仅当缺少文件时需要处理。',
    ),
    ocrReady: tr(t, 'models.ocrReady', 'OCR 已就绪'),
    ocrNotReady: tr(t, 'models.ocrNotReady', 'OCR 未就绪'),
    yoloReady: tr(t, 'models.yoloReady', 'UI 检测已就绪'),
    yoloDisabled: tr(t, 'models.yoloDisabled', 'UI 检测未启用'),
    modelDirLabel: tr(t, 'models.modelDirLabel', '模型目录：'),
    downloadingHint: tr(t, 'models.downloadingHint', '正在后台自动下载，下载完成即可使用屏幕理解…'),
    downloadFailed: tr(t, 'models.downloadFailed', '下载失败：'),
    retryDownload: tr(t, 'models.retryDownload', '重试下载'),
    downloadNow: tr(t, 'models.downloadNow', '立即下载'),
    detecting: tr(t, 'models.detecting', '检测中…'),
    screenReady: tr(t, 'models.screenReady', '屏幕理解（OCR + UI 元素检测）已就绪'),
    missingOptional: (names: string) => {
      const k = 'models.missingOptional'
      return t(k, names) === k ? `缺少 ${names}（可选，仅影响 UI 元素检测）` : t(k, names)
    },
    missingRequired: (n: number) => {
      const k = 'models.missingRequired'
      return t(k, String(n)) === k
        ? `缺少 ${n} 个模型文件，下载后即可使用屏幕理解`
        : t(k, String(n))
    },
    // ── 语音输入（STT）──
    sttSection: tr(t, 'models.sttSection', '语音输入'),
    sttSectionDesc: tr(
      t,
      'models.sttSectionDesc',
      '在输入框用语音转文字。配置云端识别模型后优先使用云端识别；未配置则使用本地离线识别（中文优化，无需联网）。',
    ),
    sttPlaceholder: tr(t, 'models.sttPlaceholder', '未配置（使用本地识别）'),
    sttSaved: tr(t, 'models.sttSaved', '云端识别模型已保存'),
    sttNoMic: tr(t, 'models.sttNoMic', '未检测到麦克风，连接麦克风后即可使用语音输入'),
    sttLocalModelMissingCloudOk: tr(
      t,
      'models.sttLocalModelMissingCloudOk',
      '本地模型未下载（云端识别已可用，仅离线识别时需要）',
    ),
    sttDownloadHint: tr(
      t,
      'models.sttDownloadHint',
      '下载语音模型（约 250 MB）即可开始本地语音输入',
    ),
    downloadVoiceModel: tr(t, 'models.downloadVoiceModel', '下载语音模型'),
    // ── 文字转语音（TTS）──
    ttsSection: tr(t, 'models.ttsSection', '文字转语音（TTS）'),
    ttsSectionDesc: tr(
      t,
      'models.ttsSectionDesc',
      '配置文字转语音模型，用于 AI 回复的语音朗读，支持 OpenAI 兼容的 TTS 服务；留空表示不使用朗读功能。',
    ),
    ttsPlaceholder: tr(t, 'models.ttsPlaceholder', '未配置（不使用朗读）'),
    ttsSaved: tr(t, 'models.ttsSaved', 'TTS 模型已保存'),
    // ── 语音克隆 ──
    voiceSection: tr(t, 'models.voiceSection', '语音克隆'),
    voiceSectionDesc: tr(
      t,
      'models.voiceSectionDesc',
      '配置语音克隆模型（云端克隆 API），配置后语音克隆工具可用；留空表示不使用语音克隆。',
    ),
    voicePlaceholder: tr(t, 'models.voicePlaceholder', '未配置（不使用）'),
    voiceSaved: tr(t, 'models.voiceSaved', '语音克隆模型已保存'),
    // ── 自定义实例条目（左栏）──
    keyConfigured: tr(t, 'models.keyConfigured', '已配置密钥'),
    keyNotConfigured: tr(t, 'models.keyNotConfigured', '未配置密钥（无鉴权端点可留空）'),
    removeInstanceLabel: (name: string) => {
      const k = 'models.removeInstanceLabel'
      return t(k, name) === k ? `删除「${name}」` : t(k, name)
    },
    // ── provider 详情区 ──
    keyNotConfiguredShort: tr(t, 'models.keyNotConfiguredShort', '尚未配置密钥'),
    modelListDescEmpty: tr(
      t,
      'models.modelListDescEmpty',
      '选择一个模型作为默认使用（点击行即可切换）。',
    ),
    detectingModels: tr(t, 'models.detectingModels', '正在连接并获取模型列表…'),
    goGateway: tr(t, 'models.goGateway', 'OpenCode Go 网关'),
    audioSupported: tr(t, 'models.audioSupported', '支持语音'),
    // ── 行内偏好：精选（眼睛）/ 厂商默认（⭐）／热门快捷 ──
    // 说明：行内视觉能力开关已移除（视觉能力改由后端探测自动判定，方案 C）。
    visibleToggleAria: (state: string) => t('models.visibleToggleAria', state),
    visibleToggleTitleOn: tr(
      t,
      'models.visibleToggleTitleOn',
      '已加入精选（该厂商切换弹窗只显示精选模型；点击移出）',
    ),
    visibleToggleTitleOff: tr(
      t,
      'models.visibleToggleTitleOff',
      '点击加入精选（该厂商切换弹窗将只显示精选模型）',
    ),
    defaultToggleAria: (state: string) => t('models.defaultToggleAria', state),
    defaultToggleTitleOn: tr(
      t,
      'models.defaultToggleTitleOn',
      '已设为该厂商默认模型（点击取消）',
    ),
    defaultToggleTitleOff: tr(
      t,
      'models.defaultToggleTitleOff',
      '设为该厂商默认模型（仅记录偏好，不会立即切换）',
    ),
    hotModelsTitle: tr(t, 'models.hotModelsTitle', '热门模型'),
    removeFromLocalList: tr(t, 'models.removeFromLocalList', '从本地列表移除'),
    localCtxLabel: tr(t, 'models.localCtxLabel', '本地模型默认上下文（K tokens）'),
    localCtxHint: tr(
      t,
      'models.localCtxHint',
      '为空则每个本地模型按需单独设置；填写后切换新模型时自动应用。',
    ),
    localCtxPlaceholder: tr(t, 'models.localCtxPlaceholder', '例如 128'),
    addLocalModelLabel: tr(t, 'models.addLocalModelLabel', '添加本地模型'),
    addLocalModelHint: tr(
      t,
      'models.addLocalModelHint',
      '手动输入模型名称并回车，随后会在上方列表出现（可设置上下文并切换）。',
    ),
    addLocalModelPlaceholder: tr(t, 'models.addLocalModelPlaceholder', '例如 qwen2.5:7b'),
  }
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
function oauthExpiryText(expiresAt: number | null | undefined, t: TFunc): string {
  if (expiresAt == null) return ''
  const local = new Date(expiresAt * 1000).toLocaleString()
  return tr(t, 'models.oauthExpiresAt', `有效期至 ${local}`).replace('{0}', local)
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
  filterCapability?: 'vision' | 'audio' | 'image_generation' | 'video_generation'
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
  // supports_vision=true 的模型，能力由用户在模型行内显式声明；生成类列表只列
  // supports_image_generation=true 的模型（注册表只有一个生成能力声明字段）。
  const filtered = selectableModels(models, filterCapability)
  const isGeneration =
    filterCapability === 'image_generation' || filterCapability === 'video_generation'
  // 触发器上的已保存值必须**脱离候选集**解析：若该模型的视觉开关当前是关的，
  // 它已不在候选集里，但用户实际配置就是它——显示成「未配置」会误导。
  const selected = Array.isArray(models)
    ? models.find(m => m.id === value && (!provider || m.provider === provider))
    : undefined
  const selectedMissing = !!value && !!selected && !filtered.includes(selected)
  const emptyText =
    placeholder || tr(t, 'models.visionNone', '跟随 Leader 模型（需支持图像理解）（推荐）')

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
                : filterCapability === undefined
                  ? '暂无可选模型（先在上方连接并选择服务商）'
                  : t('models.genCapabilityEmpty')}
            </div>
          )}
          {selectedMissing && (
            <div className="compact-select-empty" role="status">
              {isGeneration
                ? t('models.genBoundMissing', value)
                : `当前配置的 ${value} 未开启视觉能力，已不在候选列表中；若它正是你在用的模型，可用本区下方的「标记为支持视觉输入」把它加回。`}
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
  const TXT = makeTXT(t)
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
  /** 「连接测试」：用当前填写的地址与密钥探测可用模型。不传则不渲染 */
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
  const { t } = useLanguage()
  const TXT = makeTXT(t)
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
  const oauthExpiry = oauthExpiryText(oauthStatus?.expires_at, t)
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
type ModelsView = 'provider' | 'capabilities' | 'agents' | 'jev'

export function ModelsPage({
  onClose,
  onModelChanged,
  initialView = 'provider',
}: {
  onClose: () => void
  onModelChanged?: () => void
  initialView?: ModelsView
}) {
  const { t } = useLanguage()
  const TXT = makeTXT(t)
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
  const [feedback, setFeedback] = useState<{ ok: boolean; msg: string; kind?: 'detect' } | null>(
    null,
  )
  useEffect(() => {
    if (!feedback) return
    const timer = window.setTimeout(() => setFeedback(null), 2500)
    return () => window.clearTimeout(timer)
  }, [feedback])
  const [switchingModel, setSwitchingModel] = useState('')
  const switchingRef = useRef(false)
  const mountedRef = useRef(true)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])
  const [visionModel, setVisionModel] = useState('')
  const [visionProvider, setVisionProvider] = useState('')
  const [visionSaving, setVisionSaving] = useState(false)
  const [visionFeedback, setVisionFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  /** 「标记为支持视觉输入」进行中（行内开关移除后，这是唯一的自助出口） */
  const [visionMarking, setVisionMarking] = useState(false)
  // 本地 sherpa-onnx STT 状态（进入 custom tab 时一次性探测）
  const [sttLocalStatus, setSttLocalStatus] = useState<SttStatus | null>(null)
  const [sttModel, setSttModel] = useState('')
  const [sttProvider, setSttProvider] = useState('')
  const [sttSaving, setSttSaving] = useState(false)
  const [sttFeedback, setSttFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [ttsModel, setTtsModel] = useState('')
  const [ttsProvider, setTtsProvider] = useState('')
  const [ttsSaving, setTtsSaving] = useState(false)
  const [ttsFeedback, setTtsFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  const [voiceModel, setVoiceModel] = useState('')
  const [voiceProvider, setVoiceProvider] = useState('')
  const [voiceSaving, setVoiceSaving] = useState(false)
  const [voiceFeedback, setVoiceFeedback] = useState<{ ok: boolean; msg: string } | null>(null)
  // 图片 / 视频生成：tools-internal 的 image_generate / video_generate 直接读这两个
  // 绑定取凭证与模型，未绑定时工具明确报错（不静默发现 provider）。
  const [imageGenModel, setImageGenModel] = useState('')
  const [imageGenProvider, setImageGenProvider] = useState('')
  const [imageGenSaving, setImageGenSaving] = useState(false)
  const [imageGenFeedback, setImageGenFeedback] = useState<{ ok: boolean; msg: string } | null>(
    null,
  )
  const [videoGenModel, setVideoGenModel] = useState('')
  const [videoGenProvider, setVideoGenProvider] = useState('')
  const [videoGenSaving, setVideoGenSaving] = useState(false)
  const [videoGenFeedback, setVideoGenFeedback] = useState<{ ok: boolean; msg: string } | null>(
    null,
  )
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
  const [activeView, setActiveView] = useState<ModelsView>(initialView)
  const [hasKey, setHasKey] = useState(false)
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([])
  const [detecting, setDetecting] = useState(false)
  const [clearingKey, setClearingKey] = useState(false)
  const [savingKey, setSavingKey] = useState(false)
  const [detectedModels, setDetectedModels] = useState<ProviderModelBrief[]>([])
  const [filterInput, setFilterInput] = useState('')
  const [detectError, setDetectError] = useState<string | null>(null)
  const [detectSuccess, setDetectSuccess] = useState<string | null>(null)
  const detectRequestRef = useRef(0)
  useEffect(() => {
    detectRequestRef.current += 1
    setDetecting(false)
    setDetectError(null)
    setDetectSuccess(null)
    setFeedback(current => (current?.kind === 'detect' ? null : current))
  }, [provider, apiKey, baseUrl])
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

  useEffect(() => {
    setActiveView(initialView)
  }, [initialView])

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
          setTtsProvider(m.tts_provider || '')
          setSttModel(m.stt)
          setSttProvider(m.stt_provider || '')
          setVoiceModel(m.voice)
          setVoiceProvider(m.voice_provider || '')
          setImageGenModel(m.image_generation || '')
          setImageGenProvider(m.image_generation_provider || '')
          setVideoGenModel(m.video_generation || '')
          setVideoGenProvider(m.video_generation_provider || '')
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
  const viewGenerationRef = useRef(0)
  useEffect(() => {
    providerRef.current = provider
    viewGenerationRef.current += 1
  }, [provider])
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    let refreshRequest = 0
    void import('@tauri-apps/api/event')
      .then(({ listen }) =>
        listen<{ provider: string; model: string }>('model-metadata-updated', ({ payload }) => {
          if (disposed || payload.provider !== providerRef.current) return
          const request = ++refreshRequest
          void listModels()
            .then(list => {
              if (
                !disposed &&
                request === refreshRequest &&
                payload.provider === providerRef.current &&
                Array.isArray(list)
              )
                setAllModels(list)
            })
            .catch(() => {})
        }),
      )
      .then(stop => {
        if (disposed) stop()
        else unlisten = stop
      })
      .catch(() => {})
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])
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
  /** 行内偏好（精选/厂商默认）只存 localStorage、非 React 状态；写入后 bump 一次强制重渲染。 */
  const [, bumpPrefs] = useState(0)
  /** 正在保存图像生成能力开关的模型名（行内 loading 态，避免重复点击） */
  const [imageGenToggling, setImageGenToggling] = useState('')
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

  // ── 删除自定义模型实例（左栏条目删除图标 → 二次确认 → 落盘）──
  /** 待确认删除的实例；null = 弹窗关闭。同时承载删除中的 loading 态。 */
  const [pendingRemove, setPendingRemove] = useState<ProviderInfo | null>(null)
  const [removing, setRemoving] = useState(false)
  const [removeError, setRemoveError] = useState('')

  /**
   * 确认删除：整段移除 → 重新拉列表与配置态。
   *
   * 三个刻意的选择：
   * 1. **先落盘再动 UI**：与新建/编辑同一套真值观（磁盘是唯一真相），
   *    不做「前端先摘条目、失败再插回」的乐观更新——那种做法在失败时
   *    会让用户以为删成功了。
   * 2. **删的是当前选中项时主动切走**：否则右栏会渲染一个已不存在的
   *    provider，apiKey/模型列表全空而看不出原因。
   * 3. **后端返回 false 不当失败**：段本来就不存在时目标已达成，静默收敛。
   */
  const confirmRemove = async () => {
    if (!pendingRemove || removing) return
    const target = pendingRemove
    setRemoving(true)
    setRemoveError('')
    try {
      await removeCustomProvider(target.id)
      const [list, cfg] = await Promise.all([
        getSupportedProviders().catch(() => null),
        getCurrentConfig().catch(() => null),
      ])
      if (Array.isArray(list)) setProviders(sortProvidersStable(list))
      const configured = cfg?.configured_providers
      if (configured) setConfiguredProviders(configured)
      // 删的是当前选中项 → 收起表单回到未选中态，避免右栏指向已删除实例
      if (provider === target.id) {
        setFormOpen(false)
        handleProviderChange('')
      }
      setFeedback({ ok: true, msg: TXT.removeSuccess(instanceLabel(target)) })
      setPendingRemove(null)
      onModelChanged?.()
    } catch (e) {
      // 后端原文直接展示（与新建/编辑失败同一口径）：不掩盖、不改写
      setRemoveError(String(e))
    } finally {
      setRemoving(false)
    }
  }

  /** Esc 关闭删除确认：删除进行中不拦（避免用户以为取消了、实际还在删）。 */
  useEffect(() => {
    if (!pendingRemove || removing) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        setPendingRemove(null)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [pendingRemove, removing])

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
      openProviderView(created?.id || id, configured)
      // 保存成功后自动拉一次模型列表（sync=false：静默合并，不删既有条目）。
      // 此前自定义实例一律不自动拉取，用户建完中转站面对空列表，只能逐个手敲 id；
      // 拉取失败不阻塞创建结果，手动「刷新」与「+ 手动添加」仍是兜底路径。
      const newId = created?.id || id
      refreshProviderModels(newId, v.baseUrl, false)
        .then(res => {
          const models = res?.models
          if (!Array.isArray(models) || models.length === 0) return
          setDetectedModels(models)
          saveDetectedModels(newId, models)
          setFeedback({
            ok: true,
            msg: `${TXT.createSuccess}，已拉取 ${models.length} 个模型（来自中转站目录，可能含不可用条目）`,
          })
        })
        .catch(() => {
          /* 静默：失败不打扰，手动「刷新」入口仍在 */
        })
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
    const request = ++detectRequestRef.current
    const target = provider
    const isCurrent = () =>
      mountedRef.current && detectRequestRef.current === request && providerRef.current === target
    setDetecting(true)
    setDetectError(null)
    setDetectSuccess(null)
    setFeedback(null)
    setDetectedModels([])
    try {
      const models = await listProviderModels(apiKey.trim(), target, baseUrl.trim() || undefined)
      if (!isCurrent()) return
      if (!models?.length) throw new Error('API 未返回任何可用模型')
      setDetectedModels(models)
      saveDetectedModels(target, models)
      const msg = `已获取到 ${models.length} 个模型`
      setDetectSuccess(msg)
      setFeedback({ ok: true, msg, kind: 'detect' })
    } catch (e: unknown) {
      if (isCurrent()) setDetectError(friendlyIpcError(e, '检测失败'))
    } finally {
      if (isCurrent()) setDetecting(false)
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
      // 与协议无关：anthropic 兼容中转同样走 {base}/models，实测可用。
      refreshModels()
    } catch (e: any) {
      setFeedback({ ok: false, msg: friendlyIpcError(e, TXT.clearModelsFail) })
    } finally {
      setClearingModels(false)
    }
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
  }

  // ── 手动添加模型（仅 local 列表使用；basic 模型的真正添加走「连接后点击模型」）──
  const addModel = async () => {
    const name = inputVal.trim()
    if (!name) return
    const list = loadModels(provider)
    if (list.includes(name)) {
      setFeedback({ ok: false, msg: `模型「${name}」已在列表中` })
      return
    }
    list.push(name)
    saveModels(provider, list)
    setModels(list)
    setInputVal('')

    const effectiveKey = apiKey.trim() ? apiKey : ''
    if (!isLocal && !effectiveKey.trim() && !hasKey) {
      setFeedback({ ok: false, msg: TXT.apiKeyRequired })
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
        // 地址已落盘：同步 loadedBaseUrl，否则「接口地址已变更」警示条在地址
        // 持久化成功后仍残留，误导用户以为没生效
        if (resolvedBaseUrl) setLoadedBaseUrl(resolvedBaseUrl)
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

  /**
   * 行内偏好快照：精选清单与厂商默认整个列表只读一次（避免每行各解析一遍 JSON）。
   * 偏好在 localStorage，写入后由 bumpPrefs 触发重渲染刷新快照。
   */
  const pickedModelIds = new Set(getPickedModels(localStorage, provider) ?? [])
  const defaultModelId = getDefaultModel(localStorage, provider)

  /**
   * 行内「精选」开关：只写 localStorage 偏好（决定切换弹窗显示哪些模型），不发任何 IPC。
   * 眼睛开 = 该模型在精选清单里；无记录时全部为关态（此时弹窗仍显示全部）。
   */
  const toggleVisibleModel = (name: string) => {
    setModelVisible(localStorage, provider, name, !pickedModelIds.has(name))
    bumpPrefs(v => v + 1)
  }

  /**
   * 行内「设为厂商默认」：只写 localStorage 偏好，**绝不调用 switchModel / 任何 IPC**
   * （切换会写盘 + 插会话系统消息 + 触发一次付费探测，误触成本高）。再次点击取消默认。
   */
  const toggleDefaultModel = (name: string) => {
    setDefaultModel(localStorage, provider, defaultModelId === name ? '' : name)
    bumpPrefs(v => v + 1)
  }

  /** 模型当前生效的图像生成能力（本地行取值，与上下文窗口同源解析） */
  const rowImageGen = (name: string): boolean => {
    const brief = detectedModels.find(d => d.id === name)
    if (brief) return brief.supports_image_generation
    return (
      allModels.find(m => m.provider === provider && m.id === name)?.supports_image_generation ??
      false
    )
  }

  /**
   * 本服务商在用的模型条目（「标记为支持视觉输入」是否出现、以及标记谁，都由它决定）。
   *
   * 必须带 provider 归属校验：`currentModel` 是**全局生效模型**（get_current_config），
   * 用户切到别的服务商后它并不属于当前段 —— 拿它直接标记会把 A 厂商的模型写进 B 段。
   * 归属查不到（如该模型尚未进入本页列表）时不显示出口，宁可少一个按钮也不写错数据。
   */
  const currentModelEntry = allModels.find(m => m.provider === provider && m.id === currentModel)
  const currentModelSupportsVision = !!currentModelEntry?.supports_vision

  /**
   * 把当前模型一键标记为支持图像输入（写 providers.toml，来源标记为 user）。
   *
   * 行内视觉能力开关移除后，这是「候选列表里没有我的模型」的唯一自助出口：
   * 云端 / 中转模型的能力主要由自动探测写入，探测没认出来时用户仍能把它拉进
   * 图像理解候选列表。落盘为 user 来源后，自动探测不再覆盖该值。
   */
  const markCurrentModelVision = async () => {
    if (!currentModelEntry) return
    setVisionMarking(true)
    setVisionFeedback(null)
    try {
      await setModelSupportsVision(provider, currentModelEntry.id, true)
      const list = await listModels().catch(() => null)
      if (Array.isArray(list)) setAllModels(list)
      setVisionFeedback({ ok: true, msg: TXT.visionMarkDone(currentModelEntry.id) })
      setTimeout(() => setVisionFeedback(null), 2000)
    } catch (e: unknown) {
      setVisionFeedback({ ok: false, msg: friendlyIpcError(e, TXT.saveFailed) })
    } finally {
      setVisionMarking(false)
    }
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
      return
    }
    const v = Math.round(k * 1000)
    if (v < 1 || v > 10000000) {
      setFeedback({ ok: false, msg: '上下文窗口需在 1 ~ 10,000,000K 之间' })
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
  }

  /**
   * 行内切换模型的图像生成能力 —— 与「上下文窗口」同构的第二类模型元数据编辑。
   *
   * 这个开关是图片/视频生成绑定候选列表的来源：只有打开它的模型才会出现在
   * 绑定下拉里（注册表只有这一个生成能力声明字段）。落盘时标记来源为 user，
   * 此后自动探测不再覆盖（否则今天开、下次连接又被关掉）。
   *
   * 注：行内视觉能力开关已移除（视觉能力改由后端探测自动判定，方案 C）；
   * 后端命令 set_model_supports_vision 与 IPC 封装保留，仅无 UI 入口。
   */
  const toggleModelImageGen = async (name: string, next: boolean) => {
    setImageGenToggling(name)
    try {
      await setModelSupportsImageGeneration(provider, name, next)
      setFeedback({
        ok: true,
        msg: next ? t('models.imageGenMarkedOn', name) : t('models.imageGenMarkedOff', name),
      })
      const list = await listModels().catch(() => null)
      if (Array.isArray(list)) setAllModels(list)
    } catch (e: any) {
      setFeedback({ ok: false, msg: friendlyIpcError(e, t('models.genSaveFailed')) })
    } finally {
      setImageGenToggling('')
    }
  }

  const switchModel = async (name: string) => {
    if (switchingRef.current || editingCtxModel === name) return
    if (providersLoading || !providers.some(p => p.id === provider)) {
      setFeedback({ ok: false, msg: '服务商列表尚未加载完成' })
      return
    }
    const target = provider
    const viewGeneration = viewGenerationRef.current
    const isCurrent = () =>
      mountedRef.current &&
      providerRef.current === target &&
      viewGenerationRef.current === viewGeneration
    const resolvedBaseUrl = baseUrl.trim() || undefined
    const ctxArg =
      isLocal && localCtxWindow != null && !hasExplicitCtx(name) ? localCtxWindow : undefined
    switchingRef.current = true
    setSwitchingModel(name)
    setFeedback(null)
    try {
      if (apiKey.trim()) {
        await configureLlm(apiKey.trim(), name, target, resolvedBaseUrl, ctxArg)
      } else {
        await switchModelCmd(name, target, resolvedBaseUrl, ctxArg, 'default')
      }
      persistCurrentProvider(name)
      onModelChanged?.()
      if (!isCurrent()) return
      if (resolvedBaseUrl) setLoadedBaseUrl(resolvedBaseUrl)
      setCurrentModel(name)
      setFeedback({ ok: true, msg: `已切换到 ${name}` })
      void listModels()
        .then(list => {
          if (isCurrent() && Array.isArray(list)) setAllModels(list)
        })
        .catch(() => {})
    } catch (e: unknown) {
      if (isCurrent()) setFeedback({ ok: false, msg: friendlyIpcError(e, '切换失败') })
    } finally {
      switchingRef.current = false
      if (mountedRef.current) setSwitchingModel('')
    }
  }

  // ── 页面渲染 ──
  const loadingView = providersLoading ? (
    <div className="models-page-loading">正在加载服务商列表…</div>
  ) : null

  /* ── 页内反馈（六个反馈态共用一条「顶部居中」通道，见下方 .feedback-toast）──
     文案取**首个非空态**；成功色沿用原语义（任一项为 ok 即成功）。
     视觉与 island 统一：胶囊由共享组件 AppPill 渲染（app-pill.css 的 .app-pill） */
  const pageFeedbacks = [
    feedback,
    visionFeedback,
    sttFeedback,
    ttsFeedback,
    voiceFeedback,
    agentFeedback,
  ]
  const pageFeedback = pageFeedbacks.find(f => f?.msg) ?? null
  const pageFeedbackOk = pageFeedbacks.some(f => f?.ok)

  return (
    <div className="models-page-layout">
      {/* ── 左侧：两模块分组导航（模型提供商 / 自定义模型） ── */}
      <aside className="models-rail">
        <div className="models-rail-scroll">
          <div className="models-rail-group">
            <div className="models-rail-group-title">{TXT.railGroupProviders}</div>
            {providersLoading ? (
              <div className="models-rail-note">{TXT.railLoadingProviders}</div>
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
            <div className="models-rail-group-title">{TXT.railGroupCustom}</div>
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
                const displayName = instanceLabel(p)
                return (
                  /* 行容器而非单个 button：删除图标必须是独立可聚焦按钮，
                     嵌套在 <button> 内是非法 HTML 且点击会冒泡成「选中」。
                     主区域仍占满整行，点击热区与旧版一致。 */
                  <div
                    key={p.id}
                    className={['models-rail-item', isActive ? 'active' : '']
                      .filter(Boolean)
                      .join(' ')}
                  >
                    <button
                      type="button"
                      className="models-rail-item-main"
                      onClick={() => openProviderView(p.id)}
                      title={TXT.instanceIdTitle(p.id)}
                    >
                      <span
                        className={`models-rail-dot${isConfigured ? ' is-on' : ''}`}
                        title={isConfigured ? TXT.keyConfigured : TXT.keyNotConfigured}
                        aria-hidden="true"
                      />
                      <span className="models-rail-name">{displayName}</span>
                    </button>
                    <button
                      type="button"
                      className="models-rail-delete"
                      onClick={() => {
                        setRemoveError('')
                        setPendingRemove(p)
                      }}
                      title={TXT.removeInstanceLabel(displayName)}
                      aria-label={TXT.removeInstanceLabel(displayName)}
                    >
                      <IconTrash2 size={13} />
                    </button>
                  </div>
                )
              })}
            </div>
          </div>

          {/* 模块三「本地模型」：跑在自己机器上的服务，与云端提供商分开放，
              避免用户被引导到云厂商列表里找一个本机服务。 */}
          {!providersLoading && localProviders.length > 0 && (
            <div className="models-rail-group">
              <div className="models-rail-group-title">{TXT.railGroupLocal}</div>
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
            <div className="models-rail-group-title">{TXT.railGroupAutomation}</div>
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
                <span className="models-rail-name">{TXT.railGroupEnhancedJudge}</span>
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
            {TXT.toolbarCapabilities}
          </button>
          <button
            type="button"
            className={['models-toolbar-btn', activeView === 'agents' ? 'is-active' : '']
              .filter(Boolean)
              .join(' ')}
            onClick={() => setActiveView('agents')}
          >
            {TXT.toolbarAgents}
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
                  <Section title={isCustom ? TXT.customSectionTitle : TXT.providerSectionTitle}>
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
                          {hasKey ? TXT.keyConfigured : TXT.keyNotConfiguredShort}
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
                        /* 连接探测与协议无关：GET {base}/models + 按段类型发鉴权头。
                           曾以「Anthropic 协议没有 /v1/models」为由对 anthropic 实例不传
                           onTest，等于把入口关掉；实测中转站两条路由均返 200。 */
                        onTest={detectModels}
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
                              {/* 注：本分支 isCustom 恒 false，isAnthropicInstance 永假——
                                  官方/本地服务商的连接入口由 CustomModelForm 的 onTest 承担，
                                  此处保留原有结构，不为官方服务商新增按钮 */}
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

                    {detectSuccess && (
                      <div className="model-operation-status" role="status">
                        {detectSuccess}
                      </div>
                    )}
                    {detectError && (
                      <div className="detect-error" role="alert">
                        {detectError}
                      </div>
                    )}
                    {switchingModel && (
                      <div className="model-operation-status" role="status">
                        正在切换到 {switchingModel}…
                      </div>
                    )}

                    {isAnthropicInstance && (
                      <div className="text-caption hint-text">{TXT.anthropicModelListHint}</div>
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
                      currentModel ? TXT.currentModelOf(currentModel) : TXT.modelListDescEmpty
                    }
                    actions={
                      <>
                        {/* 手动添加对所有自定义实例开放；anthropic 实例用强调态呈现，
                             因为它的模型 id 多为中转自定义别名，未必在 /v1/models 里 */}
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
                        {/* 刷新入口对所有自定义实例开放（含 anthropic 兼容中转）：
                            后端 fetch_provider_models 与协议无关；曾按协议类型隐藏按钮，
                            等于关掉唯一批量入口，用户只能逐个手敲模型 id */}
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
                    {/* 热门模型快捷区：点击 = 把该关键词片段填进筛选框（子串匹配），
                        复用列表既有的 includes 筛选，不引入第二套过滤逻辑。 */}
                    <div className="models-hot-row">
                      <span className="models-hot-label">{TXT.hotModelsTitle}</span>
                      {HOT_MODELS.map(chip => (
                        <button
                          key={chip.match}
                          type="button"
                          className="models-hot-chip"
                          onClick={() => setFilterInput(chip.match)}
                        >
                          {chip.label}
                        </button>
                      ))}
                    </div>
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
                        return <div className="models-empty">{TXT.detectingModels}</div>
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
                                audio: brief?.supports_audio || info?.supports_audio || false,
                                image:
                                  brief?.supports_image_generation ||
                                  info?.supports_image_generation ||
                                  false,
                              }
                              const isPicked = pickedModelIds.has(name)
                              const isDefaultModel = defaultModelId === name
                              return (
                                <div
                                  key={name}
                                  className={'model-list-item' + (isActive ? ' active' : '')}
                                  role="button"
                                  tabIndex={0}
                                  aria-disabled={!!switchingModel}
                                  aria-busy={switchingModel === name}
                                  onClick={() => void switchModel(name)}
                                  onKeyDown={e => {
                                    if (e.target !== e.currentTarget) return
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
                                      <span className="model-go-badge" title={TXT.goGateway}>
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
                                    {/* 「精选」开关（原视觉能力开关的语义已改为精选/显示该模型）：
                                        只写 localStorage 偏好——某厂商有非空精选时，切换弹窗
                                        只显示这些模型；点击不触发任何 IPC。 */}
                                    <button
                                      type="button"
                                      className={`model-vision-toggle${isPicked ? ' is-on' : ''}`}
                                      aria-pressed={isPicked}
                                      aria-label={TXT.visibleToggleAria(
                                        isPicked
                                          ? t('models.capStateOn')
                                          : t('models.capStateOff'),
                                      )}
                                      title={
                                        isPicked
                                          ? TXT.visibleToggleTitleOn
                                          : TXT.visibleToggleTitleOff
                                      }
                                      onClick={e => {
                                        e.stopPropagation()
                                        toggleVisibleModel(name)
                                      }}
                                    >
                                      {isPicked ? <IconEye size={12} /> : <IconEyeOff size={12} />}
                                    </button>
                                    {/* 厂商默认：只写 localStorage 偏好（弹窗厂商卡片显示该默认模型），
                                        与「点行即切换」相互独立，绝不调用 switchModel。 */}
                                    <button
                                      type="button"
                                      className={`model-vision-toggle${isDefaultModel ? ' is-on' : ''}`}
                                      aria-pressed={isDefaultModel}
                                      aria-label={TXT.defaultToggleAria(
                                        isDefaultModel
                                          ? t('models.capStateOn')
                                          : t('models.capStateOff'),
                                      )}
                                      title={
                                        isDefaultModel
                                          ? TXT.defaultToggleTitleOn
                                          : TXT.defaultToggleTitleOff
                                      }
                                      onClick={e => {
                                        e.stopPropagation()
                                        toggleDefaultModel(name)
                                      }}
                                    >
                                      <IconStar size={12} />
                                    </button>
                                    {caps.audio && (
                                      <span className="model-badge" title={TXT.audioSupported}>
                                        <IconMic size={12} />
                                      </span>
                                    )}
                                    {/* 图像生成能力开关：行内模型元数据编辑，开关本身即状态
                                        （开启态 = 该模型进入图片/视频生成绑定候选列表；关闭态 = 掉出候选）。 */}
                                    <button
                                      type="button"
                                      className={`model-vision-toggle${caps.image ? ' is-on' : ''}`}
                                      aria-pressed={caps.image}
                                      aria-label={t(
                                        'models.imageGenToggleAria',
                                        caps.image
                                          ? t('models.capStateOn')
                                          : t('models.capStateOff'),
                                      )}
                                      title={
                                        caps.image
                                          ? t('models.imageGenToggleTitleOn')
                                          : t('models.imageGenToggleTitleOff')
                                      }
                                      disabled={imageGenToggling === name}
                                      onClick={e => {
                                        e.stopPropagation()
                                        void toggleModelImageGen(name, !caps.image)
                                      }}
                                    >
                                      <IconImage size={12} />
                                    </button>
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
                          const isPicked = pickedModelIds.has(m)
                          const isDefaultModel = defaultModelId === m
                          return (
                            <div
                              key={m}
                              className={'model-list-item' + (isActive ? ' active' : '')}
                              role="button"
                              tabIndex={0}
                              aria-disabled={!!switchingModel}
                              aria-busy={switchingModel === m}
                              onClick={() => void switchModel(m)}
                              onKeyDown={e => {
                                if (e.target !== e.currentTarget) return
                                if (e.key === 'Enter' || e.key === ' ') {
                                  e.preventDefault()
                                  void switchModel(m)
                                }
                              }}
                            >
                              <div className="model-list-name">{m}</div>
                              <div className="model-list-badges">
                                {/* 本地端点同样可精选（本地模型多的用户只挑常用的几个进切换弹窗）。 */}
                                <button
                                  type="button"
                                  className={`model-vision-toggle${isPicked ? ' is-on' : ''}`}
                                  aria-pressed={isPicked}
                                  aria-label={TXT.visibleToggleAria(
                                    isPicked ? t('models.capStateOn') : t('models.capStateOff'),
                                  )}
                                  title={
                                    isPicked
                                      ? TXT.visibleToggleTitleOn
                                      : TXT.visibleToggleTitleOff
                                  }
                                  onClick={e => {
                                    e.stopPropagation()
                                    toggleVisibleModel(m)
                                  }}
                                >
                                  {isPicked ? <IconEye size={12} /> : <IconEyeOff size={12} />}
                                </button>
                                {/* 本地端点同样可设为厂商默认（只写偏好，不触发切换/IPC）。 */}
                                <button
                                  type="button"
                                  className={`model-vision-toggle${isDefaultModel ? ' is-on' : ''}`}
                                  aria-pressed={isDefaultModel}
                                  aria-label={TXT.defaultToggleAria(
                                    isDefaultModel ? t('models.capStateOn') : t('models.capStateOff'),
                                  )}
                                  title={
                                    isDefaultModel
                                      ? TXT.defaultToggleTitleOn
                                      : TXT.defaultToggleTitleOff
                                  }
                                  onClick={e => {
                                    e.stopPropagation()
                                    toggleDefaultModel(m)
                                  }}
                                >
                                  <IconStar size={12} />
                                </button>
                                {/* 本地端点同样可挂图片生成模型（如本地多模态生成服务）：
                                     打开后该模型才进入图片/视频生成绑定候选列表。 */}
                                <button
                                  type="button"
                                  className={`model-vision-toggle${rowImageGen(m) ? ' is-on' : ''}`}
                                  aria-pressed={rowImageGen(m)}
                                  aria-label={t(
                                    'models.imageGenToggleAria',
                                    rowImageGen(m)
                                      ? t('models.capStateOn')
                                      : t('models.capStateOff'),
                                  )}
                                  title={
                                    rowImageGen(m)
                                      ? t('models.imageGenToggleTitleOn')
                                      : t('models.imageGenToggleTitleOff')
                                  }
                                  disabled={imageGenToggling === m}
                                  onClick={e => {
                                    e.stopPropagation()
                                    void toggleModelImageGen(m, !rowImageGen(m))
                                  }}
                                >
                                  <IconImage size={12} />
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
                                title={TXT.removeFromLocalList}
                                aria-label={TXT.removeFromLocalList}
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
                          label={TXT.localCtxLabel}
                          hint={TXT.localCtxHint}
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
                              placeholder={TXT.localCtxPlaceholder}
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
                    label={TXT.addLocalModelLabel}
                    hint={TXT.addLocalModelHint}
                    control={
                      <div className="compact-input-row input-row-spaced">
                        <input
                          className="compact-input input-flex"
                          value={inputVal}
                          onChange={e => setInputVal(e.target.value)}
                          placeholder={TXT.addLocalModelPlaceholder}
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
                          {TXT.addModelConfirm}
                        </Button>
                      </div>
                    }
                  />
                </div>
              )}

              {/* ═══════════ 图像音频模型：视觉 / 语音 / 朗读 / 生成（原「自定义能力」tab 内容） ═══════════ */}
              {activeView === 'capabilities' && (
                <>
                  {/* ── 云端图像理解模型 ── */}
                  <Section title={TXT.visionSection} description={TXT.visionSectionDesc}>
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
                          await setCapabilityBinding('vision', modelId, selectedProvider)
                          setVisionModel(modelId)
                          setVisionProvider(selectedProvider)
                          setVisionFeedback({ ok: true, msg: TXT.visionSaved })
                          setTimeout(() => setVisionFeedback(null), 2000)
                        } catch (e: any) {
                          setVisionFeedback({
                            ok: false,
                            msg: friendlyIpcError(e, TXT.saveFailed),
                          })
                        } finally {
                          setVisionSaving(false)
                        }
                      }}
                      t={t}
                    />
                    {visionFeedback && (
                      <div
                        className={`text-caption${visionFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {visionFeedback.msg}
                      </div>
                    )}
                    {/* 自助出口：行内视觉开关移除后，探测没认出来的模型在这里一键声明。
                        没有它，「候选列表里找不到我的模型」就是死路——中转站的自定义别名
                        几乎不可能被自动探测覆盖。 */}
                    {currentModelEntry && !currentModelSupportsVision && (
                      <div className="models-vision-mark">
                        <span className="text-caption">
                          {TXT.visionMarkHint(currentModelEntry.id)}
                        </span>
                        <Button
                          variant="default"
                          size="sm"
                          disabled={visionMarking}
                          onClick={() => void markCurrentModelVision()}
                        >
                          {TXT.visionMarkBtn}
                        </Button>
                      </div>
                    )}
                    {/* 图像理解生效路径如实呈现：默认「跟随 Leader」时讲清由谁处理，
                        Leader 不支持视觉时明确警示——否则用户无从知道截图识别已不可用
                        （视觉不可用时桌面操作会真实报错，这里提前把原因摆在配置处）。 */}
                    <div
                      className={`text-caption${!visionModel && leaderVisionModelId && !leaderSupportsVision ? ' text-danger' : ''}`}
                    >
                      {visionModel
                        ? TXT.visionExplicitPath(visionModel)
                        : leaderVisionModelId
                          ? leaderSupportsVision
                            ? TXT.visionFollowOk(leaderVisionModelId)
                            : TXT.visionFollowWarn(leaderVisionModelId)
                          : TXT.visionFollowUnknown}
                    </div>
                  </Section>

                  {/* ── 图片生成 ── */}
                  <Section
                    title={t('models.imageGenSection')}
                    description={t('models.imageGenSectionDesc')}
                  >
                    <VisionModelSelect
                      value={imageGenModel}
                      provider={imageGenProvider}
                      models={allModels}
                      filterCapability="image_generation"
                      placeholder={t('models.imageGenNone')}
                      showVisionIcons={false}
                      menuUp
                      onChange={async (modelId, selectedProvider) => {
                        setImageGenSaving(true)
                        setImageGenFeedback(null)
                        try {
                          // 原子写入：model 与 provider 一起落盘，杜绝
                          // 「新 model + 旧 provider」的半绑定中间态。
                          await setCapabilityBinding('image_generation', modelId, selectedProvider)
                          setImageGenModel(modelId)
                          setImageGenProvider(selectedProvider)
                          setImageGenFeedback({
                            ok: true,
                            msg: t('models.imageGenSaved'),
                          })
                          setTimeout(() => setImageGenFeedback(null), 2000)
                        } catch (e: any) {
                          setImageGenFeedback({
                            ok: false,
                            msg: friendlyIpcError(e, t('models.genSaveFailed')),
                          })
                        } finally {
                          setImageGenSaving(false)
                        }
                      }}
                      t={t}
                    />
                    {imageGenFeedback && (
                      <div
                        className={`text-caption${imageGenFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {imageGenFeedback.msg}
                      </div>
                    )}
                  </Section>

                  {/* ── 视频生成 ── */}
                  <Section
                    title={t('models.videoGenSection')}
                    description={t('models.videoGenSectionDesc')}
                  >
                    <VisionModelSelect
                      value={videoGenModel}
                      provider={videoGenProvider}
                      models={allModels}
                      filterCapability="video_generation"
                      placeholder={t('models.videoGenNone')}
                      showVisionIcons={false}
                      menuUp
                      onChange={async (modelId, selectedProvider) => {
                        setVideoGenSaving(true)
                        setVideoGenFeedback(null)
                        try {
                          await setCapabilityBinding('video_generation', modelId, selectedProvider)
                          setVideoGenModel(modelId)
                          setVideoGenProvider(selectedProvider)
                          setVideoGenFeedback({
                            ok: true,
                            msg: t('models.videoGenSaved'),
                          })
                          setTimeout(() => setVideoGenFeedback(null), 2000)
                        } catch (e: any) {
                          setVideoGenFeedback({
                            ok: false,
                            msg: friendlyIpcError(e, t('models.genSaveFailed')),
                          })
                        } finally {
                          setVideoGenSaving(false)
                        }
                      }}
                      t={t}
                    />
                    {videoGenFeedback && (
                      <div
                        className={`text-caption${videoGenFeedback.ok ? ' text-success' : ' text-danger'}`}
                      >
                        {videoGenFeedback.msg}
                      </div>
                    )}
                  </Section>

                  {/* ── 本地视觉模型（OCR / UI 元素检测）：随应用自动下载 ── */}
                  <Section title={TXT.localVisionSection} description={TXT.localVisionSectionDesc}>
                    {visionDl.status && (
                      <div className="models-dl-badges">
                        <span
                          className={`model-badge ${visionDl.status.ocrReady ? 'model-badge--ok' : ''}`}
                        >
                          {visionDl.status.ocrReady ? TXT.ocrReady : TXT.ocrNotReady}
                        </span>
                        <span
                          className={`model-badge ${visionDl.status.yoloReady ? 'model-badge--ok' : ''}`}
                        >
                          {visionDl.status.yoloReady ? TXT.yoloReady : TXT.yoloDisabled}
                        </span>
                      </div>
                    )}
                    {visionDl.status?.dir && (
                      <div className="text-caption hint-text">
                        {TXT.modelDirLabel}
                        {visionDl.status.dir}
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
                        <div className="text-caption hint-text">{TXT.downloadingHint}</div>
                      </>
                    ) : visionDl.error ? (
                      <>
                        <div className="detect-error">
                          {TXT.downloadFailed}
                          {visionDl.error}
                        </div>
                        <Button
                          variant="primary"
                          size="sm"
                          style={{ marginTop: 8 }}
                          onClick={visionDl.retry}
                        >
                          {TXT.retryDownload}
                        </Button>
                      </>
                    ) : visionDl.status === null ? (
                      <div className="text-caption hint-text">{TXT.detecting}</div>
                    ) : visionDl.status.missing.length > 0 ? (
                      <>
                        <div className="text-caption hint-text">
                          {visionDl.status.ocrReady
                            ? TXT.missingOptional(visionDl.status.missing.join('、'))
                            : TXT.missingRequired(visionDl.status.missing.length)}
                        </div>
                        <Button
                          variant="primary"
                          size="sm"
                          style={{ marginTop: 8 }}
                          onClick={visionDl.retry}
                        >
                          {TXT.downloadNow}
                        </Button>
                      </>
                    ) : (
                      <div className="text-caption hint-text">{TXT.screenReady}</div>
                    )}
                  </Section>

                  {/* ── 语音输入（STT）：云端优先，本地 sherpa-onnx 兜底 ── */}
                  <Section title={TXT.sttSection} description={TXT.sttSectionDesc}>
                    <VisionModelSelect
                      value={sttModel}
                      provider={sttProvider}
                      models={allModels}
                      filterCapability="audio"
                      placeholder={TXT.sttPlaceholder}
                      showVisionIcons={false}
                      menuUp
                      onChange={async (modelId, selectedProvider) => {
                        setSttSaving(true)
                        setSttFeedback(null)
                        try {
                          await setCapabilityBinding('stt', modelId, selectedProvider)
                          setSttModel(modelId)
                          setSttProvider(selectedProvider)
                          setSttFeedback({ ok: true, msg: TXT.sttSaved })
                          setTimeout(() => setSttFeedback(null), 2000)
                          probeStt()
                        } catch (e: any) {
                          setSttFeedback({ ok: false, msg: friendlyIpcError(e, TXT.saveFailed) })
                        } finally {
                          setSttSaving(false)
                        }
                      }}
                      t={t}
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
                              {TXT.sttNoMic}
                            </div>
                          )}
                          {!sttLocalStatus.model_dir && (
                            <div className="text-caption hint-text">
                              {TXT.sttLocalModelMissingCloudOk}
                            </div>
                          )}
                        </>
                      ) : sttLocalStatus.reason === 'no_microphone' ? (
                        <div className="text-caption hint-text models-warn">
                          <IconAlertTriangle size={12} className="icon-prefix" />
                          {TXT.sttNoMic}
                        </div>
                      ) : sttLocalStatus.reason?.startsWith('model_missing') ? (
                        <div>
                          <div className="text-caption hint-text">{TXT.sttDownloadHint}</div>
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
                            <div className="detect-error">
                              {TXT.downloadFailed}
                              {sttDl.error}
                            </div>
                          )}
                          <Button
                            variant="primary"
                            size="sm"
                            style={{ marginTop: 8 }}
                            loading={sttDl.downloading}
                            onClick={sttDl.start}
                          >
                            {sttDl.error ? TXT.retryDownload : TXT.downloadVoiceModel}
                          </Button>
                        </div>
                      ) : null)}
                  </Section>

                  {/* ── 文字转语音（TTS） ── */}
                  <Section title={TXT.ttsSection} description={TXT.ttsSectionDesc}>
                    <VisionModelSelect
                      value={ttsModel}
                      provider={ttsProvider}
                      models={allModels}
                      placeholder={TXT.ttsPlaceholder}
                      showVisionIcons={false}
                      menuUp
                      onChange={async (modelId, selectedProvider) => {
                        setTtsSaving(true)
                        setTtsFeedback(null)
                        try {
                          await setCapabilityBinding('tts', modelId, selectedProvider)
                          setTtsModel(modelId)
                          setTtsProvider(selectedProvider)
                          setTtsFeedback({ ok: true, msg: TXT.ttsSaved })
                          setTimeout(() => setTtsFeedback(null), 2000)
                        } catch (e: any) {
                          setTtsFeedback({ ok: false, msg: friendlyIpcError(e, TXT.saveFailed) })
                        } finally {
                          setTtsSaving(false)
                        }
                      }}
                      t={t}
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
                  <Section title={TXT.voiceSection} description={TXT.voiceSectionDesc}>
                    <VisionModelSelect
                      value={voiceModel}
                      provider={voiceProvider}
                      models={allModels}
                      placeholder={TXT.voicePlaceholder}
                      showVisionIcons={false}
                      menuUp
                      onChange={async (modelId, selectedProvider) => {
                        setVoiceSaving(true)
                        setVoiceFeedback(null)
                        try {
                          await setCapabilityBinding('voice', modelId, selectedProvider)
                          setVoiceModel(modelId)
                          setVoiceProvider(selectedProvider)
                          setVoiceFeedback({ ok: true, msg: TXT.voiceSaved })
                          setTimeout(() => setVoiceFeedback(null), 2000)
                        } catch (e: any) {
                          setVoiceFeedback({ ok: false, msg: friendlyIpcError(e, TXT.saveFailed) })
                        } finally {
                          setVoiceSaving(false)
                        }
                      }}
                      t={t}
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
                  <Section title={TXT.execSection} description={TXT.execSectionDesc}>
                    <VisionModelSelect
                      value={agentModels.exec}
                      models={allModels}
                      onChange={(m, provider) => void saveAgentModel('exec', m, provider)}
                      t={t}
                      placeholder={TXT.execFollowDefault}
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

        {/* ── 删除确认弹窗：与页面反馈同一套 portal 到 body 的做法（避免被
                models-rail 的滚动容器裁剪）。Esc / 点遮罩 = 取消。── */}
        {pendingRemove &&
          createPortal(
            <div
              className="models-confirm-mask"
              onClick={() => {
                if (!removing) setPendingRemove(null)
              }}
            >
              <div
                className="models-confirm"
                role="dialog"
                aria-modal="true"
                aria-label={TXT.removeInstanceTitle}
                onClick={e => e.stopPropagation()}
              >
                <div className="models-confirm-title">{TXT.removeInstanceTitle}</div>
                <div className="models-confirm-body">
                  {TXT.removeInstanceConfirm(instanceLabel(pendingRemove))}
                </div>
                {removeError && <div className="models-confirm-error">{removeError}</div>}
                <div className="models-confirm-actions">
                  <button
                    type="button"
                    className="models-confirm-btn"
                    onClick={() => setPendingRemove(null)}
                    disabled={removing}
                  >
                    {TXT.cancel}
                  </button>
                  <button
                    type="button"
                    className="models-confirm-btn is-danger"
                    onClick={() => void confirmRemove()}
                    disabled={removing}
                  >
                    {removing ? TXT.removing : TXT.confirmDelete}
                  </button>
                </div>
              </div>
            </div>,
            document.body,
          )}

        {/* ── 页内反馈：与 island 同一套胶囊视觉（共享层 ui/AppPill.tsx + app-pill.css），
               位置与层级仍归本页（顶部居中 / z 10000 / 挂在 body）── */}
        {pageFeedback &&
          createPortal(
            <div className="feedback-toast">
              {/* key 绑定文案：换一条时重新挂载，进场动画得以重放（与 island 同规格） */}
              <AppPill key={pageFeedback.msg} tone={pageFeedbackOk ? 'success' : 'error'} multiline>
                {pageFeedback.msg}
              </AppPill>
            </div>,
            document.body,
          )}
      </div>
    </div>
  )
}
