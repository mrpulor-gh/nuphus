import { useState, useRef, useEffect, useCallback, type RefObject } from 'react'
import { IconSend, IconSquare, IconBrain, IconWorkflow, IconSparkles, IconWrench } from '../../ui/Icons'
import { IconButton } from '../../ui/Button'
import { MOOD_COLORS } from '../layout/StatusBar'
import { SecurityPrompt } from '../layout/SecurityPrompt'
import { VoiceButton, type VoiceButtonHandle } from './VoiceButton'
import { useLanguage } from '../../locales'
import ReferenceBar from './ReferenceBar'
import type { ChatReference, PendingImage, PendingFile } from '../../core/types'
import {
  listCustomAgents,
  getActiveCustomAgent,
  setActiveCustomAgent,
  isBusy,
  type CustomAgentConfig,
} from '../lib/api'

interface TokenUsageInfo {
  inputTokens: number
  outputTokens: number
  cacheHitTokens: number
}

interface ChatInputBarProps {
  /** 输入框当前值 */
  input: string
  /** 输入框值变更 */
  onInputChange: (value: string) => void
  /** 输入框 keydown 事件 */
  onInputKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void
  /** textarea 引用（父组件用于 reset 高度） */
  textareaRef: RefObject<HTMLTextAreaElement | null>
  /** 图片上传 input 引用 */
  imageInputRef: RefObject<HTMLInputElement | null>
  isProcessing: boolean
  pauseState: { actionId: string } | null
  refineState: { usagePercent: number; totalLimit: number } | null
  /** token 用量 */
  tokenUsage: TokenUsageInfo | null
  mainTokenUsage: TokenUsageInfo | null
  execTokenUsage: TokenUsageInfo | null
  totalDurationMs: number | undefined
  totalCalls: number | undefined
  mood: string
  contextLimit: number | undefined
  /** 安全审查 */
  security: { tool: string; risk: string; reason: string; actionId: string } | null
  onApproveSecurity?: (id: string) => void
  onRejectSecurity?: (id: string) => void
  /** 模式切换 */
  mode: string | undefined
  onSetMode?: (mode: string) => void
  /** 打开 Custom Agent 管理页（弹窗「管理 Agent」入口） */
  onManageCustomAgents?: () => void
  /** 模型信息 */
  modelLabel: string
  modelName: string | undefined
  /** 推理深度（null = 提供商默认） */
  effort: string | null
  /** 当前模型支持的推理深度级别（空 = 不支持，隐藏入口） */
  supportedEfforts: string[]
  /** 当前模型的默认推理深度（未配置时生效；null = 无声明，显示「默认」） */
  defaultEffort?: string | null
  /** 切换推理深度（null = 清除，用提供商默认） */
  onEffortChange: (effort: string | null) => void
  /** 工作流模式（mode chip 第四档） */
  onToggleWorkAgentMode?: () => Promise<void>
  /** 桌面工具箱（Ctrl+U）显示状态与切换（workflow 模式下在 mode chip 旁显示按钮） */
  showDesktopToolbar?: boolean
  onToggleDesktopToolbar?: () => void
  onModelSwitch: () => void
  /** 权限状态（用于 WORKFLOW 模式权限检查） */
  toolPermissions?: { file_access: boolean; web_search: boolean; system_automation: boolean }
  /** 发送 / 中断 */
  onSend: () => void
  onInterrupt?: () => void
  /** Leader 暂停是否禁用（workflow 运行时禁用） */
  isWorkflowRunning?: boolean
  /** 文件选择 */
  onFileSelect: (e: React.ChangeEvent<HTMLInputElement>) => void
  /** 拖拽/粘贴图片时回调（dataUrl → processImageAttachment + 输入框指示） */
  onImageAttach: (file: { name: string; dataUrl: string }) => void
  /** 项目目录 */
  projectDir: string
  onOpenProjectDir: () => void
  /** 输入提示 */
  hints: string[]
  hintIndex: number
  hintFade: boolean
  /** 引用栏 props */
  pendingReferences?: ChatReference[]
  pendingImages?: PendingImage[]
  pendingFiles?: PendingFile[]
  onRemoveReference?: (index: number) => void
  onRemoveImage?: (index: number) => void
  onRemoveFile?: (index: number) => void
  /** 拖拽文件时回调（添加到 pendingFiles） */
  onFileAttach?: (file: PendingFile) => void
}

