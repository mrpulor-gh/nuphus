import React, { useState, useRef, useEffect, useCallback, useMemo, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import type {
  ChatMessage,
  ChatReference,
  PendingImage,
  PendingFile,
  TimelineEntry,
} from '../../core/types'
import type { SecurityCheck } from '../../core/types'
import { convertFileSrc } from '@tauri-apps/api/core'

/// 文件系统路径 → 浏览器可访问 URL（Tauri asset protocol；截图等本地文件用）
function toAssetUrl(path: string | null | undefined): string | null {
  if (!path) return null
  if (/^(https?:\/\/|data:|asset:\/\/|tauri:\/\/)/i.test(path)) return path
  try {
    return convertFileSrc(path)
  } catch {
    return null
  }
}
import {
  getCurrentConfig,
  configureLlm,
  switchModel,
  getSupportedProviders,
  getContextLimit,
  getToolPermissions,
  hudUpdate,
  isLlmConfigured,
  getReasoningEffort,
  setReasoningEffort,
  listModels,
  getEffectiveModel,
  setRelation as persistRelationToBackend,
} from '../lib/api'
import type { ProviderInfo, ModelInfo } from '../lib/api'
import { WelcomeScreen } from './WelcomeScreen'
import { OnboardingModal } from './OnboardingModal'
import { SessionDivider } from './SessionDivider'
import { PauseOverlay } from './PauseOverlay'
import { ChatInputBar } from './ChatInputBar'
import { VideoProgressBadge } from './VideoProgressBadge'
import ExternalAgentsStatusBar from './ExternalAgentsStatusBar'
import SessionRail from './SessionRail'
import { loadRelation } from '../lib/relation'
import {
  IconCopy,
  IconCheck,
  IconFolder,
  IconX,
  IconWorkflow,
  IconHistory,
  IconWrench,
  IconFile,
  IconBrain,
  IconPalette,
  IconShield,
  IconBrowser,
  IconSparkles,
  IconSquare,
  IconGrid,
  IconChevronUp,
  IconChevronDown,
} from '../../ui/Icons'
import { RatingModal } from '../layout/ExecutionTraceFloating'
import { MoodFace } from '../../ui/MoodFace'
import { useLanguage } from '../../locales'
import { NuphusLogo } from '../../ui/NuphusLogo'
import type { MoodState } from '../../ui/MoodFace'
import '../../styles/chat.css'
import { StatusBar } from '../layout/StatusBar'
import { SecurityPrompt } from '../layout/SecurityPrompt'
import { Button, IconButton } from '../../ui/Button'
import MarkdownContent from './MarkdownContent'
import { PreviewOverlay } from './PreviewOverlay'
function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
}

interface ChatPanelProps {
  messages: ChatMessage[]
  isProcessing: boolean
  onSend: (input: string, images?: string[], references?: ChatReference[]) => void
  startupStats: { tools: number; memories: number }
  onGracefulStop?: () => void
  onInterrupt?: () => void
  onRetry?: (input: string) => void
  focusSignal?: number
  onNewChat?: () => void
  /** Session Rail 切换/新建成功后：重拉 get_chat_history 替换气泡 */
  onChatReplaced?: () => void
  /** 欢迎页「继续对话」：先调后端 resume_latest_session 装镜像再渲染完整历史 */
  onResumeLast?: () => void
  /** +号菜单：打开教导原则弹窗 */
  onOpenPrinciples?: () => void
  /** +号菜单：打开关系标注弹窗 */
  onOpenAnnotations?: () => void
  tokenUsage?: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  goalType?: { type: string; label: string; confidence: number } | null
  security?: SecurityCheck | null
  pauseState?: { actionId: string } | null
  onContinue?: (actionId: string) => void
  onAppendInstruction?: (actionId: string, instruction: string) => void
  onTerminate?: (actionId: string) => void
  onApproveSecurity?: (id: string) => void
  onRejectSecurity?: (id: string) => void
  mood?: MoodState
  modelName?: string
  mainTokenUsage?: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  execTokenUsage?: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  totalDurationMs?: number
  totalCalls?: number
  contextLimit?: number
  onModelChanged?: () => void
  refineState?: { usagePercent: number; totalLimit: number } | null
  pendingRefine: { usagePercent: number; totalLimit: number; skippedTurns: number } | null
  setPendingRefine: React.Dispatch<
    React.SetStateAction<{ usagePercent: number; totalLimit: number; skippedTurns: number } | null>
  >
  onRefine?: () => void
  onSkipRefine?: () => void
  /** 提炼执行中（全局）：驱动提炼中全屏遮罩（弹窗路径与 refine-pending-btn 路径统一） */
  refining?: boolean
  setRefining?: (v: boolean) => void
  /** 手动关闭「提炼中」弹窗/遮罩：复位提炼 UI + 追踪 refs（后台提炼不中断，
   *  完成后 session_refined / refine_failed 照常落地）。缺省退化为仅收起遮罩 */
  onDismissRefine?: () => void
  onOpenPalette?: () => void
  onCommand?: (id: string) => void
  mode?: string
  onSetMode?: (mode: string) => void
  onManageCustomAgents?: () => void
  onManageExternalAgents?: () => void
  onToggleWorkAgentMode?: () => Promise<void>
  onRate?: (
    name: string,
    rating: number,
    comment: string,
    saveAsStrategy: boolean,
    userQuestion?: string,
    assistantContent?: string,
  ) => void
  /** 气泡执行回溯：点击打开执行面板显示该轮历史执行过程（traceItems） */
  onShowExecTrace?: (trace: TimelineEntry[]) => void
  /** Leader 暂停是否禁用（workflow 运行时） */
  isWorkflowRunning?: boolean
  /** 桌面工具箱（Ctrl+U）显示状态与切换（workflow 模式输入栏按钮） */
  showDesktopToolbar?: boolean
  onToggleDesktopToolbar?: () => void
}