export function ChatInputBar({
  input,
  onInputChange,
  onInputKeyDown,
  textareaRef,
  imageInputRef,
  isProcessing,
  pauseState,
  refineState,
  tokenUsage,
  mainTokenUsage,
  execTokenUsage,
  totalDurationMs,
  totalCalls,
  mood,
  contextLimit,
  security,
  onApproveSecurity,
  onRejectSecurity,
  mode,
  onSetMode,
  onManageCustomAgents,
  onToggleWorkAgentMode,
  showDesktopToolbar,
  onToggleDesktopToolbar,
  modelLabel,
  modelName,
  effort,
  supportedEfforts,
  defaultEffort,
  onEffortChange,
  onModelSwitch,
  onSend,
  onInterrupt,
  isWorkflowRunning,
  onFileSelect,
  onImageAttach,
  projectDir,
  onOpenProjectDir,
  hints,
  hintIndex,
  hintFade,
  toolPermissions,
  pendingReferences,
  pendingImages,
  pendingFiles,
  onRemoveReference,
  onRemoveImage,
  onRemoveFile,
  onFileAttach,
}: ChatInputBarProps) {
  const { t } = useLanguage()
  const [localTextareaRef, setLocalTextareaRef] = useState<HTMLTextAreaElement | null>(null)
  const modeSwitchLock = useRef(false)
  // ── 工具弹窗（附件/图片/项目目录 合并入口）──
  const [toolMenuOpen, setToolMenuOpen] = useState(false)
  /** workflow 工具箱按钮 hover 提示（自定义浮层，非原生 title） */
  const [showToolboxTip, setShowToolboxTip] = useState(false)
  const toolMenuRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!toolMenuOpen) return
    const onDown = (e: MouseEvent) => {
      if (toolMenuRef.current && !toolMenuRef.current.contains(e.target as Node)) {
        setToolMenuOpen(false)
      }
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [toolMenuOpen])
  // ── ctx 详情弹窗（hover 显示 cache/tok/step/ms，带悬停桥接防间隙丢失）──
  const [ctxHover, setCtxHover] = useState(false)
  const ctxTimer = useRef<number | null>(null)
  const openCtx = useCallback(() => {
    if (ctxTimer.current) window.clearTimeout(ctxTimer.current)
    setCtxHover(true)
  }, [])
  const closeCtx = useCallback(() => {
    if (ctxTimer.current) window.clearTimeout(ctxTimer.current)
    ctxTimer.current = window.setTimeout(() => setCtxHover(false), 160)
  }, [])
  useEffect(() => () => {
    if (ctxTimer.current) window.clearTimeout(ctxTimer.current)
  }, [])
  // ── 实时计时：执行中 time 实时走动；结束后以后端权威 totalDurationMs 覆盖 ──
  const [liveDuration, setLiveDuration] = useState(0)
  const startTimeRef = useRef<number | null>(null)
  useEffect(() => {
    if (isProcessing) {
      if (startTimeRef.current === null) startTimeRef.current = Date.now()
      const timer = window.setInterval(() => {
        setLiveDuration(Date.now() - startTimeRef.current!)
      }, 100)
      return () => window.clearInterval(timer)
    }
    // 结束：优先后端权威值；无值则清零
    setLiveDuration(totalDurationMs !== undefined && totalDurationMs > 0 ? totalDurationMs : 0)
    startTimeRef.current = null
  }, [isProcessing, totalDurationMs])
  // ── 拖拽文件支持 ──
  const [isDragOver, setIsDragOver] = useState(false)
  const inputRef = useRef(input)
  inputRef.current = input
  const onInputChangeRef = useRef(onInputChange)
  onInputChangeRef.current = onInputChange
  const onImageAttachRef = useRef(onImageAttach)
  onImageAttachRef.current = onImageAttach
  const onFileAttachRef = useRef(onFileAttach)
  onFileAttachRef.current = onFileAttach

  // ── 语音输入（STT）──
  // partial：ghost 预览（斜体弱色），不进输入框；final：拼入输入框可编辑后发送
  const [voicePartial, setVoicePartial] = useState('')
  const handleVoiceFinal = useCallback((text: string) => {
    const base = inputRef.current
    const sep = base.length > 0 && !/[\s，。！？,.!?]$/.test(base) ? ' ' : ''
    onInputChangeRef.current(base + sep + text)
    setVoicePartial('')
  }, [])

  // ── 后端任务执行状态（终止按钮唯一权威源）──
  // 终止按钮只判断后端 is_busy（state.busy，执行开始/结束时由后端置位）：
  // 与前端生命周期无关——界面刷新/重载不影响，查询即得真实状态。
  // 同步：挂载即查 + busy 期间 1.5s 轮询（感知执行开始/结束）；空闲停止轮询。
  const [backendBusy, setBackendBusy] = useState(false)
  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setInterval> | null = null
    const check = async () => {
      try {
        const busy = await isBusy()
        if (cancelled) return
        setBackendBusy(busy === true)
        if (busy && !timer) {
          timer = setInterval(() => {
            isBusy()
              .then(b => {
                if (!cancelled) setBackendBusy(b === true)
              })
              .catch(() => {})
          }, 1500)
        } else if (!busy && timer) {
          clearInterval(timer)
          timer = null
        }
      } catch {
        /* 后端不可达：保持当前状态，等待下次触发 */
      }
    }
    void check()
    return () => {
      cancelled = true
      if (timer) clearInterval(timer)
    }
  }, [])

  // ── 发送=说完：录音/识别中点发送 → 先停止会话并等尾部 final 拼入输入框，
  // 再执行发送；杜绝「发了半句 + 麦克风悬挂 + 漂浮文本进下一条草稿」──
  const voiceRef = useRef<VoiceButtonHandle>(null)
  const onSendRef = useRef(onSend)
  onSendRef.current = onSend
  const flushThenSend = useCallback(() => {
    const v = voiceRef.current
    if (!v?.isActive()) {
      onSendRef.current()
      return
    }
    void (async () => {
      await v.stopAndFlush()
      // 尾部 final 的 setInput 需等一次 React 刷新才进入 handleSubmit 闭包
      setTimeout(() => onSendRef.current(), 0)
    })()
  }, [])

  function isImagePath(p: string): boolean {
    const ext = p.split('.').pop()?.toLowerCase()
    return (
      ext === 'png' ||
      ext === 'jpg' ||
      ext === 'jpeg' ||
      ext === 'gif' ||
      ext === 'webp' ||
      ext === 'bmp'
    )
  }
  function partitionPaths(paths: string[]): { imagePaths: string[]; otherPaths: string[] } {
    const imagePaths: string[] = []
    const otherPaths: string[] = []
    for (const p of paths) {
      if (isImagePath(p)) imagePaths.push(p)
      else otherPaths.push(p)
    }
    return { imagePaths, otherPaths }
  }
  async function handleDroppedImages(
    paths: string[],
    currentInput: string,
    setInput: (v: string) => void,
    setDrag: (v: boolean) => void,
  ) {
    for (const p of paths) {
      try {
        const { invoke } = await import('@tauri-apps/api/core')
        const name = p.split('\\').pop()?.split('/').pop() || p
        const raw = await invoke<{ base64: string; mime: string }>('read_image_base64', {
          imagePath: p,
        })
        const dataUrl = `data:${raw.mime || 'image/png'};base64,${raw.base64}`
        onImageAttachRef.current({ name, dataUrl })
      } catch (e) {
        console.error('[DragDrop] read image failed:', p, e)
      }
    }
  }

  // ── mode hover 弹窗：展示 Leader/Workflow/Custom 三档，点击切换（悬停桥接防间隙丢失）──
  const [modeMenuOpen, setModeMenuOpen] = useState(false)
  const modeMenuRef = useRef<HTMLDivElement>(null)
  const modeMenuTimer = useRef<number | null>(null)
  const openModeMenu = useCallback(() => {
    if (modeMenuTimer.current) window.clearTimeout(modeMenuTimer.current)
    setModeMenuOpen(true)
  }, [])

  // ── Custom agents：卡片列表 + 当前激活（弹窗打开时刷新，保证最新）──
  const [customAgents, setCustomAgents] = useState<CustomAgentConfig[]>([])
  const [activeCustomId, setActiveCustomId] = useState<string | null>(null)
  const refreshCustomAgents = useCallback(() => {
    listCustomAgents()
      .then(list => setCustomAgents(list || []))
      .catch(() => {})
    getActiveCustomAgent()
      .then(a => setActiveCustomId(a?.id ?? null))
      .catch(() => {})
  }, [])
  useEffect(() => {
    refreshCustomAgents()
  }, [refreshCustomAgents])
  useEffect(() => {
    if (modeMenuOpen) refreshCustomAgents()
  }, [modeMenuOpen, refreshCustomAgents])
  const activeCustom = customAgents.find(c => c.id === activeCustomId) || null
  const closeModeMenu = useCallback(() => {
    if (modeMenuTimer.current) window.clearTimeout(modeMenuTimer.current)
    modeMenuTimer.current = window.setTimeout(() => setModeMenuOpen(false), 160)
  }, [])
  useEffect(() => {
    if (!modeMenuOpen) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setModeMenuOpen(false)
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [modeMenuOpen])
  const selectMode = useCallback(
    (m: string) => {
      setModeMenuOpen(false)
      // 幂等：目标已是当前 mode 时仅关菜单——onToggleWorkAgentMode 是 toggle 语义，
      // 无条件调用会把已处于 workflow 的模式退回 leader
      if (m === (mode ?? 'leader')) return
      if (modeSwitchLock.current) return
      modeSwitchLock.current = true
      if (m === 'workflow') {
        onToggleWorkAgentMode?.()
      } else if (m === 'custom') {
        // Custom 需有激活卡片才切换；无卡片由弹窗引导去配置（见弹窗逻辑）
        onSetMode?.('custom')
      } else {
        onSetMode?.('leader')
      }
      setTimeout(() => { modeSwitchLock.current = false }, 500)
    },
    [mode, onSetMode, onToggleWorkAgentMode],
  )

  // ── Custom 卡片选择：激活该卡片并切到 custom 模式 ──
  const selectCustomAgent = useCallback(
    (agentId: string) => {
      setModeMenuOpen(false)
      if (modeSwitchLock.current) return
      modeSwitchLock.current = true
      setActiveCustomAgent(agentId)
        .then(() => {
          setActiveCustomId(agentId)
          onSetMode?.('custom')
        })
        .catch(() => {})
        .finally(() => {
          setTimeout(() => { modeSwitchLock.current = false }, 500)
        })
    },
    [onSetMode],
  )
  useEffect(() => () => {
    if (modeMenuTimer.current) window.clearTimeout(modeMenuTimer.current)
  }, [])

  // ── 推理深度：model chip hover 时弹出选择（点击 model 仍是切换模型；hover 呈现强度档位）──
  const [modelEffortOpen, setModelEffortOpen] = useState(false)
  const modelEffortRef = useRef<HTMLDivElement>(null)
  const modelEffortTimer = useRef<number | null>(null)

  // 延迟关闭：光标从 chip 移动到弹窗的间隙时间内不丢失（悬停桥接）
  const openModelEffort = useCallback(() => {
    if (modelEffortTimer.current) window.clearTimeout(modelEffortTimer.current)
    setModelEffortOpen(true)
  }, [])
  const closeModelEffort = useCallback(() => {
    if (modelEffortTimer.current) window.clearTimeout(modelEffortTimer.current)
    modelEffortTimer.current = window.setTimeout(() => setModelEffortOpen(false), 160)
  }, [])

  // hover 移入容器（chip+菜单）时打开，移出时延迟关闭
  useEffect(() => {
    if (!modelEffortOpen) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setModelEffortOpen(false)
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [modelEffortOpen])

  useEffect(() => () => {
    if (modelEffortTimer.current) window.clearTimeout(modelEffortTimer.current)
  }, [])

  const selectEffort = useCallback(
    (val: string | null) => {
      setModelEffortOpen(false)
      onEffortChange(val)
    },
    [onEffortChange],
  )

  // 当前模型支持推理深度时（supportedEfforts 非空）才显示入口
  const effortAvailable = supportedEfforts.length > 0

  // WORKFLOW mode requires system_automation permission
  const workflowLocked =
    mode === 'workflow' && !!toolPermissions && !toolPermissions.system_automation

  // 自动调整 textarea 高度
  const autoResize = useCallback(() => {
    const ta = textareaRef.current
    if (!ta) return
    ta.style.height = 'auto'
    ta.style.height = Math.min(ta.scrollHeight, 280) + 'px'
  }, [textareaRef])

  // 同步 textareaRef（用于 autoResize 触发）
  useEffect(() => {
    if (textareaRef.current && !localTextareaRef) {
      setLocalTextareaRef(textareaRef.current)
    }
  }, [textareaRef, localTextareaRef])

  // input 或 voicePartial 变化时自动调整高度
  useEffect(() => {
    autoResize()
  }, [input, voicePartial, autoResize])

  // 执行中不禁用输入框——用户可随时输入消息（发送 = 追加指令，与移动端一致）
  const inputDisabled = workflowLocked

  // ── 拖拽文件：图片走 processImageAttachment，其他插路径 ──
  useEffect(() => {
    let unlisten: (() => void) | null = null
    let cancelled = false

    import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => {
        if (cancelled) return
        getCurrentWindow()
          .onDragDropEvent(event => {
            const type = event.payload.type
            if (type === 'enter' || type === 'over') {
              setIsDragOver(true)
            } else if (type === 'leave') {
              setIsDragOver(false)
            } else if (type === 'drop') {
              setIsDragOver(false)
              const paths = event.payload.paths as string[]
              if (paths && paths.length > 0) {
                // Check each path — image files are processed via processImageAttachment
                const { imagePaths, otherPaths } = partitionPaths(paths)
                if (imagePaths.length > 0) {
                  handleDroppedImages(
                    imagePaths,
                    inputRef.current,
                    onInputChangeRef.current,
                    setIsDragOver,
                  )
                }
                if (otherPaths.length > 0) {
                  for (const p of otherPaths) {
                    const name = p.split('\\').pop()?.split('/').pop() || p
                    onFileAttachRef.current?.({ path: p, name })
                  }
                }
              }
            }
          })
          .then(fn => {
            unlisten = fn
          })
      })
      .catch(err => {
        console.error('[DragDrop] 拖拽监听注册失败（Tauri window API 不可用）:', err)
      })

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    onInputChange(e.target.value)
    // auto-resize
    const ta = e.target
    ta.style.height = 'auto'
    ta.style.height = Math.min(ta.scrollHeight, 280) + 'px'
  }

  // ── 状态数据 ──
  const usage = mainTokenUsage || tokenUsage
  const ctxUsed = usage?.inputTokens || 0
  const ctxLimit = contextLimit || 128000
  const ctxPct = ctxLimit > 0 ? Math.min(ctxUsed / ctxLimit, 1) : 0
  const ctxColor = ctxPct > 0.8 ? '#ef4444' : ctxPct > 0.6 ? '#f59e0b' : '#22c55e'
  const cacheHit = usage?.cacheHitTokens || 0
  const cacheTotal = usage?.inputTokens || 0
  const cacheRate = cacheTotal > 0 ? (cacheHit / cacheTotal) * 100 : -1
  const cacheColor = cacheRate > 60 ? '#22c55e' : cacheRate > 30 ? '#f59e0b' : '#ef4444'
  const execTokens = (execTokenUsage?.inputTokens || 0) + (execTokenUsage?.outputTokens || 0)
  const moodColor = MOOD_COLORS[mood || 'idle'] || MOOD_COLORS.idle
  function fmt(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
    return String(n)
  }
  function fmtDur(ms: number): string {
    if (ms < 1000) return `${ms}ms`
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
    const m = Math.floor(ms / 60000)
    const s = Math.floor((ms % 60000) / 1000)
    return `${m}m ${s}s`
  }

  const placeholderText = workflowLocked
    ? 'WORKFLOW 需要打开全部安全权限，请点击右上角控制面板 → 权限与安全 → 勾选全部权限'
    : mode === 'workflow'
      ? '描述你需要的工作流...'
      : refineState
        ? t('input.placeholder.refine')
        : hints[hintIndex] || ''

  return (
    // 注意：不能注册 HTML5 级 onDragOver/onDrop + preventDefault——
    // Tauri(Windows, dragDropEnabled=true) 原生 DND 监听器与 HTML5 dragover/drop 互斥，
    // 页面 preventDefault 会阻止原生 onDragDropEvent 触发（见 tauri issue #15138）。
    // 拖放高亮由 Tauri 原生事件的 enter/over 驱动，无需 HTML5 事件。
    <div className={`chat-input-area${isDragOver ? ' drag-over' : ''}`}>
      <div className="chat-input-body">
        {/* ── Security Bar ── */}
        {security && (
          <SecurityPrompt
            tool={security.tool}
            risk={security.risk as 'low' | 'medium' | 'high' | 'critical'}
            reason={security.reason}
            actionId={security.actionId}
            onApprove={onApproveSecurity || (() => {})}
            onReject={onRejectSecurity || (() => {})}
          />
        )}
        {/* ── Reference bar: skill/knowledge/workflow chips + image pills ── */}
        <ReferenceBar
          references={pendingReferences || []}
          pendingImages={pendingImages || []}
          pendingFiles={pendingFiles || []}
          onRemoveReference={onRemoveReference || (() => {})}
          onRemoveImage={onRemoveImage || (() => {})}
          onRemoveFile={onRemoveFile || (() => {})}
        />
        {/* ── Input row: textarea + send button ── */}
        <div className="chat-input-row">
          <div className="chat-input-wrap">
            <div className="chat-input-edit">
              <textarea
                ref={textareaRef as React.LegacyRef<HTMLTextAreaElement>}
                className={`chat-input ${voicePartial ? 'voice-active ' : ''}${!refineState && !isProcessing ? (hintFade ? 'hint-visible' : 'hint-fade') : ''}`}
                placeholder={placeholderText}
                value={voicePartial || input}
                onChange={handleChange}
                onKeyDown={e => {
                  // 录音/识别中按 Enter = 说完发送：先冲刷语音会话再发送
                  if (e.key === 'Enter' && !e.shiftKey && !isProcessing && voiceRef.current?.isActive()) {
                    e.preventDefault()
                    flushThenSend()
                    return
                  }
                  onInputKeyDown(e)
                }}
                disabled={inputDisabled}
              />
              {voicePartial && (
                <span className="voice-mini-wave-input" aria-hidden>
                  <i />
                  <i />
                  <i />
                </span>
              )}
              <input
                type="file"
                ref={imageInputRef as React.RefObject<HTMLInputElement>}
                style={{ display: 'none' }}
                onChange={onFileSelect}
                accept="image/*"
              />
            </div>
          </div>
        </div>
        {/* ── 右下角操作组：+ 工具 / 语音 / 发送 固定在整个输入框右下角 ── */}
        <div className="input-actions">
          {/* ── 工具入口「+」：附件/图片/项目目录合并弹窗 ── */}
          <div className="input-tool-plus-wrap" ref={toolMenuRef}>
                <IconButton
                  variant="raw"
                  className="input-tool-plus-btn"
                  label={t('input.tools')}
                  title={t('input.tools')}
                  onClick={() => setToolMenuOpen(o => !o)}
                >
                  <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="M12 5v14" />
                    <path d="M5 12h14" />
                  </svg>
                </IconButton>
                {toolMenuOpen && (
                  <div className="input-tool-menu" role="menu">
                    <button
                      type="button"
                      className="input-tool-menu-item"
                      role="menuitem"
                      disabled={isProcessing && !pauseState}
                      onClick={async () => {
                        setToolMenuOpen(false)
                        const { open } = await import('@tauri-apps/plugin-dialog')
                        try {
                          const selected = await open({ multiple: true })
                          if (selected && Array.isArray(selected)) {
                            const paths = selected as string[]
                            onInputChange(
                              input + (input ? '\n' : '') + paths.map(p => `[附件: ${p}]`).join('\n'),
                            )
                          } else if (selected) {
                            onInputChange(input + (input ? '\n' : '') + `[附件: ${selected}]`)
                          }
                        } catch (e) {
                          console.error('文件选择失败:', e)
                        }
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
                        <path d="M15 3v11a5 5 0 1 1-10 0V7a2 2 0 1 1 4 0v5.5" />
                      </svg>
                      <span className="input-tool-menu-label">{t('input.attach')}</span>
                    </button>
                    <button
                      type="button"
                      className="input-tool-menu-item"
                      role="menuitem"
                      disabled={isProcessing && !pauseState}
                      onClick={() => {
                        setToolMenuOpen(false)
                        imageInputRef.current?.click()
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
                        <rect x="3" y="4" width="18" height="16" rx="3" />
                        <circle cx="8.5" cy="10" r="2.5" />
                        <path d="M3 16c4-3 6-2 8.5.5s5-3 9.5-1.5" />
                      </svg>
                      <span className="input-tool-menu-label">{t('input.image')}</span>
                    </button>
                    <button
                      type="button"
                      className="input-tool-menu-item"
                      role="menuitem"
                      disabled={isProcessing && !pauseState}
                      onClick={() => {
                        setToolMenuOpen(false)
                        onOpenProjectDir()
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
                        <path d="M3 7a2 2 0 0 1 2-2h4l2.5 2h7.5a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
                      </svg>
                      <span className="input-tool-menu-label">{t('input.projectDir')}</span>
                    </button>
                  </div>
                )}
              </div>
              <VoiceButton
                ref={voiceRef}
                onFinalText={handleVoiceFinal}
                onPartialText={setVoicePartial}
                // 与发送按钮一致：执行中（含 workflow 执行中）不禁用——
                // 语音转文字后发送 = 追加指令插入队列（Leader 与 Workflow 统一）。
                // 仅 workflow 权限锁定 / 暂停等待决策时禁用。
                disabled={workflowLocked || !!pauseState}
              />
              {/* 发送 / 终止按钮三态：
                  执行中（仅后端 is_busy 判断）且输入框无任何内容（含语音 partial）→ 终止按钮（可点，终止当前执行）；
                  执行中 + 有内容 → 发送按钮（追加指令）；
                  空闲 + 空内容 → 发送按钮灰显（待命）；
                  空闲 + 有内容 → 发送按钮高亮 */}
              {backendBusy && !input.trim() && !voicePartial ? (
                <IconButton
                  variant="input-send"
                  className="interrupt"
                  label={t('input.interrupt')}
                  title={t('input.interruptTitle')}
                  onClick={() => onInterrupt?.()}
                >
                  <IconSquare size={14} />
                </IconButton>
              ) : (
                <IconButton
                  variant={!isProcessing && input.trim() ? 'input-send-active' : 'input-send'}
                  className={isProcessing && !pauseState ? 'processing' : ''}
                  label={
                    isProcessing
                      ? input.trim() || voicePartial
                        ? '发送（追加指令）'
                        : t('input.send')
                      : t('input.send')
                  }
                  onClick={() => {
                    // 单一发送语义（Leader 与 Workflow 一致）：空闲 = 新执行；执行中 = 追加指令（下一轮生效）。
                    // 无暂停按钮/暂停弹窗——执行控制已从输入栏移除（与手机端一致）。
                    // workflow 执行中也不禁用：发送 = 追加指令插入队列，由 workflow_agent 迭代边界注入。
                    if (!isProcessing) flushThenSend()
                    else if (input.trim() || voicePartial) flushThenSend()
                  }}
                  disabled={
                    workflowLocked ||
                    !input.trim() ||
                    !!pauseState
                  }
                  title={
                    workflowLocked
                      ? 'WORKFLOW 需要打开全部安全权限'
                      : isProcessing
                        ? input.trim() || voicePartial
                          ? '执行中发送 = 追加指令，立即纳入当前任务'
                          : '执行中请先输入内容，发送 = 追加指令'
                        : t('input.send')
                  }
                >
                  <IconSend size={14} />
                </IconButton>
              )}
            </div>

        {/* ── 统一底栏：全部 flat 文字 + flat 图标，同一视觉语言 ── */}
        <div className="input-bar">
          <div className="input-bar-left">
            {/* ── mode 切换：始终显示；执行时叠加状态点 + 背景呼吸，禁用切换 ── */}
            <div
              className="input-bar-mode-wrap"
              ref={modeMenuRef}
              onMouseEnter={openModeMenu}
              onMouseLeave={closeModeMenu}
            >
              <span
                className={`input-bar-chip mode-${mode === 'workflow' ? 'workflow' : mode === 'custom' ? 'custom' : 'leader'}${isProcessing ? ' is-processing' : ''}`}
                onClick={isProcessing ? undefined : () => setModeMenuOpen(o => !o)}
              >
                {isProcessing && (
                  <span className="input-bar-status-dot" style={{ color: moodColor }} />
                )}
                {mode === 'custom' ? (
                    <>
                      <IconSparkles size={13} />
                      <span className="input-bar-mode-text">
                        {(activeCustom?.name || 'CUSTOM').toUpperCase()}
                      </span>
                    </>
                  ) : mode === 'workflow' ? (
                    <>
                      <IconWorkflow size={13} />
                      <span className="input-bar-mode-text">WORKFLOW</span>
                    </>
                  ) : (
                    <>
                      <IconBrain size={13} />
                      <span className="input-bar-mode-text">LEADER</span>
                    </>
                  )}
                </span>
                {!isProcessing && modeMenuOpen && (
                  <div className="input-bar-mode-menu">
                    <div
                      className={`input-bar-mode-option ${mode !== 'workflow' && mode !== 'custom' ? 'active' : ''}`}
                      onClick={() => selectMode('leader')}
                    >
                      <span className="input-bar-mode-option-name mode-leader">Leader</span>
                      <span className="input-bar-mode-option-desc">
                        {t('input.mode.leader.desc')}
                      </span>
                    </div>
                    <div
                      className={`input-bar-mode-option ${mode === 'workflow' ? 'active' : ''}`}
                      onClick={() => selectMode('workflow')}
                    >
                      <span className="input-bar-mode-option-name mode-workflow">Workflow</span>
                      <span className="input-bar-mode-option-desc">
                        {t('input.mode.workflow.desc')}
                      </span>
                    </div>
                    {/* ── Custom 档：列出卡片（点击激活+切换）；无卡片引导创建 ── */}
                    {customAgents.length > 0 ? (
                      <>
                        {customAgents.map(agent => (
                          <div
                            key={agent.id}
                            className={`input-bar-mode-option ${mode === 'custom' && agent.id === activeCustomId ? 'active' : ''}`}
                            onClick={() => selectCustomAgent(agent.id)}
                          >
                            <span className="input-bar-mode-option-name mode-custom">
                              {agent.name}
                            </span>
                            <span className="input-bar-mode-option-desc">
                              {t('input.mode.custom.desc')}
                            </span>
                          </div>
                        ))}
                        {onManageCustomAgents && (
                          <div
                            className="input-bar-mode-manage"
                            onClick={() => {
                              setModeMenuOpen(false)
                              onManageCustomAgents()
                            }}
                          >
                            {t('input.mode.custom.manage')}
                          </div>
                        )}
                      </>
                    ) : (
                      <div
                        className="input-bar-mode-option"
                        onClick={() => {
                          setModeMenuOpen(false)
                          onManageCustomAgents?.()
                        }}
                      >
                        <span className="input-bar-mode-option-name mode-custom">
                          Custom
                        </span>
                        <span className="input-bar-mode-option-desc">
                          {t('input.mode.custom.create')}
                        </span>
                      </div>
                    )}
                  </div>
                )}
              </div>
            {/* ── workflow 桌面工具箱按钮（Ctrl+U）：仅 workflow 模式显示，
                切换 Ctrl+U 工具箱显示；active 态表示工具箱当前打开。
                hover 提示：自定义浮层（复用输入栏弹窗样式），不用原生 title ── */}
            {mode === 'workflow' && (
              <div
                className="input-bar-toolbox-wrap"
                onMouseEnter={() => setShowToolboxTip(true)}
                onMouseLeave={() => setShowToolboxTip(false)}
              >
                <IconButton
                  variant="raw"
                  className={`input-bar-toolbox-btn${showDesktopToolbar ? ' is-active' : ''}`}
                  label="桌面工具箱 (Ctrl+U)"
                  onClick={onToggleDesktopToolbar}
                >
                  <IconWrench size={13} />
                </IconButton>
                {showToolboxTip && (
                  <div className="input-bar-toolbox-tip">
                    <span className="input-bar-toolbox-tip-text">Ctrl+U 工具箱</span>
                  </div>
                )}
              </div>
            )}
            {/* ── model chip：点击切换模型；hover 弹出推理强度选择（默认/low/high/max）── */}
            <div
              className="input-bar-model"
              ref={modelEffortRef}
              onMouseEnter={openModelEffort}
              onMouseLeave={closeModelEffort}
            >
              <span
                className="input-bar-text input-bar-chip"
                onClick={onModelSwitch}
              >
                {modelLabel || modelName || '—'}
                {effortAvailable && (
                  <svg
                    className="input-bar-effort-caret"
                    aria-hidden
                    width="9"
                    height="9"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2.5"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="m6 9 6 6 6-6" />
                  </svg>
                )}
              </span>
              {effortAvailable && modelEffortOpen && (
                <div className="input-bar-effort-menu">
                  <div
                    className={`input-bar-effort-option ${!effort ? 'active' : ''}`}
                    onClick={() => selectEffort(null)}
                  >
                    {t('models.reasoningDefault')}
                  </div>
                  {supportedEfforts.map(e => (
                    <div
                      key={e}
                      className={`input-bar-effort-option ${effort === e ? 'active' : ''}`}
                      onClick={() => selectEffort(e)}
                    >
                      {e}
                      {e === 'high' ? ` (${t('models.reasoningHighHint')})` : ''}
                    </div>
                  ))}
                </div>
              )}
            </div>
            {/* ── 状态：唯一常驻 ctx，迷你进度条 + hover 弹窗详情 ── */}
            <span
              className="input-bar-ctx"
              onMouseEnter={openCtx}
              onMouseLeave={closeCtx}
            >
              <span className="input-bar-ctx-label">ctx</span>
              <span className="input-bar-ctx-gauge" aria-hidden>
                {Array.from({ length: 5 }).map((_, i) => (
                  <span
                    key={i}
                    className="input-bar-ctx-gauge-cell"
                    style={
                      i < Math.round(ctxPct * 5)
                        ? { background: ctxColor }
                        : undefined
                    }
                  />
                ))}
              </span>
              <span className="input-bar-ctx-pct" style={{ color: ctxColor }}>
                {Math.round(ctxPct * 100)}%
              </span>
              {ctxHover && (
                <span className="input-bar-ctx-detail">
                  {cacheRate >= 0 && (
                    <span className="input-bar-ctx-row">
                      <span className="input-bar-ctx-detail-label">cache</span>
                      <span>{cacheRate.toFixed(0)}%</span>
                    </span>
                  )}
                  <span className="input-bar-ctx-row">
                    <span className="input-bar-ctx-detail-label">tok</span>
                    <span>{fmt(execTokens)}</span>
                  </span>
                  <span className="input-bar-ctx-row">
                    <span className="input-bar-ctx-detail-label">step</span>
                    <span>{totalCalls || 0}</span>
                  </span>
                  <span className="input-bar-ctx-row">
                    <span className="input-bar-ctx-detail-label">time</span>
                    <span>{fmtDur(liveDuration)}</span>
                  </span>
                </span>
              )}
            </span>
          </div>
        </div>
      </div>
    </div>
  )
}