export function ChatPanel({
  messages,
  isProcessing,
  onSend,
  onGracefulStop,
  onInterrupt,
  onRetry,
  focusSignal,
  onNewChat,
  onChatReplaced,
  onResumeLast,
  onOpenPrinciples,
  onOpenAnnotations,
  tokenUsage,
  goalType,
  security,
  pauseState,
  onContinue,
  onAppendInstruction,
  onTerminate,
  onApproveSecurity,
  onRejectSecurity,
  mood,
  modelName,
  mainTokenUsage,
  execTokenUsage,
  totalDurationMs,
  totalCalls,
  contextLimit,
  onModelChanged,
  refineState,
  pendingRefine,
  setPendingRefine,
  onRefine,
  onSkipRefine,
  refining,
  setRefining,
  onDismissRefine,
  onOpenPalette,
  onCommand,
  mode,
  onSetMode,
  onManageCustomAgents,
  onManageExternalAgents,
  onToggleWorkAgentMode,
  startupStats,
  isWorkflowRunning,
  showDesktopToolbar,
  onToggleDesktopToolbar,
  onRate,
  onShowExecTrace,
}: ChatPanelProps) {
  const { t } = useLanguage()
  const [pauseMode, setPauseMode] = useState<'menu' | 'preparing' | 'input'>('menu')
  const [appendInput, setAppendInput] = useState('')
  const [pauseSubmitting, setPauseSubmitting] = useState(false)
  const [selectedOption, setSelectedOption] = useState(0)
  const [pausing, setPausing] = useState(false)
  const [showPauseLocal, setShowPauseLocal] = useState(false)

  // ── 文件预览覆盖层（AI 回复路径点击） ──
  const [previewPath, setPreviewPath] = useState<string | null>(null)

  const [pauseActionBusy, setPauseActionBusy] = useState(false)
  // ── 点评弹窗 ──
  const [ratingMsg, setRatingMsg] = useState<{
    id: string
    content: string
    userQuestion?: string
  } | null>(null)
  // Pending image data URLs — attached to next user message
  const [pendingImages, setPendingImages] = useState<PendingImage[]>([])
  // Lightbox for image click-to-zoom
  const [lightboxUrl, setLightboxUrl] = useState<string | null>(null)
  // Pending file paths (drag-drop) — shown in ReferenceBar, attached to next user message
  const [pendingFiles, setPendingFiles] = useState<PendingFile[]>([])
  // Pending resource references (skill/knowledge/workflow) — attached to next user message
  const [pendingReferences, setPendingReferences] = useState<ChatReference[]>([])

  // Load tool permissions for WORKFLOW mode check
  const [toolPermissions, setToolPermissions] = useState<
    { file_access: boolean; web_search: boolean; system_automation: boolean } | undefined
  >()
  useEffect(() => {
    getToolPermissions()
      .then(r => {
        if (r && typeof r === 'string') {
          try {
            const data = JSON.parse(r)
            setToolPermissions({
              file_access: data.file_access ?? data.fileAccess ?? true,
              web_search: data.web_search ?? data.webSearch ?? true,
              system_automation: data.system_automation ?? data.systemAutomation ?? false,
            })
          } catch {
            /* ignore */
          }
        }
      })
      .catch(() => {})
  }, [])

  const handlePauseChoice = useCallback(
    async (choice: string) => {
      if (pauseActionBusy) return

      // 本地暂停对话框（Case 1）已移除——执行控制入口不再触发本地暂停菜单；
      // 仅保留后端主动暂停（workflow/系统）的决策处理。
      if (showPauseLocal) {
        setPauseActionBusy(true)
        try {
          switch (choice) {
            case 'continue':
              setShowPauseLocal(false)
              break
            case 'append':
              setPauseMode('preparing')
              break
            case 'terminate':
              setShowPauseLocal(false)
              onGracefulStop?.()
              break
            case 'interrupt':
              setShowPauseLocal(false)
              onInterrupt?.()
              break
          }
        } finally {
          setPauseActionBusy(false)
        }
        return
      }

      // Case 2: Backend-initiated pause
      if (!pauseState) return
      setPauseActionBusy(true)
      try {
        switch (choice) {
          case 'continue':
            await onContinue?.(pauseState.actionId)
            break
          case 'append':
            setPauseMode('preparing')
            break
          case 'terminate':
            await onTerminate?.(pauseState.actionId)
            break
          case 'interrupt':
            onInterrupt?.()
            break
        }
      } finally {
        setPauseActionBusy(false)
      }
    },
    [
      pauseState,
      showPauseLocal,
      pauseActionBusy,
      onContinue,
      onTerminate,
      onInterrupt,
      onGracefulStop,
    ],
  )

  const handleSubmitAppend = useCallback(async () => {
    if (!pauseState || !appendInput.trim() || pauseSubmitting) return
    setPauseSubmitting(true)
    try {
      await onAppendInstruction?.(pauseState.actionId, appendInput)
      setShowPauseLocal(false)
      setPauseMode('menu')
      setAppendInput('')
    } finally {
      setPauseSubmitting(false)
    }
  }, [pauseState, appendInput, pauseSubmitting, onAppendInstruction])

  // Keyboard navigation for pause menu
  useEffect(() => {
    if (!pauseState && !showPauseLocal) return
    const handler = (e: KeyboardEvent) => {
      if (pauseMode === 'menu' || pauseMode === 'preparing') {
        if (e.key === 'ArrowDown') {
          e.preventDefault()
          setSelectedOption(prev => Math.min(prev + 1, 3))
        } else if (e.key === 'ArrowUp') {
          e.preventDefault()
          setSelectedOption(prev => Math.max(prev - 1, 0))
        } else if (e.key === 'Enter') {
          e.preventDefault()
          const choices = ['continue', 'append', 'terminate', 'interrupt']
          handlePauseChoice(choices[selectedOption])
        } else if (e.key === 'Escape') {
          e.preventDefault()
          if (pauseMode === 'preparing') {
            setPauseMode('menu')
          } else if (showPauseLocal) {
            setShowPauseLocal(false)
          }
        }
      } else if (pauseMode === 'input') {
        if (e.key === 'Escape') {
          e.preventDefault()
          setPauseMode('menu')
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [pauseState, showPauseLocal, pauseMode, selectedOption, handlePauseChoice])

  // Reset pause modal state when pauseState changes (but not if appending)
  useEffect(() => {
    if (pauseState) {
      if (pauseMode === 'preparing' || pauseMode === 'input') return
      setPauseMode('menu')
      setAppendInput('')
      setPauseSubmitting(false)
      setSelectedOption(0)
    }
  }, [pauseState, pauseMode])

  // When pause is cleared (setPauseState(null)), reset all pause UI state
  useEffect(() => {
    if (!pauseState) {
      setPauseMode('menu')
      setAppendInput('')
      setPauseSubmitting(false)
      setSelectedOption(0)
    }
  }, [pauseState])

  // Clear local pause when backend responds with pauseState
  useEffect(() => {
    if (pauseState && showPauseLocal) {
      setShowPauseLocal(false)
    }
  }, [pauseState, showPauseLocal])

  // Preparing timeout: transition to input after backend pause is received
  useEffect(() => {
    if (pauseMode !== 'preparing') return
    if (!showPauseLocal && !pauseState) return
    const timer = setTimeout(() => setPauseMode('input'), 800)
    return () => clearTimeout(timer)
  }, [pauseMode, showPauseLocal, pauseState])

  // Allow Escape to cancel preparing state and return to menu
  useEffect(() => {
    if (!pauseState || pauseMode !== 'preparing') return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        setPauseMode('menu')
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [pauseState, pauseMode])

  const [input, setInput] = useState('')
  const [refineSelected, setRefineSelected] = useState(0) // 0=refine, 1=skip
  const [showRefineConfirm, setShowRefineConfirm] = useState(false)
  // 分母缺失（contextLimit 0/未知）→ 保持 0（前端消费端显示 "--"），不伪装 128000
  const [contextTotal, setContextTotal] = useState(contextLimit || 0)
  useEffect(() => {
    if (contextLimit != null && contextLimit > 0) setContextTotal(contextLimit)
  }, [contextLimit])
  const [modelLabel, setModelLabel] = useState('')
  const [relation, setRelation] = useState(loadRelation)
  const [copiedMsgId, setCopiedMsgId] = useState<string | null>(null)
  const [dirOpen, setDirOpen] = useState(false)
  const [modelOpen, setModelOpen] = useState(false)
  const [skillsOpen, setSkillsOpen] = useState(false)
  const [switchingId, setSwitchingId] = useState<string | null>(null)

  // ── Slash Commands ──
  const SLASH_ITEMS = useMemo(
    () => [
      { id: 'new-chat', label: '/new', desc: t('slash.new'), category: '' },
      { id: 'project', label: '/project', desc: t('slash.project'), category: '' },
      { id: 'models', label: '/models', desc: t('slash.models'), category: '' },
      { id: 'themes', label: '/themes', desc: t('slash.themes'), category: '' },
      { id: 'security', label: '/security', desc: t('slash.security'), category: '' },
      { id: 'browser', label: '/browser', desc: t('slash.browser'), category: '' },
      { id: 'memories', label: '/memories', desc: t('slash.memories'), category: '' },
      { id: 'workflows', label: '/workflow', desc: t('slash.workflow'), category: '' },
      { id: 'skills', label: '/skills', desc: t('slash.skills'), category: '' },
      { id: 'knowledge', label: '/knowledge', desc: t('slash.knowledge'), category: '' },
      { id: 'soul', label: '/soul', desc: t('slash.soul'), category: '' },
      { id: 'force-reset', label: '/reset', desc: t('slash.reset'), category: '' },
      { id: 'help', label: '/help', desc: t('slash.help'), category: '' },
      { id: 'snake-game', label: '/snake', desc: t('cmd.snakeGameDesc'), category: '' },
    ],
    [t],
  )
  const SLASH_ICONS: Record<string, ReactNode> = {
    workflows: <IconWorkflow size={14} />,
    memories: <IconHistory size={14} />,
    skills: <IconWrench size={14} />,
    knowledge: <IconFile size={14} />,
    models: <IconBrain size={14} />,
    themes: <IconPalette size={14} />,
    project: <IconFolder size={14} />,
    security: <IconShield size={14} />,
    browser: <IconBrowser size={14} />,
    soul: <IconSparkles size={14} />,
    'new-chat': <IconSquare size={14} />,
    'force-reset': <IconX size={14} />,
    help: <IconCopy size={14} />,
    'snake-game': <IconGrid size={14} />,
  }
  const [cmdOpen, setCmdOpen] = useState(false)
  const [cmdQuery, setCmdQuery] = useState('')
  const [cmdIdx, setCmdIdx] = useState(0)
  // ── Resource picker (triggered by /skills, /knowledge, /workflow slash commands) ──
  const [resPickerOpen, setResPickerOpen] = useState(false)
  const [resPickerType, setResPickerType] = useState<ChatReference['type'] | null>(null)
  type ResItem = { id: string; label: string; desc?: string }
  const [resItems, setResItems] = useState<ResItem[]>([])
  const [resLoading, setResLoading] = useState(false)
  const [resError, setResError] = useState<string | null>(null)
  const [resIdx, setResIdx] = useState(0)
  const resItemRefs = useRef<(HTMLDivElement | null)[]>([])
  const filteredSlash = useMemo(
    () =>
      cmdQuery
        ? SLASH_ITEMS.filter(i => i.label.toLowerCase().includes(cmdQuery.toLowerCase()))
        : SLASH_ITEMS,
    [cmdQuery, SLASH_ITEMS],
  )
  const cmdItemRefs = useRef<(HTMLDivElement | null)[]>([])
  useEffect(() => {
    const el = cmdItemRefs.current[cmdIdx]
    if (el) el.scrollIntoView({ block: 'nearest' })
  }, [cmdIdx])

  // ── Hints ──
  const HINTS = useMemo(
    () => [
      t('input.placeholder'),
      t('input.hint.mobile'),
      t('input.hint.shortcuts'),
      t('input.hint.desktop'),
      t('input.hint.workflow'),
    ],
    [t],
  )
  const [hintIndex, setHintIndex] = useState(0)
  const [hintFade, setHintFade] = useState(true)
  useEffect(() => {
    const t = setInterval(() => {
      setHintFade(false)
      setTimeout(() => {
        setHintIndex(i => (i + 1) % HINTS.length)
        setHintFade(true)
      }, 500)
    }, 30000)
    return () => clearInterval(t)
  }, [HINTS.length])

  const [projectDir, setProjectDir] = useState(() => {
    try {
      return localStorage.getItem('nuphus_project_dir') || ''
    } catch {
      return ''
    }
  })
  const [dirInput, setDirInput] = useState('')
  const [dirBookmarks, setDirBookmarks] = useState<{ label: string; path: string }[]>(() => {
    try {
      return JSON.parse(localStorage.getItem('nuphus_projects') || '[]')
    } catch {
      return []
    }
  })

  const [savedConfigs, setSavedConfigs] = useState<
    { id: string; label: string; model: string; provider: string; baseUrl: string }[]
  >([])
  const [allProviders, setAllProviders] = useState<ProviderInfo[]>([])
  const [allModels, setAllModels] = useState<ModelInfo[]>([])
  // 当前推理深度（null = 提供商默认），provider 由当前配置解析（见 loadEffortContext）
  const [effort, setEffort] = useState<string | null>(null)
  const [currentProvider, setCurrentProvider] = useState('')
  const [showOnboarding, setShowOnboarding] = useState(false)
  const loadSavedConfigs = async () => {
    const providers = (await getSupportedProviders().catch(() => [])) || []
    setAllProviders(providers)

    const activeCfg = await getCurrentConfig().catch(() => null)
    // 所有在 config.toml 中有 API key 的 provider 列表
    const configuredProviders: string[] = activeCfg?.configured_providers || []

    const configs: {
      id: string
      label: string
      model: string
      provider: string
      baseUrl: string
    }[] = []
    const seen = new Set<string>()

    providers.forEach(p => {
      // 只显示：有 key 的 provider、custom、local
      const hasApiKey = configuredProviders.includes(p.id) || p.id === 'custom' || p.id === 'local'
      if (!hasApiKey) return
      // 读取该 provider 持久化的当前 model,无则跳过
      let currentModel = ''
      try {
        currentModel = localStorage.getItem(`nuphus_current_model_${p.id}`) || ''
      } catch {
        /* localStorage 读取失败按未配置处理 */
      }
      if (!currentModel) return
      const key = `${p.id}::${currentModel}`
      if (seen.has(key)) return
      seen.add(key)
      configs.push({
        id: key,
        label: p.name,
        model: currentModel,
        provider: p.id,
        baseUrl: p.base_url,
      })
    })
    return configs
  }

  // ── 推理深度：从当前配置解析 provider，加载已配置值 + 模型元数据（支持的级别）──
  // 注意：本函数只负责 provider/effort 上下文，禁止写 modelLabel——
  // modelLabel 唯一数据源是 getEffectiveModel(mode)（getCurrentConfig 返回 config.toml
  // 根模型，不感知 mode，异步覆盖会把输入框显示打回默认模型）。
  const loadEffortContext = useCallback(async () => {
    const cfg = await getCurrentConfig().catch(() => null)
    const provider = cfg?.provider || ''
    setCurrentProvider(provider)
    if (provider) {
      try {
        const e = await getReasoningEffort(provider)
        setEffort(e ?? null)
      } catch {
        setEffort(null)
      }
    } else {
      setEffort(null)
    }
    try {
      const list = await listModels()
      if (Array.isArray(list)) setAllModels(list)
    } catch {
      /* 模型元数据加载失败时保持现状（入口隐藏） */
    }
  }, [])

  // ── 模型切换：点击卡片本身 → 切换到卡片当前显示的模型（所见即所得，切后自动关弹窗）──
  const switchConfig = useCallback(
    async (
      cfg: { id: string; label: string; model: string; provider: string; baseUrl: string },
      closeAfter: boolean,
    ) => {
      if (switchingId) return
      setSwitchingId(cfg.id)
      try {
        const prov = allProviders.find(p => p.id === cfg.provider)
        const resolvedUrl = prov?.base_url || ''
        // provider-driven: switch_model reads key from config.toml, no key param
        // 按当前 mode 写入对应 agent 模型配置（Leader/Workflow/Custom 联动）
        await switchModel(cfg.model, cfg.provider, resolvedUrl, undefined, mode)
        const limit = await getContextLimit()
        if (limit != null && limit > 0) setContextTotal(limit)
        onModelChanged?.()
        setModelLabel(cfg.model)
        // 同步持久化该 provider 的当前 model,保持与 /models 切换一致
        try {
          localStorage.setItem(`nuphus_current_model_${cfg.provider}`, cfg.model)
        } catch {
          /* localStorage 写入失败不阻塞切换流程 */
        }
        // 本地同步 savedConfigs（轮播切换后卡片立即显示新模型名 + ✓）
        setSavedConfigs(prev =>
          prev.map(c =>
            c.provider === cfg.provider
              ? { ...c, model: cfg.model, id: `${cfg.provider}::${cfg.model}` }
              : c,
          ),
        )
        await new Promise(r => setTimeout(r, 300))
      } catch {
        /* ignore */
      }
      setSwitchingId(null)
      if (closeAfter) setModelOpen(false)
    },
    [switchingId, allProviders, mode, onModelChanged],
  )

  // ── 上下按钮模型浏览：仅预览不切换，直接换显示的模型名（无翻转动画，高效直给）。
  //    不调用 switch_model、不写 localStorage——真正切换仍靠点击卡片本身确认。
  //    peekModels: provider → 正在预览的模型 id（关弹窗即清空，避免下次打开残留）
  const [peekModels, setPeekModels] = useState<Record<string, string>>({})
  useEffect(() => {
    if (!modelOpen) setPeekModels({})
  }, [modelOpen])

  const peekSwitch = useCallback(
    (
      cfg: { id: string; label: string; model: string; provider: string; baseUrl: string },
      dir: 1 | -1,
    ) => {
      const models = allModels.filter(m => m.provider === cfg.provider)
      if (models.length <= 1) return
      // 浏览基准 = 当前显示中的模型（含预览态），非已保存配置
      const displayed = peekModels[cfg.provider] || cfg.model
      const idx = models.findIndex(m => m.id === displayed)
      if (idx < 0) return
      const next = models[(idx + dir + models.length) % models.length]
      if (!next || next.id === displayed) return
      setPeekModels(prev => ({ ...prev, [cfg.provider]: next.id }))
    },
    [allModels, peekModels],
  )

  // 切换推理深度：写入 config.toml + 触发 Runtime 重建（后端已就绪，前端无需额外刷新）
  const handleEffortChange = useCallback(
    async (next: string | null) => {
      if (!currentProvider) return
      try {
        await setReasoningEffort(currentProvider, next)
        setEffort(next)
      } catch (e) {
        console.error('[Effort] set failed:', e)
      }
    },
    [currentProvider],
  )

  const scrollRef = useRef<HTMLDivElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const overlayRef = useRef<HTMLDivElement>(null)
  // Reload relation config on mount
  useEffect(() => {
    setRelation(loadRelation())
    // 启动迁移：把历史用户 localStorage 里已有的称呼持久化到后端 relation.json，
    // 保证老用户升级后手机端 /identity 立即拿到用户定义的称呼（桌面端未发消息时也生效）。
    void persistRelationToBackend(loadRelation()).catch(() => {})
    // modelLabel 不在此初始化：统一由下方「mode 生效模型」effect 负责（mode 感知）
    loadSavedConfigs().then(setSavedConfigs)
    void loadEffortContext()
  }, [])

  // 首次启动引导检测
  useEffect(() => {
    const done = localStorage.getItem('nuphus_onboarding_done')
    if (done === 'true') return
    isLlmConfigured()
      .then(ok => {
        if (ok) {
          localStorage.setItem('nuphus_onboarding_done', 'true')
        } else {
          setShowOnboarding(true)
        }
      })
      .catch(() => setShowOnboarding(true))
  }, [])

  // ── modelLabel 唯一权威数据源：当前 mode 的生效模型（后端 effective_model 单点解析）──
  // 触发时机：挂载 / mode 切换 / session_info 推送（modelName 变化，如模型切换广播）。
  // 禁止直接用 modelName（全局 runtime 模型，非 leader mode 下与 mode 生效模型不一致）
  // 或 getCurrentConfig（config.toml 根模型锚点）写入 modelLabel。
  useEffect(() => {
    getEffectiveModel(mode || 'leader')
      .then(m => {
        if (m) setModelLabel(m)
      })
      .catch(() => {
        if (modelName) setModelLabel(modelName)
      })
    void loadEffortContext()
  }, [mode, modelName, loadEffortContext])

  // 当前模型声明的可选推理深度（来自内置 ModelDef 元数据；空 = 不支持配置，隐藏入口）
  const currentModelEfforts = useMemo(() => {
    const id = modelLabel || modelName || ''
    return allModels.find(m => m.id === id)?.reasoning_efforts ?? []
  }, [allModels, modelLabel, modelName])

  // 当前模型的默认推理强度（未配置时生效；null = 无声明，UI 显示「默认」）
  const currentModelDefaultEffort = useMemo(() => {
    const id = modelLabel || modelName || ''
    return allModels.find(m => m.id === id)?.default_effort ?? null
  }, [allModels, modelLabel, modelName])

  // Reload latest config when quick-switch modal opens
  useEffect(() => {
    if (!modelOpen) return
    // 勾选态跟随当前 mode 的生效模型（非 config.toml 根模型）
    getEffectiveModel(mode || 'leader')
      .then(m => {
        if (m) setModelLabel(m)
      })
      .catch(() => {})
    loadSavedConfigs().then(setSavedConfigs)
  }, [modelOpen])

  // Focus input when focusSignal changes
  useEffect(() => {
    if (focusSignal && focusSignal > 0) {
      textareaRef.current?.focus()
    }
  }, [focusSignal])

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    requestAnimationFrame(() => {
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' })
    })
  }, [messages])

  const autoResize = useCallback(() => {
    const ta = textareaRef.current
    if (!ta) return
    ta.style.height = 'auto'
    ta.style.height = Math.min(ta.scrollHeight, 240) + 'px'
  }, [])

  // Listen for workflow export-to-chat events
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail
      const text = detail?.text
      const mode = detail?.mode
      if (text) {
        setInput(prev => (prev ? prev + '\n' + text : text))
        autoResize()
      }
      if (mode && onSetMode) {
        onSetMode(mode)
      }
    }
    window.addEventListener('nuphus:append-to-chat', handler)
    return () => window.removeEventListener('nuphus:append-to-chat', handler)
  }, [])

  // Reset refining and selection state when refine modal closes
  useEffect(() => {
    if (!refineState) {
      setRefining?.(false)
      setRefineSelected(0)
    }
  }, [refineState, setRefining])

  // Refine modal keyboard nav: ↑↓ select + Enter confirm + Esc skip
  useEffect(() => {
    if (!refineState || refining) return
    const el = overlayRef.current
    if (!el) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setRefineSelected(prev => (prev > 0 ? prev - 1 : 1))
      } else if (e.key === 'ArrowDown') {
        e.preventDefault()
        setRefineSelected(prev => (prev < 1 ? prev + 1 : 0))
      } else if (e.key === 'Enter') {
        e.preventDefault()
        if (refineSelected === 0) {
          setRefining?.(true)
          onRefine?.()
        } else {
          onSkipRefine?.()
        }
      } else if (e.key === 'Escape') {
        e.preventDefault()
        onSkipRefine?.()
      }
    }
    el.addEventListener('keydown', handler)
    return () => el.removeEventListener('keydown', handler)
  }, [refineState, refining, onRefine, onSkipRefine, refineSelected, setRefining])

  const handleInputChange = useCallback((v: string) => {
    console.log('[ChatPanel] handleInputChange called, v:', v)
    setInput(v)
    if (v.startsWith('/') && !v.includes(' ')) {
      setCmdQuery(v)
      setCmdOpen(true)
      setCmdIdx(0)
    } else {
      setCmdOpen(false)
      setResPickerOpen(false)
    }
  }, [])

  // SLASH_ITEMS id (plural) → ChatReference.type (singular)
  const RES_TYPE_MAP: Record<string, ChatReference['type']> = {
    skills: 'skill',
    knowledge: 'knowledge',
    workflows: 'workflow',
  }

  const executeSlash = (id: string) => {
    setCmdOpen(false)
    // Intercept resource commands → open resource picker
    if (id in RES_TYPE_MAP) {
      openResourcePicker(RES_TYPE_MAP[id])
      return
    }
    setInput('')
    onCommand?.(id)
  }

  const executeSlashByIndex = useCallback(() => {
    const item = filteredSlash[cmdIdx]
    if (item) {
      setCmdOpen(false)
      const refType = RES_TYPE_MAP[item.id]
      if (refType) {
        openResourcePicker(refType)
      } else {
        setInput('')
        onCommand?.(item.id)
      }
    }
  }, [filteredSlash, cmdIdx, onCommand])

  useEffect(() => {
    if (!cmdOpen) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        setCmdOpen(false)
        return
      }
      // 空结果（如已有文字前加 "/" 无匹配）：不拦截键盘，放行正常输入/发送
      if (filteredSlash.length === 0) return
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setCmdIdx(i => Math.min(i + 1, filteredSlash.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setCmdIdx(i => Math.max(i - 1, 0))
        return
      }
      if (e.key === 'Enter') {
        e.preventDefault()
        executeSlashByIndex()
        return
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [cmdOpen, filteredSlash.length, executeSlashByIndex])

  const handleSubmit = () => {
    // 执行中发送 = 追加指令（与手机端一致）：不禁用。仅 refine 弹窗打开时拦截。
    if (!input.trim() || refineState) return
    // 将 pendingFiles 注入到 input 文本末尾
    let finalInput = input
    if (pendingFiles.length > 0) {
      const fileRefs = pendingFiles.map(f => `[附件: ${f.path}]`).join('\n')
      finalInput = input + '\n' + fileRefs
    }
    onSend(
      finalInput,
      pendingImages.length > 0 ? pendingImages.map(p => p.dataUrl) : undefined,
      pendingReferences.length > 0 ? pendingReferences : undefined,
    )
    setInput('')
    setPendingImages([])
    setPendingReferences([])
    setPendingFiles([])
    requestAnimationFrame(() => {
      if (textareaRef.current) {
        textareaRef.current.style.height = 'auto'
      }
    })
  }

  const handleRetry = (msgContent: string) => {
    onRetry?.(msgContent)
  }

  const handleCopy = async (msgId: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopiedMsgId(msgId)
      setTimeout(() => setCopiedMsgId(null), 1500)
    } catch {
      // fallback: select and exec copy
      const ta = document.createElement('textarea')
      ta.value = text
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
      setCopiedMsgId(msgId)
      setTimeout(() => setCopiedMsgId(null), 1500)
    }
  }

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return

    if (file.type.startsWith('image/')) {
      const reader = new FileReader()
      reader.onload = () => {
        const dataUrl = reader.result as string
        setPendingImages(prev => [...prev, { dataUrl, name: file.name }])
        setInput(prev => {
          const indicator = `[图片 ${file.name}]`
          return prev ? prev + '\n' + indicator : indicator
        })
        autoResize()
      }
      reader.readAsDataURL(file)
    } else {
      // Non-image file → keep existing behavior: readAsText and append to input
      const reader = new FileReader()
      reader.onload = () => {
        const content = reader.result as string
        const header = `[${file.name}]\n`
        setInput(prev => (prev ? prev + '\n' + header + content : header + content))
        autoResize()
      }
      reader.readAsText(file)
    }
    e.target.value = ''
  }

  /** Handle image attached via drag-drop or paste */
  const handleImageAttach = (file: { name: string; dataUrl: string }) => {
    setPendingImages(prev => [...prev, { dataUrl: file.dataUrl, name: file.name }])
  }

  // ── Reference helpers ──
  const addReference = useCallback((ref: ChatReference) => {
    setPendingReferences(prev => {
      if (prev.some(r => r.type === ref.type && r.id === ref.id)) return prev
      return [...prev, ref]
    })
  }, [])

  const removeReference = useCallback((index: number) => {
    setPendingReferences(prev => prev.filter((_, i) => i !== index))
  }, [])

  const removePendingImage = useCallback((index: number) => {
    setPendingImages(prev => prev.filter((_, i) => i !== index))
  }, [])

  const handleFileAttach = useCallback((file: PendingFile) => {
    setPendingFiles(prev => {
      if (prev.some(f => f.path === file.path)) return prev
      return [...prev, file]
    })
  }, [])

  const removePendingFile = useCallback((index: number) => {
    setPendingFiles(prev => prev.filter((_, i) => i !== index))
  }, [])

  // ── Capture result from DesktopToolbar (Ctrl+U screenshot) ──
  useEffect(() => {
    const handler = (e: Event) => {
      const { path, region, base64 } = (e as CustomEvent).detail as {
        path: string
        region: { x: number; y: number; width: number; height: number }
        base64?: string | null
      }
      if (!path) return
      const fileName = path.split(/[\\/]/).pop() || path
      addReference({
        type: 'capture',
        id: path,
        label: `${fileName} (${region.width}×${region.height})`,
        meta: { region, base64: base64 || undefined },
      })
    }
    window.addEventListener('nuphus:capture-result', handler)
    return () => window.removeEventListener('nuphus:capture-result', handler)
  }, [addReference])

  // ── Resource picker ──
  const openResourcePicker = useCallback(async (type: ChatReference['type']) => {
    setInput('') // clear slash command so stale text isn't sent
    setCmdQuery('')
    setCmdOpen(false)
    setResPickerOpen(true)
    setResPickerType(type)
    setResLoading(true)
    setResError(null)
    setResIdx(0)
    setResItems([])
    try {
      if (type === 'skill') {
        const { invoke } = await import('@tauri-apps/api/core')
        const list =
          await invoke<Array<{ name: string; display_name: string; description: string }>>(
            'skill_list',
          )
        setResItems(
          list.map(s => ({ id: s.name, label: s.display_name || s.name, desc: s.description })),
        )
      } else if (type === 'knowledge') {
        const { listKnowledge } = await import('../lib/api')
        const list = await listKnowledge()
        setResItems(
          (list ?? []).map(k => ({
            id: k.rel_path,
            label: k.title || k.rel_path,
            desc: k.snippet,
          })),
        )
      } else if (type === 'workflow') {
        const { listWorkflows } = await import('../lib/api')
        const list = await listWorkflows()
        setResItems((list ?? []).map(w => ({ id: w.id, label: w.title, desc: w.description })))
      }
    } catch (err: unknown) {
      setResError(err instanceof Error ? err.message : '加载失败')
    } finally {
      setResLoading(false)
    }
  }, [])

  const closeResourcePicker = useCallback(() => {
    setResPickerOpen(false)
    setResPickerType(null)
    setResItems([])
    setResError(null)
  }, [])

  const selectResource = useCallback(
    (item: ResItem) => {
      if (!resPickerType) return
      addReference({ type: resPickerType, id: item.id, label: item.label })
      closeResourcePicker()
    },
    [resPickerType, addReference, closeResourcePicker],
  )

  // ── Resource picker keyboard nav ──
  useEffect(() => {
    if (!resPickerOpen) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        closeResourcePicker()
        return
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setResIdx(i => Math.min(i + 1, resItems.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setResIdx(i => Math.max(i - 1, 0))
        return
      }
      if (e.key === 'Enter') {
        e.preventDefault()
        const item = resItems[resIdx]
        if (item) selectResource(item)
        return
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [resPickerOpen, resItems, resIdx, selectResource, closeResourcePicker])

  // Scroll selected resource item into view
  useEffect(() => {
    const el = resItemRefs.current[resIdx]
    if (el) el.scrollIntoView({ block: 'nearest' })
  }, [resIdx])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey && !cmdOpen && !resPickerOpen) {
      e.preventDefault()
      handleSubmit()
    }
  }

  return (
    <div className="chat-panel">
      {/* ── Session Rail：面板级左缘挂载（自聊天区顶部 10% 起锚，
          感应区纯几何不拦截点击）── */}
      {onChatReplaced && <SessionRail onSessionChanged={onChatReplaced} onNewChat={onNewChat} />}
      {/* ── Chat Header (command palette entry) ── */}
      <div className="chat-header">
        <div className="chat-header-left" />
        <div className="chat-header-right">
          <button
            className="chat-header-settings-btn"
            aria-label={t('app.settings')}
            title={t('app.settings')}
            onClick={() => onOpenPalette?.()}
          >
            <svg
              width="15"
              height="15"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="3" y="3" width="7" height="7" rx="1" />
              <rect x="14" y="3" width="7" height="7" rx="1" />
              <rect x="3" y="14" width="7" height="7" rx="1" />
              <rect x="14" y="14" width="7" height="7" rx="1" />
            </svg>
          </button>
        </div>
      </div>
      {/* ── Refine Pending Button ── */}
      {pendingRefine && !refineState && !refining && (
        <div className="refine-pending-area">
          <button
            className="refine-pending-btn"
            onClick={() => setShowRefineConfirm(true)}
            title={`${t('refine.pendingBtn')} (${pendingRefine.usagePercent}%)`}
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
            >
              <path d="M12 20V10M18 20V4M6 20v-4" strokeLinecap="round" />
            </svg>
            <span className="refine-pending-pct">{pendingRefine.usagePercent}%</span>
            {pendingRefine.skippedTurns > 0 && (
              <span className="refine-pending-badge">
                {pendingRefine.skippedTurns > 99 ? '99+' : pendingRefine.skippedTurns}
              </span>
            )}
          </button>

          {/* ── Confirm dialog when button clicked ── */}
          {showRefineConfirm && (
            <div className="refine-pending-confirm">
              <div className="refine-confirm-body">
                <div className="item-desc">
                  {t('refine.pendingDesc', String(pendingRefine.usagePercent))}
                </div>
                <div className="refine-confirm-actions">
                  <button
                    className="refine-confirm-btn"
                    onClick={() => {
                      setShowRefineConfirm(false)
                      // 与 refine 弹窗路径一致：进入提炼中状态（全屏遮罩由全局
                      // refining 驱动，refine-pending-btn 路径同样触发）
                      setRefining?.(true)
                      onRefine?.()
                      setPendingRefine(null)
                    }}
                  >
                    {t('refine.action')}
                  </button>
                  <button
                    className="refine-confirm-cancel"
                    onClick={() => setShowRefineConfirm(false)}
                  >
                    {t('common.cancel') || 'Cancel'}
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      )}
      <div className="chat-messages" ref={scrollRef}>
        {messages.length === 0 ? (
          <WelcomeScreen
            onSend={onSend}
            startupStats={startupStats}
            onResume={onResumeLast ?? onChatReplaced}
          />
        ) : (
          <div className="chat-messages-inner">
            {(() => {
              return (
                <>
                  {messages.map((msg, idx) => {
                    // Refine message → render SessionDivider, skip normal message-row
                    if (msg.role === 'refine') {
                      return (
                        <SessionDivider
                          key={msg.id}
                          summary={msg.refineStatus === 'completed' ? msg.content : ''}
                          messageCount={msg.messageCount ?? 0}
                          sessionId={msg.sessionId ?? ''}
                          streamingContent={
                            msg.refineStatus === 'streaming' ? msg.content || null : null
                          }
                        />
                      )
                    }
                    const isError =
                      (msg.role === 'system' && msg.content.startsWith('错误')) ||
                      (msg.role === 'assistant' && msg.content.includes('LLM请求失败'))
                    const prevIsUser = idx > 0 && messages[idx - 1]?.role === 'user'
                    // Execution in progress: only the last agent message shows animation
                    const isCurrentAgent = msg.role === 'assistant' && idx === messages.length - 1
                    // Avatar settings
                    const showAvatar = localStorage.getItem('nuphus_show_avatar') === 'true'
                    const userAvatar = localStorage.getItem('nuphus_user_avatar') || ''
                    const nuphusAvatar = localStorage.getItem('nuphus_nuphus_avatar') || ''
                    const skinBg = localStorage.getItem('nuphus_skin_bg') || ''

                    // Default avatar — NuphusLogo (窗口 N)
                    const AvatarComp =
                      msg.role === 'user' ? (
                        userAvatar ? (
                          <img src={userAvatar} alt="" className="msg-avatar-img" />
                        ) : (
                          <NuphusLogo size={22} variant="mark" />
                        )
                      ) : nuphusAvatar ? (
                        <img src={nuphusAvatar} alt="" className="msg-avatar-img" />
                      ) : (
                        <NuphusLogo size={22} variant="mark" />
                      )

                    return (
                      <React.Fragment key={`row-${msg.id}`}>
                        <div
                          key={msg.id}
                          className={`message-row ${msg.role} ${showAvatar ? 'with-avatar' : ''}`}
                        >
                          {msg.role === 'assistant' && showAvatar && (
                            <div className="message-avatar">{AvatarComp}</div>
                          )}
                          <div
                            className={`message-bubble ${msg.role} ${showAvatar ? 'with-avatar' : ''} ${skinBg ? 'with-skin' : ''}`}
                          >
                            <div className="message-header">
                              <span className={`message-label ${msg.role}`}>
                                {msg.role === 'user' ? (
                                  relation.userLabel
                                ) : msg.role === 'assistant' ? (
                                  relation.assistantName
                                ) : (
                                  <span style={{ color: 'var(--warning)' }}>
                                    {t('chat.systemLabel')}
                                  </span>
                                )}
                              </span>
                              {msg.sourceLabel && (
                                <span
                                  className="message-source-badge"
                                  title={`来自插件 ${msg.sourceLabel}`}
                                >
                                  {msg.sourceLabel}
                                </span>
                              )}
                              <span className="message-time">
                                {new Date(msg.timestamp).toLocaleTimeString()}
                              </span>
                            </div>
                            <div className={`message-content ${msg.role}`}>
                              {/* ── 图片附件 ── */}
                              {msg.images && msg.images.length > 0 && (
                                <div className="msg-images">
                                  {msg.images.map((img, i) => (
                                    <img
                                      key={i}
                                      src={img}
                                      alt={`图片 ${i + 1}`}
                                      className="msg-image"
                                      onClick={() => setLightboxUrl(img)}
                                      onError={e => {
                                        ;(e.target as HTMLImageElement).style.display = 'none'
                                      }}
                                    />
                                  ))}
                                </div>
                              )}
                              {/* ── 截图引用（Ctrl+U 截图：本地文件路径经 asset 协议显示）── */}
                              {msg.references && msg.references.some(r => r.type === 'capture') && (
                                <div className="msg-images">
                                  {msg.references
                                    .filter(r => r.type === 'capture')
                                    .map((r, i) => {
                                      const src = r.meta?.base64 || toAssetUrl(r.id)
                                      if (!src) return null
                                      return (
                                        <img
                                          key={`cap-${i}`}
                                          src={src}
                                          alt={r.label || `截图 ${i + 1}`}
                                          className="msg-image"
                                          onClick={() => setLightboxUrl(src)}
                                          onError={e => {
                                            ;(e.target as HTMLImageElement).style.display = 'none'
                                          }}
                                        />
                                      )
                                    })}
                                </div>
                              )}
                              {/* ── 音频附件 ── */}
                              {msg.audio && msg.audio.length > 0 && (
                                <div className="msg-audio-list">
                                  {msg.audio.map((aud, i) => (
                                    <audio
                                      key={i}
                                      controls
                                      className="msg-audio"
                                      preload="metadata"
                                    >
                                      <source src={aud} />
                                    </audio>
                                  ))}
                                </div>
                              )}
                              {/* ── 文本内容 ── */}
                              {msg.role === 'assistant' ? (
                                (() => {
                                  // Normal assistant message
                                  return isCurrentAgent && isProcessing ? (
                                    msg.content ? (
                                      <>
                                        <MarkdownContent
                                          content={msg.content}
                                          onFileClick={setPreviewPath}
                                        />
                                        <span className="message-thinking-cursor" />
                                      </>
                                    ) : (
                                      <span className="message-thinking-dots">
                                        <span className="mtd" />
                                        <span className="mtd" />
                                        <span className="mtd" />
                                      </span>
                                    )
                                  ) : (
                                    <MarkdownContent
                                      content={msg.content}
                                      onFileClick={setPreviewPath}
                                    />
                                  )
                                })()
                              ) : msg.content ? (
                                <span className="msg-plain-text">{msg.content}</span>
                              ) : null}
                            </div>
                            {msg.role === 'assistant' && (
                              <div className="message-actions">
                                <IconButton
                                  variant="msg-action"
                                  label="复制"
                                  title={copiedMsgId === msg.id ? '已复制' : '复制内容'}
                                  onClick={() => handleCopy(msg.id, msg.content)}
                                >
                                  {copiedMsgId === msg.id ? (
                                    <IconCheck size={14} />
                                  ) : (
                                    <IconCopy size={14} />
                                  )}
                                </IconButton>
                                <IconButton
                                  variant="msg-action"
                                  label="点评"
                                  title="点评"
                                  onClick={() => {
                                    const userMsg =
                                      idx > 0 && messages[idx - 1]?.role === 'user'
                                        ? messages[idx - 1].content
                                        : ''
                                    setRatingMsg({
                                      id: msg.id,
                                      content: msg.content,
                                      userQuestion: userMsg,
                                    })
                                  }}
                                >
                                  <svg
                                    width="14"
                                    height="14"
                                    viewBox="0 0 24 24"
                                    fill="none"
                                    stroke="currentColor"
                                    strokeWidth="2"
                                    strokeLinecap="round"
                                    strokeLinejoin="round"
                                  >
                                    <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                                  </svg>
                                </IconButton>
                                {msg.traceItems && msg.traceItems.length > 0 && (
                                  <IconButton
                                    variant="msg-action"
                                    label="执行回溯"
                                    title="查看该轮执行过程"
                                    onClick={() => onShowExecTrace?.(msg.traceItems!)}
                                  >
                                    <svg
                                      width="14"
                                      height="14"
                                      viewBox="0 0 24 24"
                                      fill="none"
                                      stroke="currentColor"
                                      strokeWidth="2"
                                      strokeLinecap="round"
                                      strokeLinejoin="round"
                                    >
                                      <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
                                      <path d="M3 3v5h5" />
                                      <path d="M12 7v5l4 2" />
                                    </svg>
                                  </IconButton>
                                )}
                              </div>
                            )}
                          </div>
                          {msg.role === 'user' && showAvatar && (
                            <div className="message-avatar user-side">{AvatarComp}</div>
                          )}
                          {isError && prevIsUser && !isProcessing && onRetry && (
                            <div className="message-retry">
                              <Button
                                variant="ghost"
                                size="sm"
                                onClick={() => handleRetry(messages[idx - 1].content)}
                              >
                                重试 ↻
                              </Button>
                            </div>
                          )}
                        </div>
                      </React.Fragment>
                    )
                  })}
                </>
              )
            })()}
          </div>
        )}
      </div>

      {/* ── Pause Modal ── */}
      <PauseOverlay
        pauseState={showPauseLocal ? { actionId: 'local' } : (pauseState ?? null)}
        pauseMode={pauseMode}
        pauseActionBusy={pauseActionBusy}
        selectedOption={selectedOption}
        appendInput={appendInput}
        pauseSubmitting={pauseSubmitting}
        onPauseChoice={handlePauseChoice}
        onAppendInputChange={setAppendInput}
        onBackToMenu={() => setPauseMode('menu')}
        onSubmitAppend={handleSubmitAppend}
      />

      {/* ── Refine Modal ── */}
      {refineState &&
        createPortal(
          <div className="compact-overlay compact-overlay--high">
            <div
              className="compact-modal compact-modal--sm compact-modal--fit"
              onClick={e => e.stopPropagation()}
            >
              <div className="compact-header">
                <span className="compact-header-title">{t('refine.title')}</span>
                {/* 提炼中允许关闭：后端失败（key 失效/连不上）不再广播结束事件时，
                    用户不会被全屏弹窗困死（后台提炼继续，完成后照常落地） */}
                {refining && (
                  <button
                    className="compact-header-close"
                    title={t('refine.dismissHint')}
                    aria-label={t('refine.dismissHint')}
                    onClick={() => (onDismissRefine ? onDismissRefine() : setRefining?.(false))}
                  >
                    <svg
                      width="14"
                      height="14"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                    >
                      <line x1="18" y1="6" x2="6" y2="18" />
                      <line x1="6" y1="6" x2="18" y2="18" />
                    </svg>
                  </button>
                )}
              </div>
              <div className="compact-divider" />
              <div className="compact-body">
                {!refining ? (
                  <>
                    <div className="refine-modal-head">
                      <span className="badge badge-warning">{t('refine.suggest')}</span>
                      <span className="refine-usage-pct">
                        {Math.round(refineState.usagePercent)}%
                      </span>
                    </div>
                    <div className="item-desc">{t('refine.desc')}</div>
                    <div className="item-sub item-sub--mono refine-usage">
                      {t(
                        'refine.usage',
                        String(Math.round(refineState.usagePercent)),
                        formatTokens(
                          Math.round((refineState.totalLimit * refineState.usagePercent) / 100),
                        ),
                        formatTokens(refineState.totalLimit),
                      )}
                    </div>
                    <div
                      style={{
                        height: 3,
                        background: 'var(--glass-2)',
                        borderRadius: 2,
                        overflow: 'hidden',
                        marginBottom: 12,
                      }}
                    >
                      <div
                        style={{
                          height: '100%',
                          width: `${Math.min(refineState.usagePercent, 100)}%`,
                          background:
                            refineState.usagePercent > 80
                              ? 'var(--error)'
                              : refineState.usagePercent > 60
                                ? 'var(--warning)'
                                : 'var(--accent)',
                          borderRadius: 2,
                          transition: 'width .3s ease',
                        }}
                      />
                    </div>
                  </>
                ) : (
                  <div
                    style={{
                      display: 'flex',
                      flexDirection: 'column',
                      alignItems: 'center',
                      gap: 12,
                      padding: '24px 0',
                    }}
                  >
                    <div
                      style={{
                        width: 28,
                        height: 28,
                        border: '3px solid var(--glass-1)',
                        borderTopColor: 'var(--accent)',
                        borderRadius: '50%',
                        animation: 'spin .8s linear infinite',
                      }}
                    />
                    <div style={{ fontSize: 14, color: 'var(--spark-primary)', fontWeight: 500 }}>
                      {t('refine.processing')}
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--spark-muted)' }}>
                      {t('refine.processingDesc')}
                    </div>
                  </div>
                )}
                {!refining && (
                  <div
                    style={{ display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 8 }}
                  >
                    <div
                      onClick={() => {
                        if (!refining) {
                          setRefining?.(true)
                          onRefine?.()
                        }
                      }}
                      className={refining ? 'refine-option refining' : 'refine-option'}
                      style={{
                        background: refineSelected === 0 ? 'var(--void-hover)' : 'transparent',
                      }}
                    >
                      <span
                        style={{
                          fontSize: 12,
                          color: 'var(--accent)',
                          width: 14,
                          flexShrink: 0,
                          fontFamily: 'var(--font-mono)',
                        }}
                      >
                        ▸
                      </span>
                      <div>
                        <div
                          style={{ fontSize: 13, color: 'var(--spark-primary)', fontWeight: 500 }}
                        >
                          {t('refine.action')}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--spark-muted)' }}>
                          {t('refine.processingAction')}
                        </div>
                      </div>
                    </div>
                    <div
                      onClick={() => onSkipRefine?.()}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        padding: '8px 10px',
                        borderRadius: 8,
                        cursor: 'pointer',
                        background: refineSelected === 1 ? 'var(--void-hover)' : 'transparent',
                      }}
                    >
                      <span
                        style={{
                          fontSize: 12,
                          color: 'var(--accent)',
                          width: 14,
                          flexShrink: 0,
                          fontFamily: 'var(--font-mono)',
                        }}
                      >
                        {refineSelected === 1 ? '▸' : ' '}
                      </span>
                      <div>
                        <div
                          style={{ fontSize: 13, color: 'var(--spark-primary)', fontWeight: 500 }}
                        >
                          {t('refine.skip')}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--spark-muted)' }}>
                          {t('refine.skipDesc')}
                        </div>
                      </div>
                    </div>
                  </div>
                )}
                <div
                  style={{
                    display: 'flex',
                    gap: 12,
                    fontSize: 10,
                    color: 'var(--spark-dim)',
                    fontFamily: 'var(--font-mono)',
                  }}
                >
                  <span>{refining ? t('refine.processing') : t('refine.hintSelect')}</span>
                  <span>{refining ? '' : t('refine.hintConfirm')}</span>
                </div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* ── 提炼中全屏遮罩（refine-pending-btn 路径 / 弹窗已关闭但后端仍在提炼）──
          弹窗路径的提炼中状态内嵌在 refineState 弹窗里；pending 路径 refineState
          为 null，需独立遮罩。关闭按钮走 onDismissRefine（复位 UI + 提炼追踪
          refs）——后端失败/超时未发结束事件时可手动退出，不困在全屏遮罩里。 */}
      {refining &&
        !refineState &&
        !pendingRefine &&
        createPortal(
          <div className="compact-overlay compact-overlay--high">
            <div
              className="compact-modal compact-modal--sm compact-modal--fit"
              onClick={e => e.stopPropagation()}
            >
              <div className="compact-header">
                <span className="compact-header-title">{t('refine.title')}</span>
                <button
                  className="compact-header-close"
                  title={t('refine.dismissHint')}
                  aria-label={t('refine.dismissHint')}
                  onClick={() => (onDismissRefine ? onDismissRefine() : setRefining?.(false))}
                >
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                  >
                    <line x1="18" y1="6" x2="6" y2="18" />
                    <line x1="6" y1="6" x2="18" y2="18" />
                  </svg>
                </button>
              </div>
              <div className="compact-divider" />
              <div className="compact-body">
                <div className="refine-processing-box">
                  <span className="refine-spinner" aria-hidden="true" />
                  <span className="refine-processing-text">{t('refine.processing')}</span>
                </div>
                <div className="item-sub item-sub--mono">{t('refine.processingDesc')}</div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* 新手引导弹窗 */}
      {showOnboarding && (
        <OnboardingModal
          onComplete={() => {
            setShowOnboarding(false)
            loadSavedConfigs().then(setSavedConfigs)
          }}
          onSkip={() => {
            setShowOnboarding(false)
            hudUpdate('⚠ 尚未配置模型，Ctrl+K → 模型设置', 'warning')
          }}
        />
      )}

      {/* 输入区 dock：badge 与输入框共享同一定位几何，保证左边缘对齐 */}
      <div className="chat-input-dock">
        <VideoProgressBadge />
        {/* 外部 Agent 悬浮胶囊：输入框外层右上角（absolute 定位，不占文档流） */}
        <ExternalAgentsStatusBar onOpenConfig={onManageExternalAgents} />
        <ChatInputBar
          input={input}
          onInputChange={handleInputChange}
          onInputKeyDown={handleKeyDown}
          textareaRef={textareaRef}
          imageInputRef={fileInputRef}
          isProcessing={isProcessing}
          pauseState={pauseState ?? null}
          refineState={refineState ?? null}
          tokenUsage={tokenUsage || null}
          mainTokenUsage={mainTokenUsage || null}
          execTokenUsage={execTokenUsage || null}
          totalDurationMs={totalDurationMs}
          totalCalls={totalCalls}
          mood={mood || 'idle'}
          contextLimit={contextLimit}
          security={security ?? null}
          onApproveSecurity={onApproveSecurity}
          onRejectSecurity={onRejectSecurity}
          mode={mode}
          onSetMode={onSetMode}
          onManageCustomAgents={onManageCustomAgents}
          onToggleWorkAgentMode={onToggleWorkAgentMode}
          modelLabel={modelLabel}
          modelName={modelName}
          effort={effort}
          supportedEfforts={currentModelEfforts}
          defaultEffort={currentModelDefaultEffort}
          onEffortChange={handleEffortChange}
          onModelSwitch={() => setModelOpen(true)}
          onSend={handleSubmit}
          onInterrupt={onInterrupt}
          isWorkflowRunning={isWorkflowRunning}
          showDesktopToolbar={showDesktopToolbar}
          onToggleDesktopToolbar={onToggleDesktopToolbar}
          toolPermissions={toolPermissions}
          onFileSelect={handleFileSelect}
          onImageAttach={handleImageAttach}
          onFileAttach={handleFileAttach}
          projectDir={projectDir}
          onOpenProjectDir={() => {
            setDirInput(projectDir)
            setDirOpen(true)
          }}
          onOpenPrinciples={onOpenPrinciples}
          onOpenAnnotations={onOpenAnnotations}
          hints={HINTS}
          hintIndex={hintIndex}
          hintFade={hintFade}
          pendingReferences={pendingReferences}
          pendingImages={pendingImages}
          pendingFiles={pendingFiles}
          onRemoveReference={removeReference}
          onRemoveImage={removePendingImage}
          onRemoveFile={removePendingFile}
        />
      </div>

      {/* ── Project directory Modal ── */}
      {dirOpen && (
        <div className="input-modal-overlay" onClick={() => setDirOpen(false)}>
          <div className="input-modal" onClick={e => e.stopPropagation()}>
            <div className="input-modal-header">
              <span>
                <IconFolder size={14} /> 项目目录
              </span>
              <IconButton variant="modal-close" label="关闭" onClick={() => setDirOpen(false)}>
                <IconX size={14} />
              </IconButton>
            </div>
            <div className="input-modal-body">
              <div className="input-modal-label">当前工作目录</div>
              <div className="input-modal-path">
                {projectDir || <span style={{ opacity: 0.4 }}>未设置</span>}
              </div>

              {dirBookmarks.length > 0 && (
                <>
                  <div className="input-modal-label input-modal-label-gap">项目书签</div>
                  <div className="input-modal-grid">
                    {dirBookmarks.map(d => (
                      <button
                        key={d.path}
                        className={`input-modal-chip ${projectDir === d.path ? 'active' : ''}`}
                        onClick={() => {
                          setProjectDir(d.path)
                          setDirInput(d.path)
                          try {
                            localStorage.setItem('nuphus_project_dir', d.path)
                          } catch {}
                          import('../lib/api').then(m => m.setProjectDir(d.path)).catch(() => {})
                        }}
                      >
                        <IconFolder size={12} />
                        <span>{d.label}</span>
                      </button>
                    ))}
                  </div>
                </>
              )}

              <div className="input-modal-label input-modal-label-gap">自定义路径</div>
              <div className="input-modal-input-row">
                <input
                  className="input-modal-input"
                  value={dirInput}
                  onChange={e => setDirInput(e.target.value)}
                  placeholder="输入完整路径..."
                />
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() => {
                    setProjectDir(dirInput)
                    try {
                      localStorage.setItem('nuphus_project_dir', dirInput)
                    } catch {}
                    import('../lib/api').then(m => m.setProjectDir(dirInput)).catch(() => {})
                    setDirOpen(false)
                  }}
                >
                  确定
                </Button>
              </div>
              {dirInput.trim() && !dirBookmarks.find(b => b.path === dirInput.trim()) && (
                <div style={{ marginTop: 8, display: 'flex', justifyContent: 'flex-end' }}>
                  <button
                    style={{
                      fontSize: 11,
                      padding: '4px 10px',
                      borderRadius: 6,
                      border: '1px solid var(--glass-2)',
                      background: 'var(--glass-1)',
                      color: 'var(--accent)',
                      cursor: 'pointer',
                    }}
                    onClick={() => {
                      const label = dirInput.split(/[/\\]/).filter(Boolean).pop() || '未命名'
                      const updated = [...dirBookmarks, { label, path: dirInput.trim() }]
                      setDirBookmarks(updated)
                      try {
                        localStorage.setItem('nuphus_projects', JSON.stringify(updated))
                      } catch {}
                    }}
                  >
                    + 保存到项目书签
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ── Skills Manager Modal ── */}
      {skillsOpen &&
        createPortal(
          <div className="cmd-modal-overlay" onClick={() => setSkillsOpen(false)}>
            <div className="cmd-modal" onClick={e => e.stopPropagation()}>
              <div className="cmd-modal-header">
                <span className="cmd-modal-icon">◆</span>
                <span className="cmd-modal-title">Skills Manager</span>
                <IconButton variant="modal-close" label="关闭" onClick={() => setSkillsOpen(false)}>
                  <IconX size={14} />
                </IconButton>
              </div>
              <div className="cmd-modal-body">
                <div className="cmd-modal-section">
                  <div className="cmd-modal-subtitle">Installed Skills</div>
                  <div className="cmd-modal-empty">
                    No skill packages installed yet.
                    <div className="cmd-modal-empty-hint">
                      Skills extend Nuphus capabilities with domain-specific knowledge.
                    </div>
                  </div>
                </div>
                <div className="cmd-modal-section">
                  <div className="cmd-modal-subtitle">Install New Skill</div>
                  <div className="cmd-modal-input-row">
                    <input className="cmd-modal-input" placeholder="Skill name or git URL..." />
                    <Button variant="primary" size="sm">
                      Install
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* ── Model Manager Modal ── */}
      {modelOpen &&
        createPortal(
          <div className="cmd-modal-overlay" onClick={() => setModelOpen(false)}>
            <div className="cmd-modal cmd-modal-sm" onClick={e => e.stopPropagation()}>
              <div className="cmd-modal-header">
                <span className="cmd-modal-icon">⚙</span>
                <span className="cmd-modal-title">{t('modelManager.title')}</span>
                <IconButton variant="modal-close" label="关闭" onClick={() => setModelOpen(false)}>
                  <IconX size={14} />
                </IconButton>
              </div>
              <div className="cmd-modal-body">
                {savedConfigs.length === 0 ? (
                  <div className="cmd-modal-empty">
                    {t('modelManager.noConfigs')}
                    <div className="cmd-modal-empty-hint">{t('modelManager.noConfigsHint')}</div>
                  </div>
                ) : (
                  <div className="cmd-modal-list">
                    {savedConfigs.map(cfg => {
                      // 卡片当前显���的模型 = 预览态（上下按钮浏览）|| 已保存配置
                      const displayedModel = peekModels[cfg.provider] || cfg.model
                      // ✓ 只标真正生效的模型：预览到别的模型时该卡不视为 active
                      const isActive = !peekModels[cfg.provider] && modelLabel === cfg.model
                      // 同 provider 全部模型（上下浏览数据源；仅 1 个时禁用浏览）
                      const providerModels = allModels.filter(m => m.provider === cfg.provider)
                      return (
                        <div key={cfg.provider}>
                          <div
                            className={`cmd-modal-card cmd-modal-card-face ${isActive ? 'active' : ''} ${switchingId === cfg.id ? 'switching' : ''}`}
                            onClick={() =>
                              switchConfig(
                                {
                                  ...cfg,
                                  id: `${cfg.provider}::${displayedModel}`,
                                  model: displayedModel,
                                },
                                true,
                              )
                            }
                          >
                            <div className="cmd-modal-card-left">
                              <span className="cmd-modal-provider-icon">
                                {(cfg.label || cfg.provider).charAt(0).toUpperCase()}
                              </span>
                            </div>
                            <div className="cmd-modal-card-body">
                              <div className="cmd-modal-card-name">{cfg.label}</div>
                              <div className="cmd-modal-card-meta">{displayedModel}</div>
                            </div>
                            {switchingId === cfg.id ? (
                              <div className="cmd-modal-card-spinner" />
                            ) : isActive ? (
                              <span className="cmd-modal-card-check">✓</span>
                            ) : null}
                            {/* 上下浏览按钮：仅预览相邻模型，不切换——切换靠点击卡片本身 */}
                            <div className="cmd-modal-card-peek">
                              <button
                                type="button"
                                className="cmd-modal-card-peek-btn"
                                title={t('modelManager.peekPrev')}
                                aria-label={t('modelManager.peekPrev')}
                                disabled={providerModels.length <= 1}
                                onClick={e => {
                                  e.stopPropagation()
                                  peekSwitch(cfg, -1)
                                }}
                              >
                                <IconChevronUp size={12} />
                              </button>
                              <button
                                type="button"
                                className="cmd-modal-card-peek-btn"
                                title={t('modelManager.peekNext')}
                                aria-label={t('modelManager.peekNext')}
                                disabled={providerModels.length <= 1}
                                onClick={e => {
                                  e.stopPropagation()
                                  peekSwitch(cfg, 1)
                                }}
                              >
                                <IconChevronDown size={12} />
                              </button>
                            </div>
                          </div>
                        </div>
                      )
                    })}
                  </div>
                )}
                <div className="cmd-modal-footer-hint">
                  Tip: Use <strong>/models</strong> to quickly switch models anytime
                </div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* 空结果（输入框已有文字前加 "/" 无匹配）时不渲染——避免空 palette 的
          border/shadow 显示为长条黑块（用户实测） */}
      {cmdOpen &&
        filteredSlash.length > 0 &&
        createPortal(
          <div className="cmd-overlay" onClick={() => setCmdOpen(false)}>
            <div
              className="cmd-palette"
              onClick={e => e.stopPropagation()}
              onWheel={e => {
                e.preventDefault()
                if (e.deltaY > 0) {
                  setCmdIdx(i => Math.min(i + 1, filteredSlash.length - 1))
                } else {
                  setCmdIdx(i => Math.max(i - 1, 0))
                }
              }}
            >
              {filteredSlash.map((item, i) => (
                <div
                  key={item.id}
                  ref={el => {
                    cmdItemRefs.current[i] = el
                  }}
                  className={`cmd-item ${i === cmdIdx ? 'selected' : ''}`}
                  onClick={() => executeSlash(item.id)}
                  onMouseEnter={() => setCmdIdx(i)}
                >
                  {SLASH_ICONS[item.id] && <span className="cmd-icon">{SLASH_ICONS[item.id]}</span>}
                  <span className="cmd-label">{item.label}</span>
                  <span className="cmd-desc">{item.desc}</span>
                  {i === cmdIdx && <span className="cmd-arrow">↵</span>}
                </div>
              ))}
            </div>
          </div>,
          document.body,
        )}

      {/* ── Resource picker (skill/knowledge/workflow selection) ── */}
      {resPickerOpen &&
        createPortal(
          <div className="cmd-overlay" onClick={closeResourcePicker}>
            <div
              className="cmd-palette"
              onClick={e => e.stopPropagation()}
              onWheel={e => {
                e.preventDefault()
                if (e.deltaY > 0) {
                  setResIdx(i => Math.min(i + 1, resItems.length - 1))
                } else {
                  setResIdx(i => Math.max(i - 1, 0))
                }
              }}
            >
              <div className="cmd-palette-header">
                <span className="cmd-palette-title">
                  {resPickerType === 'skill'
                    ? '选择 Skill'
                    : resPickerType === 'knowledge'
                      ? '选择知识库'
                      : '选择工作流'}
                </span>
              </div>
              {resLoading && <div className="cmd-item cmd-item--hint">加载中...</div>}
              {resError && (
                <div className="cmd-item cmd-item--hint" style={{ color: 'var(--error)' }}>
                  {resError}
                </div>
              )}
              {!resLoading && !resError && resItems.length === 0 && (
                <div className="cmd-item cmd-item--hint">无可用项</div>
              )}
              {!resLoading &&
                resItems.map((item, i) => (
                  <div
                    key={item.id}
                    ref={el => {
                      resItemRefs.current[i] = el
                    }}
                    className={`cmd-item ${i === resIdx ? 'selected' : ''}`}
                    onClick={() => selectResource(item)}
                    onMouseEnter={() => setResIdx(i)}
                  >
                    <span className="cmd-label">{item.label}</span>
                    {item.desc && <span className="cmd-desc">{item.desc}</span>}
                    {i === resIdx && <span className="cmd-arrow">↵</span>}
                  </div>
                ))}
            </div>
          </div>,
          document.body,
        )}

      {/* ── 点评弹窗 ── */}
      {ratingMsg && (
        <RatingModal
          goal={ratingMsg.content.slice(0, 80)}
          toolCalls={[]}
          totalMs={0}
          onClose={() => setRatingMsg(null)}
          onSubmit={(name, rating, comment, saveAsStrategy) => {
            onRate?.(
              name,
              rating,
              comment,
              saveAsStrategy,
              ratingMsg.userQuestion,
              ratingMsg.content,
            )
            setRatingMsg(null)
          }}
        />
      )}
      {/* Lightbox overlay for image click-to-zoom */}
      {lightboxUrl && (
        <div className="msg-lightbox-overlay" onClick={() => setLightboxUrl(null)}>
          <img
            src={lightboxUrl}
            alt="放大预览"
            className="msg-lightbox-image"
            onClick={e => e.stopPropagation()}
          />
        </div>
      )}

      {/* ── 文件预览覆盖层（AI 回复路径点击，全屏对齐画布范式） ── */}
      {previewPath && <PreviewOverlay path={previewPath} onClose={() => setPreviewPath(null)} />}
    </div>
  )
}
