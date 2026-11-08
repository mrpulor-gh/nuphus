import {
  Fragment,
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  Archive as ArchiveIcon,
  Maximize as MaximizeIcon,
  Minimize as MinimizeIcon,
  Share2 as ShareIcon,
} from 'lucide'
import { MorphIcon } from 'morphicons/react'
import {
  IconArrowRight,
  IconBookOpen,
  IconBot,
  IconBrowser,
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconChevronUp,
  IconClock3,
  IconFolder,
  IconGrip,
  IconLoader2,
  IconPaperclip,
  IconPlus,
  IconPuzzle,
  IconSave,
  IconSend,
  IconSquare,
  IconTrash2,
  IconWrench,
  IconX,
} from '../../ui/Icons'
import { IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'
import { ProviderIcon, hasProviderIcon } from '../components/ProviderIcon'
import {
  getProjectBookmarks,
  getProjectDir,
  hudUpdate,
  listModels,
  sideChatCancel,
  sideChatAppend,
  sideChatHistoryDelete,
  sideChatHistoryList,
  sideChatPrepare,
  sideChatRemoveAppendItem,
  sideChatReset,
  sideChatSave,
  sideChatSend,
  sideChatState,
} from '../lib/api'
import {
  loadSideChatMessages,
  loadSideChatPrefs,
  saveSideChatMessages,
  saveSideChatPrefs,
} from './sideChatStorage'
import MarkdownContent from './MarkdownContent'
import { useStickyScroll } from '../../hooks/useStickyScroll'
import type { ModelInfo, ProjectBookmark, SideChatHistoryEntry } from '../lib/api'
import type { ChatReference } from '../../core/types'
import { formatTokens } from '../../ui/tokenFormat'
import { formatToolParams } from '../../ui/toolParams'
import './side-chat.css'

/**
 * 「Explore」哨兵：项目选择器的内置默认项，**不属于任何项目路径**。
 *
 * 与空串（跟随主会话）的分界：空串不注入工作目录，模型沿用系统提示环境段里
 * 主会话的全局 project_dir；哨兵让后端生成「本轮无项目」段，把环境段那一行
 * 覆盖掉——这才配得上「不属于任何项目」六个字，否则它只是跟随主会话的复制品。
 *
 * 与后端 `src-tauri/src/commands/side_chat.rs` 的同名常量一一对应，改一处
 * 必须同步改另一处。
 */
export const EXPLORE_PROJECT = '__explore__'

/** 面板尺寸下限（px）：再窄多列结论就没法读了。上限永远相对宿主容器实测。 */
const PANEL_SIZE_MIN = { w: 320, h: 280 }

/**
 * 后端「该临时会话正在执行上一条消息」的镜像文案
 * （`src-tauri/src/commands/side_chat.rs` 的 `BUSY_MESSAGE`）。
 *
 * 命中它 = **后端说在途**。`slot.running` 是在途的唯一权威，前端 `sending`
 * 只是它的投影——投影落后于事实时（派发起点事件未到 / 监听漏事件 / 刷新恢复
 * 竞态），发送应当降级为追加，而不是给用户一个错误气泡。
 * 两侧文案由源码断言测试锁定，改一侧必须双改。
 */
const BACKEND_BUSY_MESSAGE = '该临时会话正在执行上一条消息，请稍候'

interface SideChatStep {
  tool: string
  ok: boolean
  summary: string
  /** 工具调用耗时（ms）。旧存档 / 上游未下发时缺失，界面与导出都做降级 */
  durationMs?: number
}

interface SideChatMessage {
  role: 'user' | 'assistant'
  text: string
  /** 本轮工具调用步骤：渲染成「已用 N 个工具」折叠条，点击展开明细 */
  steps?: SideChatStep[]
  /** 思考内容：深度思考过程文本 */
  thinking?: string
  /** 失败提示单独成样式，避免和正常回复混淆 */
  error?: boolean
  /** 失败的可操作建议（由 explainSideChatError 生成，随 error 消息一起落） */
  errorHint?: string
  /** 原始错误详情：默认折叠，只给需要排查的人看 */
  errorDetail?: string
  /** 执行中追加的消息：已入队、尚未被迭代边界消费。
   *  渲染成半透明气泡 + 「已入队」角标，收到 append_queue_updated 后转 normal。 */
  queued?: boolean
  /** 该轮结束时的 ctx 占用（token，最近一次 API 的 input+output）。
   *  与主会话输入框同口径；0 / 缺失 = 尚无官方读数，界面显示 `--`。 */
  ctxUsed?: number
  /** 轮次 id：刷新恢复时据此判重——正常返回、完成事件、state 回填三条路径
   *  可能都带同一轮结果，没有这个键就会落成重复气泡。 */
  turnId?: string
  /** 本轮块序列（正文 / 思考 / 工具调用按发生顺序）。有它就走交错渲染，
   *  没有（历史消息 / 流式事件缺失）退回「正文整段 + 摘要条」。 */
  blocks?: MsgBlock[]
  /** 消息来源：Leader 经后端通道派发的 brief（伙伴窗口显式展示派发任务，
   *  避免派发轮对用户是黑盒）。普通用户轮次无此字段。 */
  from?: 'leader_dispatch'
}

/** 失败归因分类：只做「原因 + 下一步」，原始错误串另存折叠详情。
 *  分类与后端 `is_retryable_llm_error` 的不可重试清单对齐——余额/鉴权类重发无意义。 */
export type SideChatErrorKind =
  'balance' | 'auth' | 'forbidden' | 'rate' | 'timeout' | 'network' | 'context' | 'generic'

export function explainSideChatError(detail: string): SideChatErrorKind {
  const d = detail.toLowerCase()
  if (/402|insufficient|balance|quota|credit|payment/.test(d)) return 'balance'
  if (/401|unauthorized|invalid api key|invalid_api_key|incorrect api key/.test(d)) return 'auth'
  if (/403|forbidden/.test(d)) return 'forbidden'
  if (/429|rate limit|too many requests/.test(d)) return 'rate'
  if (/timeout|timed out/.test(d)) return 'timeout'
  if (/connect|network|dns|tls|econn|socket/.test(d)) return 'network'
  if (/context length|too long|max tokens/.test(d)) return 'context'
  return 'generic'
}

/** 输入区附件 chip：name 显示、path 注入（`[附件: path]` 文本行） */
interface SideChatAttachment {
  name: string
  path: string
}

/** 实时过程态：在途工具调用（call_id → 工具名 + 一行摘要） */
interface LiveCall {
  tool: string
  summary: string
}

/** 本轮一条工具记录：start 入列，end 把 ok 从 undefined 改成 true/false。
 *
 * 与 LiveCall 的区别是它**不随 end 消失**——状态行要能回答「刚刚用了什么工具」。
 * 只用在途集合（activeCalls）是不够的：同步工具毫秒级返回，start 与 end 会被
 * React 18 合批成同一次渲染，Map「加了又删」净值为空，工具行永远不显示。 */
interface LiveToolEntry {
  callId: string
  tool: string
  summary: string
  /** undefined = 在途；true / false = 已结束（成功 / 失败） */
  ok?: boolean
  /** 工具调用耗时（ms），来自 tool_call_end 事件的 duration_ms；未知则为 undefined */
  durationMs?: number
}

/** 流式文本有序分段：思考 / 正文按到达顺序排列（迭代间不合并、不跨类型
 *  倒手拼接）——修复「思考与正文混在同一容器、丢失轮次顺序」。渲染时逐段
 *  上色：思考段弱化斜体、正文段常规色。 */
interface LiveSegment {
  kind: 'thinking' | 'text'
  text: string
}

const EMPTY_LIVE_SEGMENTS: LiveSegment[] = []

/** 工具耗时格式化：<1s 用 ms（如 `312ms`），否则用秒（如 `1.4s`）。
 *  不用 toFixed(1) 统一到秒——几十毫秒的工具会显示成 `0.0s`，丢掉精度。 */
function formatDuration(ms: number | undefined): string {
  if (typeof ms !== 'number' || !Number.isFinite(ms) || ms < 0) return ''
  return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`
}

/** 一轮内**按时间顺序**的块：正文 / 思考 / 工具调用交替出现。
 *  与 `steps` 的分工：steps 是集合（只有用了什么），块序列保留「这一步发生在
 *  哪段话之后」——这是把工具块渲染回流式位置的唯一依据。 */
type LiveBlock =
  | { kind: 'text'; text: string }
  | { kind: 'thinking'; text: string }
  | {
      kind: 'tool'
      tool: string
      summary: string
      /** 原始参数：执行过程按移动端口径渲染 command / cwd / path 等定位字段 */
      params?: unknown
      ok?: boolean
      callId?: string
      durationMs?: number
    }

/** 落盘用的块：正文只存**字符区间**（指向 `msg.text`），不与正文重复占存储。
 *  text 块在完成时按流式累积长度切分，切点天然落在工具调用发生处。 */
type MsgBlock =
  | { kind: 'text'; from: number; to: number }
  | { kind: 'thinking'; text: string }
  | { kind: 'tool'; tool: string; ok: boolean; summary: string }

/** 流式块 → 持久化块。把连续正文映射成 `msg.text` 上的区间。
 *
 *  两种降级都要保住内容：
 *  - 回复与流式累积不一致（事件丢包 / provider 非流式）→ 正文整块 + 工具块附后；
 *  - 一个文本块都没产出 → 同上，绝不留空气泡。 */
function buildMsgBlocks(reply: string, live: LiveBlock[]): MsgBlock[] {
  if (live.length === 0) return []
  const streamed = live
    .filter(b => b.kind === 'text')
    .map(b => (b as { text: string }).text)
    .join('')
  const toolsOnly = (): MsgBlock[] =>
    live
      .filter(b => b.kind === 'tool')
      .map(b => {
        const t = b as Extract<LiveBlock, { kind: 'tool' }>
        return { kind: 'tool' as const, tool: t.tool, ok: t.ok === true, summary: t.summary }
      })
  if (streamed !== reply && !(streamed && reply.startsWith(streamed))) {
    const head: MsgBlock[] = reply ? [{ kind: 'text', from: 0, to: reply.length }] : []
    return [...head, ...toolsOnly()]
  }
  let cursor = 0
  const out: MsgBlock[] = []
  for (const b of live) {
    if (b.kind === 'text') {
      const len = b.text.length
      if (len > 0) out.push({ kind: 'text', from: cursor, to: cursor + len })
      cursor += len
    } else if (b.kind === 'thinking') {
      const text = b.text.trim()
      if (text) out.push({ kind: 'thinking', text })
    } else {
      out.push({ kind: 'tool', tool: b.tool, ok: b.ok === true, summary: b.summary })
    }
  }
  // 正文尾块：流式没收全（截断 / 末帧未到）时补齐，否则后半段会消失
  if (cursor < reply.length) out.push({ kind: 'text', from: cursor, to: reply.length })
  if (!out.some(b => b.kind === 'text') && reply) {
    out.unshift({ kind: 'text', from: 0, to: reply.length })
  }
  return out
}

/**
 * 轻量 tips：纯 CSS 驱动（hover / focus-within 即显）——原生 `title` 有约 1s
 * 响应延迟，header 图标与消息操作条都点名要快的，且原生灰框不可控样式。
 *
 * 三条防呆约束（.scp-panel 是 overflow:hidden，.scp-messages 是滚动容器）：
 * · **方向跟着图标位置走**（place）：header 在面板顶部 → 向下展开（place-below，
 *   向上必探出面板上缘被裁）；消息操作条在列表尾部 → 向上展开（place-above，
 *   向下会顶到滚动视口底缘被裁）；
 * · **宽度内容定宽**（width:max-content + 220px 上限）：包含块只有 28px 的
 *   图标盒，不显式定宽气泡会被压成贴图标的细条；
 * · **水平锚定看分组**（align）：右组图标贴面板右缘 → 右对齐、文字向左展开；
 *   左组元素（拖拽手柄 / 项目）→ 左对齐、文字向右，否则会探出面板左缘被裁。
 * · text 传空串 = 不渲染气泡（浮层已打开时避免 tip 盖住浮层）。
 */
function IconTip({
  text,
  align = 'right',
  place = 'below',
  children,
}: {
  text: string
  align?: 'left' | 'right'
  place?: 'below' | 'above'
  children: ReactNode
}) {
  return (
    <span className={`scp-tip align-${align} place-${place}`}>
      {children}
      {text && (
        <span className="scp-tip-bubble" role="tooltip">
          {text}
        </span>
      )}
    </span>
  )
}

/** 流式正文/思考并入块序列：与 streamSegments 同一套幂等合并规则
 *  （同类型末块合并、快照式重发替换、重复投递丢弃）。 */
function appendLiveText(blocks: LiveBlock[], kind: 'text' | 'thinking', text: string): LiveBlock[] {
  const last = blocks[blocks.length - 1]
  if (!last || last.kind !== kind) return [...blocks, { kind, text }]
  if (text === last.text || last.text.endsWith(text)) return blocks
  const merged = text.startsWith(last.text) ? text : last.text + text
  return [...blocks.slice(0, -1), { kind, text: merged }]
}

/** 工具摘要取参键优先级：与后端 step_summary（side_chat.rs）同规则 */
const SUMMARY_KEYS = ['path', 'file_path', 'query', 'url', 'pattern', 'name']

/** 一步的一行摘要：取参数字符串里最像「目标」的那个，截 80 字符。 */
function liveSummary(params: unknown): string {
  if (typeof params === 'string') return params.slice(0, 80)
  if (params && typeof params === 'object') {
    const map = params as Record<string, unknown>
    for (const k of SUMMARY_KEYS) {
      const v = map[k]
      if (typeof v === 'string') return v.slice(0, 80)
    }
    for (const v of Object.values(map)) {
      if (typeof v === 'string') return v.slice(0, 80)
    }
  }
  return ''
}

/** 会话 → md 存档（保存 / 分享共用同一份纯函数）。 */
export function buildSessionMarkdown(
  messages: SideChatMessage[],
  modelName: string | undefined,
): string {
  const now = new Date()
  const pad = (n: number) => String(n).padStart(2, '0')
  const stamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`
  const lines: string[] = [
    '# 临时伙伴存档',
    `> 时间：${stamp} · 模型：${modelName || '跟随主会话'}`,
    '',
  ]
  for (const msg of messages) {
    lines.push(msg.role === 'user' ? '## 大王' : '## 临时助手')
    if (msg.thinking) {
      lines.push('')
      lines.push('<details><summary>思考过程</summary>')
      lines.push(msg.thinking)
      lines.push('</details>')
    }
    lines.push(msg.text)
    const steps = msg.steps ?? []
    if (steps.length > 0) {
      lines.push('')
      lines.push(`<details><summary>工具步骤（${steps.length}）</summary>`)
      for (const s of steps) {
        // 与界面同口径：文字状态 + 耗时，不用 ✓/✗（导出物常被贴进文档 /
        // issue，emoji 在不渲染它的环境里会变成乱码方块）
        const state = s.ok ? '成功' : '失败'
        const dur = formatDuration(s.durationMs)
        lines.push(`- ${s.tool} ${state}${dur ? ` ${dur}` : ''}${s.summary ? ` ${s.summary}` : ''}`)
      }
      lines.push('</details>')
    }
    lines.push('')
  }
  return lines.join('\n')
}

/** ctx 状态色：与主会话输入框同一组阈值与色值（>80% 红 / >60% 黄 / 其余绿）。
 *  抽成纯函数是为了两处共用同一口径——颜色分别写会让「同一个 75%」在两边
 *  呈现不同颜色，用户无法横向对比。
 *  cap / used 任一为 0（无官方读数）→ 中性灰，不假装有进度。 */
function ctxToneOf(used: number, cap: number): string {
  if (cap <= 0 || used <= 0) return 'var(--fg-3)'
  const pct = used / cap
  return pct > 0.8 ? '#ef4444' : pct > 0.6 ? '#f59e0b' : '#22c55e'
}

interface SideChatPanelProps {
  /**
   * 后端临时会话 key。由父组件在面板打开时生成（crypto.randomUUID），
   * 关闭 / 切换主会话时随 side_chat_close 一并销毁——本组件不自己生成，
   * 否则父组件在会话切换时拿不到 key 就无法销毁。
   */
  sessionKey: string
  /** 当前主会话模型名：临时伙伴默认跟随；输入栏可手选模型覆盖（空 = 跟随） */
  modelName: string | undefined
  /** 关闭面板（父组件负责调 side_chat_close 销毁后端会话） */
  onClose: () => void
  /** 收起为最小化条：不销毁会话、不清空消息，父组件只切显示态 */
  onMinimize?: () => void
  /** 受控最小化态（true = 只露标题栏，消息与输入区隐藏但保留） */
  minimized?: boolean
  /**
   * 「送入主对话」：把该条助手回复**全文**交给父组件入主会话输入栏
   * （父组件构造 sidechat 引用 chip；前端 chip 只截断显示，注入是全文）。
   */
  onSendToMain: (text: string) => void
  /**
   * 「直接提交」：把该条助手回复**全文**经主会话共享提交入口直接提交
   * （父组件走 send_message_cmd，source="sidechat"）。与 onSendToMain
   * 的分工：后者进输入栏可改可删；这里一键提交。这是用户手动强制的跨会话
   * 通道，任何轮次都能用；另一条自动交付由后端按派发态判定（仅主会话正在
   *   派发执行任务时），没有用户开关。
   */
  onSubmitToMain: (text: string) => void
  /**
   * 主会话正在执行的 Exec 任务标题（父组件从 task_runs 快照取第一个 running
   * 条目的 title）。null / 空 = 无在途 Exec，不渲染提示。只做细提示展示——
   * 侧边没有暂停 / 抢占 Exec 的任何机制（隔离边界）。
   */
  mainExecTitle?: string | null
  /** 在途执行态变化上报：父组件据此切左下角入口角标（执行中 / 空闲） */
  onSendingChange?: (sending: boolean) => void
}

/**
 * 临时伙伴（Side Chat）面板：复用 ExecAgent 的一次性临时执行体。
 *
 * 边界：不落库、不写记忆、不进 Leader 上下文；系统操作类工具在后端恒关
 * （前端不提供开关，只做只读说明）。
 *
 * 实时过程：执行中经独立频道 'side-chat-event'（按 sessionKey 过滤）接收
 * ToolCallStart/End 与 LlmTextDelta，渲染「当前工具 + 流式文本」；完成后
 * 清空实时态，落最终回复与 steps 折叠条。
 */
function SideChatPanelImpl({
  sessionKey,
  modelName,
  onClose,
  onMinimize,
  minimized = false,
  onSendToMain,
  onSubmitToMain,
  mainExecTitle,
  onSendingChange,
}: SideChatPanelProps) {
  const { t } = useLanguage()
  // 刷新续命：消息随 sessionKey 落 localStorage（webview 刷新不重载 Rust 进程，
  // 后端会话与执行体仍活着；挂回同一个 key 即无缝继续。整机重启后只剩 UI 消息，
  // 见 sideChatStorage 头注的边界声明）
  const [messages, setMessages] = useState<SideChatMessage[]>(() =>
    // 落盘的消息恢复时清掉 queued 标记：刷新前那轮执行要么已经结束（队列被
    // 后端清空），要么执行体随进程消失。留着一个「已入队」角标会让用户以为
    // 那句还在等注入，实际永远不会生效。
    loadSideChatMessages<SideChatMessage>(sessionKey).map(m =>
      m.queued ? { ...m, queued: false } : m,
    ),
  )
  useEffect(() => {
    saveSideChatMessages(sessionKey, messages)
  }, [sessionKey, messages])
  // 偏好持久化：模型 / 推理强度 / 项目路径。
  // 此前只有 key / min / 消息落了盘，这三项是裸 state——刷新（webview 重载、
  // Rust 进程不重建）后全部打回默认，模型每次都被打回「跟随主会话」。
  // 与消息的分界：消息是会话级（关闭即清），偏好是用户级（clearSideChatStorage
  // 刻意不清 PREFS）——关面板、切主会话都不该丢掉用户选好的模型。
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  /** sending 的同步镜像：React state 更新有批处理窗口，快速连击时第二次
   *  handleSend 闭包里 sending 仍是旧值 false，会让乐观更新的用户消息被 push
   *  两条（后端 BUSY 只拦真实发送，拦不住前端已渲染的重复气泡）。ref 立即
   *  生效，作为 push 前的硬闸门——见 handleSend 开头与各 sending 收尾点。 */
  const sendingRef = useRef(false)
  // 工具权限不在本面板提供开关：全关会让面板失去全部功能，且「系统与桌面
  // 自动化」在只读 shell 子集放开后已是误导性表述。固定开，由后端
  // ToolPermissions 决定实际可用面。
  const fileAccess = true
  const webSearch = true
  /** 模型选择：空 = 跟随主会话当前模型（发送时回落到 modelName）。
   *  初值取持久化偏好——刷新后不再被打回「跟随主会话」。 */
  const [model, setModel] = useState(() => loadSideChatPrefs().model)
  /** 选中模型所属提供商：与 model 成对持久化。同名模型跨提供商时，只靠 id
   *  后端会按段序首段猜（曾连到本地代理 8317），必须成对传。 */
  const [modelProvider, setModelProvider] = useState(() => loadSideChatPrefs().provider)
  const [models, setModels] = useState<ModelInfo[]>([])
  /** 模型弹窗：打开中 + 当前展开的提供商（手风琴，一次一个） */
  const [modelPickerOpen, setModelPickerOpen] = useState(false)
  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)
  /** 推理强度覆盖：null = 提供商默认；入口仅当模型声明 reasoning_efforts 时出现。
   *  同样取持久化偏好（换模型时 pickModel 会把它重置为 null）。 */
  const [effort, setEffort] = useState<string | null>(() => loadSideChatPrefs().effort)
  const [effortOpen, setEffortOpen] = useState(false)
  /** 面板内选定的项目目录（绝对路径）；空 = 跟随主会话项目；
   *  EXPLORE_PROJECT = 内置的「Explore」项，不属于任何项目路径。
   *  与全局 project_dir 分离：临时伙伴常被用来「顺手看别的项目」，
   *  不应改写主会话的目录注入。选择随每条消息下发（side_chat_send 的 projectDir）。 */
  const [projectPath, setProjectPath] = useState(() => loadSideChatPrefs().projectPath)
  /** 项目中心书签（已设项目）：数据源与项目页同一份，过滤掉已归档文件夹 */
  const [projects, setProjects] = useState<ProjectBookmark[]>([])
  /** 主会话当前项目（用于「跟随主会话」这一项的副标题与兜底判定） */
  const [mainProject, setMainProject] = useState<{ name: string; path: string }>({
    name: '',
    path: '',
  })
  const [projectPickerOpen, setProjectPickerOpen] = useState(false)
  const modelPickerRef = useRef<HTMLDivElement>(null)
  const effortRef = useRef<HTMLDivElement>(null)
  const projectRef = useRef<HTMLDivElement>(null)
  /** 面板尺寸（px）：null = 默认几何（CSS 的 width + top/bottom 推导高度）。
   *  边缘拉伸 / 一键放大都会写这里并落盘，刷新后保持。 */
  const [panelSize, setPanelSize] = useState<{ w: number; h: number } | null>(
    () => loadSideChatPrefs().size,
  )
  const panelRef = useRef<HTMLElement>(null)
  const [resizing, setResizing] = useState(false)
  /** 交付箱积压条数（后端 Finalizing 拒收后落箱、尚未补投的结论）。
   *  只随 sideChatState 查询刷新（挂载 / 轮次结束时各一次），不做实时订阅。 */
  const [pendingDeliveries, setPendingDeliveries] = useState(0)

  // 偏好落盘：必须排在上面几个 state 之后，否则依赖数组渲染期求值 → TDZ 白屏
  // （权限两字段已改为常量，不再参与持久化）
  useEffect(() => {
    // 权限两字段已改为常量、不参与持久化；这里以已存偏好打底，避免把它们写丢
    saveSideChatPrefs({
      ...loadSideChatPrefs(),
      model,
      provider: modelProvider,
      effort,
      projectPath,
      size: panelSize,
    })
  }, [model, modelProvider, effort, projectPath, panelSize])

  // 项目选项：书签表（已设项目）+ 主会话当前项目。两者都失败时选择器只剩
  // 「跟随主会话」一项，不影响收发消息。
  useEffect(() => {
    let alive = true
    void getProjectBookmarks()
      .then(list => {
        if (alive) setProjects(list.filter(b => !b.archived && Boolean(b.path)))
      })
      .catch(() => {
        /* 项目中心不可用：保持空列表，选择器退化为仅「跟随主会话」 */
      })
    void getProjectDir()
      .then(state => {
        if (alive) setMainProject({ name: state.name, path: state.path })
      })
      .catch(() => {
        /* 同上 */
      })
    return () => {
      alive = false
    }
  }, [sessionKey])

  // ── 打开面板即建壳：同步面板当前配置（大王 2026-11-04 定）──
  // 壳存在 = Leader 派发的前置（选壳判据是「壳存在」，不是「跑过一轮 send」）。
  // 面板一挂载就建壳，用户不需要先和伙伴对话；配置跟随面板变化（换模型 /
  // effort / 权限后壳同步更新），派发轮拿到的始终是面板当前配置。
  useEffect(() => {
    void sideChatPrepare(
      sessionKey,
      fileAccess,
      webSearch,
      model || modelName || null,
      effort,
      model ? modelProvider || null : null,
    ).catch(() => {
      /* bridge 不可用（mock / 无 Tauri）→ 静默；真机恒可用 */
    })
  }, [sessionKey, model, modelProvider, effort, modelName, fileAccess, webSearch])

  // ── 刷新恢复：invoke 的 Promise 随页面重载消失，Rust 侧执行体却还在跑 ──
  // 缺这一步的后果（实测缺陷）：刷新后正在流式的那条气泡消失，`sending` 被打回
  // false，实时事件即便继续到达也不渲染，最终回复只走 invoke 返回值 → 永远丢。
  // 两件事分头补：在途态（恢复过程区）+ 已完成但没送达的回复（按 turn_id 判重）。
  // 第三件：交付箱积压计数（Finalizing 拒收落箱的结论），供「N 条待交付」提示。
  useEffect(() => {
    let alive = true
    void sideChatState(sessionKey)
      .then(st => {
        if (!alive || !st) return
        if (st.running) {
          sendingRef.current = true
          setSending(true)
        }
        // 交付箱计数：后端未下发 / 非数字按 0 处理（旧版本无此字段）
        setPendingDeliveries(typeof st.pending_deliveries === 'number' ? st.pending_deliveries : 0)
        const t = st.last_turn
        if (!t?.turn_id) return
        setMessages(prev =>
          prev.some(m => m.role === 'assistant' && m.turnId === t.turn_id)
            ? prev
            : [
                ...prev,
                {
                  role: 'assistant',
                  text: t.reply,
                  steps: Array.isArray(t.steps) ? t.steps : [],
                  ctxUsed: t.ctx_used,
                  turnId: t.turn_id,
                },
              ],
        )
      })
      .catch(() => {
        /* bridge 不可用 / 整机重启后壳已不存在：按空态处理，不打扰用户 */
      })
    return () => {
      alive = false
    }
  }, [sessionKey])

  // ── 实时过程态（'side-chat-event'，按 sessionKey 过滤）──
  /** 在途工具调用：call_id → 工具名 + 一行摘要（end 即移除；只用于并行计数） */
  const [activeCalls, setActiveCalls] = useState<Map<string, LiveCall>>(() => new Map())
  /** 本轮工具轨迹：start 入列，end 只把 ok 从 undefined 改成 true/false，**不移除**。
   *
   *  对齐主通道 timeline 的做法（useEvents.ts 的 tool_call_start / tool_call_end：
   *  追加 + 原地改状态）。没有它，同步工具的在途窗口会被合批吞掉，工具行不显示。 */
  const [toolTrace, setToolTrace] = useState<LiveToolEntry[]>([])
  /** 最近一条实时事件的类型：决定状态行优先显示工具还是流式文本。
   *  没有它，先到的思考文本会把刚发生的工具调用挤出行外。 */
  const [lastLiveKind, setLastLiveKind] = useState<'tool' | 'thinking' | 'text' | null>(null)
  /** 流式内容区是否被用户收起：收起后正文/思考只在「过程行」这一行里体现，
   *  面板不再铺开大段流式文本（盯结果不想被刷屏时用）。每轮发送重置回展开。 */
  const [liveCollapsed, setLiveCollapsed] = useState(false)
  /** 上游未下发 call_id 时的合成序号（见 tool_call_start 的兜底） */
  const toolSeqRef = useRef(0)
  /** 流式文本有序分段：思考 / 正文按到达序（见 LiveSegment 注释） */
  const [streamSegments, setStreamSegments] = useState<LiveSegment[]>(EMPTY_LIVE_SEGMENTS)
  const liveSegmentsRef = useRef<LiveSegment[]>(EMPTY_LIVE_SEGMENTS)
  /** 按发生顺序的块序列（正文/思考/工具交错）。streamSegments 只管「当前在说什么」，
   *  这个序列才回答「这句话出现在哪一步工具之后」——两者并行维护，互不替代。 */
  const [liveBlocks, setLiveBlocks] = useState<LiveBlock[]>([])
  const liveBlocksRef = useRef<LiveBlock[]>([])

  // ── 输入区附件 / skill chips ──
  const [attachments, setAttachments] = useState<SideChatAttachment[]>([])
  const [skills, setSkills] = useState<ChatReference[]>([])
  /** 「+」小菜单（添加附件 / 选择技能）与技能列表浮层 */
  const [plusMenuOpen, setPlusMenuOpen] = useState(false)
  const [skillPickerOpen, setSkillPickerOpen] = useState(false)
  const [skillOptions, setSkillOptions] = useState<Array<{ name: string; label: string }>>([])
  const plusMenuRef = useRef<HTMLDivElement>(null)
  const skillPickerRef = useRef<HTMLDivElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  // 模型列表：**挂载即拉一次**（modelsLoaded 门保证只拉一次）。
  // 早年「弹窗打开才拉」的懒加载在推理强度入口上来后成了死锁——effort 旋钮的
  // 显示条件（模型是否声明 reasoning_efforts）也要这份数据，而用户不先开模型
  // 弹窗就永远看不到旋钮。listModels 是轻量本地命令，挂载拉一次的代价可接受。
  const [modelsLoaded, setModelsLoaded] = useState(false)
  useEffect(() => {
    if (modelsLoaded) return
    let cancelled = false
    void listModels()
      .then(list => {
        if (cancelled) return
        const filtered = (list ?? []).filter(m => !m.supports_image_generation)
        setModels(filtered)
        setModelsLoaded(true)
        // 持久化的模型可能已被删除 / 改名：列表非空且不含它时打回「跟随主会话」。
        // 否则触发器会显示一个不存在的 id，而后端会静默回落到主模型——显示与
        // 实际用的不是同一个，比「显示跟随」更难排查。
        setModel(cur =>
          cur && filtered.length > 0 && !filtered.some(m => m.id === cur) ? '' : cur,
        )
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [modelsLoaded])

  // 按提供商分组：未展开的提供商只出一行（logo + 名称 + 数量），不渲染模型子列表
  const providerGroups = useMemo(() => {
    const grouped = new Map<string, ModelInfo[]>()
    for (const m of models) {
      const bucket = grouped.get(m.provider)
      if (bucket) bucket.push(m)
      else grouped.set(m.provider, [m])
    }
    return [...grouped.entries()].map(([id, list]) => ({ id, models: list }))
  }, [models])

  const pickModel = useCallback((id: string, provider = '') => {
    setModel(id)
    // 提供商与模型成对：清空模型（跟随主会话）时 provider 必须一起清
    setModelProvider(provider)
    // 档位是模型能力：换模型（含切回跟随主会话）即回默认，避免残留不支持的等级
    setEffort(null)
    setModelPickerOpen(false)
    setExpandedProvider(null)
  }, [])

  // ── 推理强度能力契约：reasoning_efforts 非空才有旋钮（与主会话同口径）──
  // 有效模型 = 手动选择 || 跟随主会话；元数据来自懒加载的 models 列表。
  const effectiveModel = model || modelName || ''
  const supportedEfforts = useMemo(
    () => models.find(m => m.id === effectiveModel)?.reasoning_efforts ?? [],
    [models, effectiveModel],
  )
  const effortAvailable = supportedEfforts.length > 0

  /** ctx 分母：当前有效模型的上下文窗口。元数据缺失 → 0（界面显示 `--`，
   *  与主会话输入框同一条兜底：宁可留空也不拿 128K 假设冒充真实容量）。 */
  const ctxCap = useMemo(
    () => models.find(m => m.id === effectiveModel)?.context_window ?? 0,
    [models, effectiveModel],
  )

  // ── 拖拽移动面板：手柄按下记录起点，window 级 move/up 跟手 ──
  // 用 transform 位移而不是改 left/top：不触发重排，也不破坏原有定位基准。
  const [dragOffset, setDragOffset] = useState({ x: 0, y: 0 })
  const [dragging, setDragging] = useState(false)
  const dragOrigin = useRef({ x: 0, y: 0 })
  const onDragHandleDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      dragOrigin.current = { x: e.clientX - dragOffset.x, y: e.clientY - dragOffset.y }
      setDragging(true)
    },
    [dragOffset],
  )
  useEffect(() => {
    if (!dragging) return
    const onMove = (e: MouseEvent) => {
      setDragOffset({ x: e.clientX - dragOrigin.current.x, y: e.clientY - dragOrigin.current.y })
    }
    const onUp = () => setDragging(false)
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [dragging])

  // ── 面板尺寸：边缘拉伸 + 一键放大/还原 ──
  // 需求背景：最终结论里常见多列列表，380px 定宽会把表格挤成竖排换行，读不动。
  // 两条入口共用同一份带上下限的尺寸状态：
  //   ① 右下角把手连续拖（边缘拉伸）；
  //   ② 顶栏一键放大（按**应用窗口**尺寸自适应取比例），再点还原默认几何。
  // 上下限全部相对宿主容器（.chat-panel）实测，不写死像素——窗口变小 / 分屏时
  // 面板不会溢出屏幕，也永远不会被拉成一条线。
  const hostBox = useCallback(() => {
    const host = panelRef.current?.parentElement
    return {
      w: host?.clientWidth ?? window.innerWidth,
      h: host?.clientHeight ?? window.innerHeight,
    }
  }, [])
  const clampSize = useCallback(
    (w: number, h: number) => {
      const host = hostBox()
      return {
        w: Math.round(
          Math.min(Math.max(w, PANEL_SIZE_MIN.w), Math.max(PANEL_SIZE_MIN.w, host.w - 40)),
        ),
        h: Math.round(
          Math.min(Math.max(h, PANEL_SIZE_MIN.h), Math.max(PANEL_SIZE_MIN.h, host.h - 40)),
        ),
      }
    },
    [hostBox],
  )
  const applySize = useCallback(
    (w: number, h: number) => setPanelSize(clampSize(w, h)),
    [clampSize],
  )
  /** 当前生效尺寸：未拉过时按默认几何实测（放大以此为基准，不是从 0 起跳） */
  const currentSize = useCallback(() => {
    const el = panelRef.current
    const host = hostBox()
    return {
      w: el?.offsetWidth || 380,
      h: el?.offsetHeight || Math.max(PANEL_SIZE_MIN.h, host.h - 160),
    }
  }, [hostBox])
  const panelZoomed = panelSize !== null
  const togglePanelZoom = useCallback(() => {
    if (panelZoomed) {
      // 还原：清掉自定义尺寸，回到 CSS 的默认几何（width 380 + top/bottom 推导高）
      setPanelSize(null)
      return
    }
    const host = hostBox()
    // 放大取比例而非固定像素：应用窗口多大都自适应（分屏 / 小窗也不溢出）
    applySize(host.w * 0.62, host.h * 0.88)
  }, [panelZoomed, hostBox, applySize])

  // 边缘拉伸：右下角把手按下 → window 级 move/up 跟手（与拖拽移动同一套路）。
  // 起点记录「按下时尺寸 + 指针位置」，移动量即尺寸增量；下限由 clampSize 兜。
  const resizeOrigin = useRef({ x: 0, y: 0, w: 0, h: 0 })
  const onResizeHandleDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const cur = currentSize()
      resizeOrigin.current = { x: e.clientX, y: e.clientY, w: cur.w, h: cur.h }
      setResizing(true)
    },
    [currentSize],
  )
  useEffect(() => {
    if (!resizing) return
    const onMove = (e: MouseEvent) => {
      const o = resizeOrigin.current
      applySize(o.w + (e.clientX - o.x), o.h + (e.clientY - o.y))
    }
    const onUp = () => setResizing(false)
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [resizing, applySize])

  // ── 贴底跟随（useStickyScroll，与主对话 / 执行追踪面板同一实现）──
  // 旧实现是「内容一变就裸写滚动高度」：流式高频增长下每帧强行
  // 拉底，用户上滚 / 拖滚动条都被当场拽回——抢操作。统一走 hooks/useStickyScroll：
  // 只有用户手势能改写跟随态（上滚立即解锁、滚回底部区才恢复），内容增长永远不抢。
  // bottomZonePx / smoothPull 取与主对话窗口同口径（80 / true），观感一致。
  const liveTextLen = streamSegments.reduce((n, s) => n + s.text.length, 0)
  const stickKey = `${messages.length}:${liveTextLen}:${activeCalls.size}:${sending}`
  // 单行调用：源码契约测试以 `useStickyScroll(stickKey` 为锚点（见测试文件），
  // 折成多行会让那条断言失配。
  const { scrollRef, showJumpButton, onScroll, jumpToBottom, followReset } = useStickyScroll(
    stickKey,
    {
      bottomZonePx: 80,
      smoothPull: true,
    },
  )

  /** 轮次收口：后端在轮次结束（`ReturnGuard::drop`）时会清掉未消费的追加队列，
   *  UI 必须同步丢掉仍挂着 queued 的消息——否则「已入队」角标永驻，用户以为
   *  那句还在等注入，实际永远不会生效。已消费的那些已被 append_queue_updated
   *  翻成 normal，不在此列。
   *
   *  声明位置是硬约束：下方实时事件 effect 的 deps 与 handler 都要用它，
   *  必须早于那个 useEffect——const 在声明前处于 TDZ，deps 数组渲染期求值，
   *  晚一步就是 "Cannot access before initialization" 整面板崩。 */
  const dropPendingAppends = useCallback(() => {
    setMessages(prev => (prev.some(m => m.queued) ? prev.filter(m => !m.queued) : prev))
  }, [])

  // ── 实时执行过程：独立频道 'side-chat-event'，按 sessionKey 过滤 ──
  // 白名单：tool_call_start / tool_call_end / llm_text_delta / tool_output_line
  // （后者 Side Chat 无 system_shell，基本不触发，留着不渲染）；
  // 其余类型（execution_started / completed / hud_update / progress 等）忽略。
  // 监听随 sessionKey 变化重绑（面板复用但 key 变 = 另一场会话）。
  useEffect(() => {
    let cancelled = false
    const unlisteners: Array<() => void> = []
    void listen<unknown>('side-chat-event', payload => {
      // handler 收到 Event<T>，数据在 .payload（与 ChatInputBar 直连同款剥法）
      const p = payload as { payload?: { session_key?: string; event?: { type?: string } } }
      const data = p?.payload
      // 非本会话事件直接丢弃：多面板 / 主会话事件都不进本面板
      if (!data || data.session_key !== sessionKey) return
      const ev = data.event
      if (!ev || typeof ev.type !== 'string') return
      // 派发轮不经前端发送、前端从无 sending 态，实时过程区（渲染门槛
      // `sending && liveActive`，见下方过程区）对它整体不渲染——表现为
      // 「Leader 派发的任务执行时窗口看不见」。收到任何实时执行事件即置位
      // sending，让派发轮与普通轮一样执行中实时可见；收尾由 turn_finished
      // 复位。sendingRef 同步置位，兼作连击闸门。
      if (
        ev.type === 'tool_call_start' ||
        ev.type === 'tool_call_end' ||
        ev.type === 'llm_text_delta'
      ) {
        sendingRef.current = true
        setSending(true)
      }
      switch (ev.type) {
        case 'tool_call_start': {
          const e = ev as unknown as { call_id?: string; tool_name?: string; params?: unknown }
          const tool = e.tool_name ?? 'tool'
          const summary = liveSummary(e.params)
          // call_id 缺失（个别 provider 不下发 id）也要显示：合成一个键。
          // 旧写法在这里整条 return，等于「没有 call_id 的工具调用不显示」。
          const callId = e.call_id || `tool-${toolSeqRef.current++}`
          setActiveCalls(prev => {
            const next = new Map(prev)
            next.set(callId, { tool, summary })
            return next
          })
          // 轨迹只追加、永不移除：end 只改状态，状态行因此始终有内容可显示
          setToolTrace(prev => [...prev, { callId, tool, summary }])
          // 块序列：工具调用按发生顺序落到当前位置（前面的正文块自然被封口）
          const nextBlocks: LiveBlock[] = [
            ...liveBlocksRef.current,
            { kind: 'tool', tool, summary, params: e.params, callId },
          ]
          liveBlocksRef.current = nextBlocks
          setLiveBlocks(nextBlocks)
          setLastLiveKind('tool')
          return
        }
        case 'tool_call_end': {
          const e = ev as unknown as {
            call_id?: string
            success?: boolean
            duration_ms?: number
          }
          // 耗时直接取事件里的 duration_ms（后端 ToolResult 已统计，与主通道
          // useEvents.ts 同源）——不自己用 Date.now() 差值算：那会把事件排队 /
          // 合批的延迟算进工具耗时，数字不可信。
          const durationMs = typeof e.duration_ms === 'number' ? e.duration_ms : undefined
          setActiveCalls(prev => {
            if (prev.size === 0 || (e.call_id && !prev.has(e.call_id))) return prev
            const next = new Map(prev)
            if (e.call_id) {
              next.delete(e.call_id)
            } else {
              // 上游没下发 call_id：start 时用的是合成键，这里按 Map 保序删最后一个，
              // 否则那条调用永远挂在「在途」，状态行停在 in-flight、✓/✗ 永不出现。
              let lastKey = ''
              for (const k of next.keys()) lastKey = k
              if (lastKey) next.delete(lastKey)
            }
            return next
          })
          // 有 id 按 id 定位；没有 id（上游没下发）就标最后一条未完成的
          setToolTrace(prev => {
            const idx = e.call_id
              ? prev.findIndex(t => t.callId === e.call_id)
              : prev.reduce((acc, t, i) => (t.ok === undefined ? i : acc), -1)
            if (idx === -1) return prev
            const next = prev.slice()
            next[idx] = { ...next[idx], ok: e.success !== false, durationMs }
            return next
          })
          // 块序列回填成败：无 call_id 时标最后一个未完结的工具块（与上面同口径）
          {
            const done = e.success !== false
            const blocks = liveBlocksRef.current
            const target = e.call_id
              ? blocks.findIndex(b => b.kind === 'tool' && b.callId === e.call_id)
              : blocks.reduce(
                  (acc, b, idx) => (b.kind === 'tool' && b.ok === undefined ? idx : acc),
                  -1,
                )
            if (target !== -1) {
              const nextBlocks = blocks.slice()
              const b = nextBlocks[target]
              if (b.kind === 'tool') {
                nextBlocks[target] = { ...b, ok: done, durationMs }
              }
              liveBlocksRef.current = nextBlocks
              setLiveBlocks(nextBlocks)
            }
          }
          setLastLiveKind('tool')
          return
        }
        case 'llm_text_delta': {
          const e = ev as unknown as { text?: string; is_thinking?: boolean }
          const text = e.text ?? ''
          if (!text) return
          const kind: LiveSegment['kind'] = e.is_thinking === true ? 'thinking' : 'text'
          // 累积以 ref 为单一事实源，state 只做渲染镜像。副作用因此必须写在
          // updater 外面——updater 在 StrictMode 下会被调用两次，写进去等于
          // 同一段文本被追加两遍（同类约定见 BackgroundTasksPanel.tsx 的注释）。
          // ref 先行的另一个好处：发送结束时读它拿到的必然是全量，不受 React
          // 尚未提交最后一帧的影响。
          const prev = liveSegmentsRef.current
          const last = prev[prev.length - 1]
          let next: LiveSegment[]
          // 同类型连续 delta 并入末段；思考↔正文切换即开新段——
          // 多轮迭代的思考与正文由此保持真实到达顺序，不再各攒一坨。
          //
          // 合并必须幂等。侧频道没有 seq / 去重（side_chat.rs 的 SideChatEmitter
          // 只做裸 emit），重复投递无法在传输层排除：同一 text 二次到达（幽灵
          // 监听器 / 上游重发）与 text 是末段的尾巴（重叠重发）都判定为「无新
          // 内容」。曾经的实现把 text === last.text 当成增量而整段再拼一遍，
          // 且末段被污染后 startsWith 再也不命中，误差沿整条流复利放大——
          // 表现就是思考文本出现交错重复。
          if (!last || last.kind !== kind) {
            next = [...prev, { kind, text }]
          } else if (text === last.text || last.text.endsWith(text)) {
            next = prev
          } else {
            // 累积式快照（provider 整段重发到当前位置）→ 替换；否则按增量拼接
            const mergedText = text.startsWith(last.text) ? text : last.text + text
            next = [...prev.slice(0, -1), { kind, text: mergedText }]
          }
          if (next !== prev) {
            liveSegmentsRef.current = next
            setStreamSegments(next)
          }
          // 块序列同步（换成同一份合并逻辑）；工具位置由它保留
          const nextBlocks = appendLiveText(liveBlocksRef.current, kind, text)
          if (nextBlocks !== liveBlocksRef.current) {
            liveBlocksRef.current = nextBlocks
            setLiveBlocks(nextBlocks)
          }
          setLastLiveKind(kind)
          return
        }
        // Leader 派发轮：brief 显式落用户气泡（带来源标识）。两种时机：
        // ① 空闲派发起点（带 turn_id）——执行前的任务气泡；
        // ② 派发轮在途追加（queued=true，无 turn_id，唯一豁免）——「已入队」
        //    气泡，渲染在输入框上方固定区，迭代边界消费后由
        //    append_queue_updated 转正为列表正式消息（from 保留）。
        // 用户手动追加走前端乐观路径（handleSend），与此事件无关。
        case 'side_chat_dispatch_question': {
          const e = ev as unknown as { question?: string; turn_id?: string; queued?: boolean }
          // 派发起点 = 后端已在途的权威信号（`dispatch_fresh` 在 await
          // send_impl **之前**就 emit 它）。前端对这一路从无 sending 态，若不在此
          // 置位，从派发到首个流式事件之间（LLM 首 token 可能几十秒）composer
          // 一直停在「发送」态：用户一发就被后端 BUSY 拒收，追加钮与停止钮都不
          // 在——正是「追加路径不见了 / 停不下来」的体感来源。
          // queued=true 是「派发轮在途追加」：轮次早已在跑，无需再置位。
          if (e.queued !== true) {
            sendingRef.current = true
            setSending(true)
          }
          if (e.question) {
            const qTurn = e.turn_id ?? ''
            setMessages(prev =>
              qTurn && prev.some(m => m.role === 'user' && m.turnId === qTurn)
                ? prev
                : [
                    ...prev,
                    {
                      role: 'user',
                      text: e.question as string,
                      turnId: qTurn || undefined,
                      from: 'leader_dispatch',
                      queued: e.queued === true,
                    },
                  ],
            )
          }
          return
        }
        // 追加指令被迭代边界消费：对应气泡从「已入队」转为正式消息。
        // 后端在 drain 之后 emit（sub_task_loop 的 append 消费点），所以这是
        // 「真的进上下文了」的回执，不是乐观 UI。
        case 'append_queue_updated': {
          setMessages(prev =>
            prev.some(m => m.queued)
              ? prev.map(m => (m.queued ? { ...m, queued: false } : m))
              : prev,
          )
          return
        }
        // 轮次完成（后端在 invoke 返回之外补发）：刷新过的页面只能靠它拿到回复。
        // 正常路径下同一个 turn_id 已由 invoke 返回落过气泡 → 这里判重后跳过。
        case 'side_chat_turn_finished': {
          const e = ev as unknown as {
            turn_id?: string
            reply?: string
            /** 拼装前的指令原文（后端 raw_message）：Leader 派发轮次的 brief 由后端
             *  直注伙伴、不经前端，本地没有对应用户气泡——靠它补落 */
            question?: string
            steps?: { tool: string; ok: boolean; summary: string }[]
            ctx_used?: number
          }
          const finishedTurn = e.turn_id ?? ''
          if (!finishedTurn) return
          // 回看通道的数据装配：后端 turn_finished 的 steps 恒为空数组——
          // run_free 五个返回点全部硬编码 vec![]（src/runtime/sub_task_loop.rs
          // 的 :98/:208/:284/:334/:379），completed 消息的过程条曾因此永不渲染。
          // 这里用与流式过程区**同源**的 liveBlocks/liveSegments 现攒现补：
          // 事件先行到达时由本路径建消息；命令返回竞态先到时补写既有气泡，
          // 两条路径合一，hasProcess 不再依赖后端 steps。
          const traceSteps = liveBlocksRef.current
            .filter(b => b.kind === 'tool')
            .map(b => ({ tool: b.tool, ok: b.ok !== false, summary: b.summary }))
          const thinking = liveSegmentsRef.current
            .filter(s => s.kind === 'thinking')
            .map(s => s.text)
            .join('\n\n')
            .trim()
          const blocks = buildMsgBlocks(e.reply ?? '', liveBlocksRef.current)
          setMessages(prev => {
            const idx = prev.findIndex(m => m.role === 'assistant' && m.turnId === finishedTurn)
            if (idx === -1) {
              // 只落回复：user 消息只来自「派发起点 side_chat_dispatch_question」与
              // 「用户手动发送」；turn_finished 不再补 user（后端也不再带 question）。
              return [
                ...prev,
                {
                  role: 'assistant',
                  text: e.reply ?? '',
                  // 后端 steps 日后修好后优先用它；当前恒空，用实时轨迹兜底
                  steps: Array.isArray(e.steps) && e.steps.length > 0 ? e.steps : traceSteps,
                  thinking: thinking || undefined,
                  blocks,
                  ctxUsed: e.ctx_used ?? 0,
                  turnId: finishedTurn,
                },
              ]
            }
            // 竞态落败方（命令返回先到、气泡已由 handleSend 落过）：已有气泡但
            // 缺过程数据时补写 steps/thinking/blocks，不重复建条、不丢既有内容。
            const existed = prev[idx]
            const hasProcess =
              (existed.steps?.length ?? 0) > 0 ||
              (existed.blocks?.length ?? 0) > 0 ||
              Boolean(existed.thinking)
            if (hasProcess) return prev
            const next = prev.slice()
            next[idx] = {
              ...existed,
              steps: Array.isArray(e.steps) && e.steps.length > 0 ? e.steps : traceSteps,
              thinking: thinking || existed.thinking,
              blocks,
            }
            return next
          })
          // 该轮已收尾：执行中态结束（正常路径的 finally 也设 false，幂等）
          sendingRef.current = false
          setSending(false)
          // 同上：轮次结束 = 后端已清未消费追加队列，UI 同步丢
          dropPendingAppends()
          return
        }
        // tool_output_line：白名单保留但不渲染（无 system_shell，基本不触发）
        default:
          return
      }
    }).then(u => {
      // 已卸载（StrictMode 双挂载 / sessionKey 快速切换时 cleanup 先于
      // listen() 的 Promise resolve）→ 立刻退订。旧写法在这里直接 return，
      // 监听器已经注册却永不退订，成为幽灵监听器：此后同一条事件会被这个
      // handler 再处理一次，是 dev 下重复投递的根源。正确范式见 useEvents.ts
      // 的 disposed 分支（已卸载即 fn()）。
      if (cancelled) u()
      else unlisteners.push(u)
    })
    return () => {
      cancelled = true
      unlisteners.forEach(u => u())
    }
  }, [sessionKey, dropPendingAppends])

  /** 下发给后端的工作目录：Explore 哨兵原样透传（后端据此生成「无项目」段），
   *  空串与具体路径按原样（空串 = 跟随主会话，后端不注入头部）。 */
  const projectDirArg = projectPath === EXPLORE_PROJECT ? EXPLORE_PROJECT : projectPath || null

  // 模型弹窗 / + 菜单 / 技能浮层 / 推理强度 / 项目选择器：点外面收起（同一套路）
  useEffect(() => {
    if (
      !modelPickerOpen &&
      !plusMenuOpen &&
      !skillPickerOpen &&
      !effortOpen &&
      !projectPickerOpen
    ) {
      return
    }
    const onDown = (e: MouseEvent) => {
      if (modelPickerRef.current && !modelPickerRef.current.contains(e.target as Node)) {
        setModelPickerOpen(false)
        setExpandedProvider(null)
      }
      if (plusMenuRef.current && !plusMenuRef.current.contains(e.target as Node)) {
        setPlusMenuOpen(false)
      }
      if (skillPickerRef.current && !skillPickerRef.current.contains(e.target as Node)) {
        setSkillPickerOpen(false)
      }
      if (effortRef.current && !effortRef.current.contains(e.target as Node)) {
        setEffortOpen(false)
      }
      if (projectRef.current && !projectRef.current.contains(e.target as Node)) {
        setProjectPickerOpen(false)
      }
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [modelPickerOpen, plusMenuOpen, skillPickerOpen, effortOpen, projectPickerOpen])

  /** 执行中追加：入队到**当前轮**，由迭代边界注入上下文，不新建轮次。
   *
   *  与 handleSend 的分工：sending 时 send 会被后端 BUSY 拒绝，追加是唯一的
   *  合法入口。三条硬规则——
   *  1. 成功入队才清空输入框（失败原样保留，用户不用重打）；
   *  2. 后端判重返回 duplicate 时明确提示，不假装成功；
   *  3. 追加走的是纯文本通道，不经过 references 解析，选中技能时先告警再发，
   *     不静默丢掉用户的选择。 */
  const handleAppend = useCallback(async () => {
    const text = input.trim()
    if (!text) return
    if (skills.length > 0) {
      hudUpdate(t('sideChat.appendSkillIgnored'), 'warning')
    }
    const attachmentLines = attachments.map(a => `[附件: ${a.path}]`)
    const finalText = attachmentLines.length > 0 ? `${text}\n${attachmentLines.join('\n')}` : text
    try {
      const res = await sideChatAppend(sessionKey, finalText, projectDirArg)
      if (res?.duplicate) {
        hudUpdate(t('sideChat.appendDuplicate'), 'warning')
        return
      }
      if (!res?.queued) {
        hudUpdate(t('sideChat.appendFailed'), 'warning')
        return
      }
      setMessages(prev => [...prev, { role: 'user', text: finalText, queued: true }])
      // 同上：追加也是新内容落袋，同样强制回底
      followReset()
      setInput('')
      setAttachments([])
      setSkills([])
    } catch (err) {
      // 空闲态拒绝 / 会话已销毁：原样保留输入，用户可直接改走发送
      hudUpdate(err instanceof Error ? err.message : String(err), 'warning')
    }
  }, [attachments, input, sessionKey, skills, t, projectDirArg, followReset])

  /** 执行中追加、尚未被迭代边界消费的消息。
   *
   *  为什么不放在消息列表里：列表跟着流式内容一起长，追加气泡会被顶出视口
   *  （用户原话「流式输出时界面里根本看不到」）。改成渲染在输入框上方的固定
   *  区域后，它与滚动解耦，执行期间始终可见；被 drain 后翻成 normal，自然
   *  过渡成消息列表里的正式气泡。 */
  const queuedAppends = useMemo(() => messages.filter(m => m.queued), [messages])

  /** 取消一条执行中追加：后端按内容从隔离队列移除（见 api 头注）。
   *  removed=false = 消费者抢先一步（已被 drain），按「已生效」收尾并明示，
   *  不做无意义重试——重试只会再次错过。 */
  const cancelAppend = useCallback(
    async (text: string) => {
      try {
        const res = await sideChatRemoveAppendItem(sessionKey, text)
        if (res?.removed) {
          setMessages(prev => prev.filter(m => !(m.queued && m.text === text)))
          return
        }
        hudUpdate(t('sideChat.appendCancelTooLate'), 'warning')
        setMessages(prev =>
          prev.map(m => (m.queued && m.text === text ? { ...m, queued: false } : m)),
        )
      } catch (err) {
        hudUpdate(err instanceof Error ? err.message : String(err), 'warning')
      }
    },
    [sessionKey, t],
  )

  /** 刷一次交付箱积压计数（读 sideChatState，不做实时订阅）。
   *  调用点：挂载（刷新恢复 effect）与每轮发送结束——后者正是「本轮结论被
   *  Finalizing 拒收落箱」的时刻，不刷的话用户要等到刷新才看见计数。 */
  const refreshPendingDeliveries = useCallback(() => {
    void sideChatState(sessionKey)
      .then(st => {
        if (!st) return
        setPendingDeliveries(typeof st.pending_deliveries === 'number' ? st.pending_deliveries : 0)
      })
      .catch(() => {
        /* 查询失败：保留上一次计数，不打扰用户 */
      })
  }, [sessionKey])

  const handleSend = useCallback(async () => {
    const text = input.trim()
    if (!text) return
    // 执行中：同一次回车不再被丢掉，转为「追加到本轮」。
    // 后端并发兜底仍在（BUSY_MESSAGE），这里只是不让用户的输入白敲。
    // 闸门用 sendingRef（同步）而非 sending（异步 state）：快速连击时 state
    // 尚未提交，sending 闭包值仍是 false，会漏过这层把用户消息 push 两条。
    if (sending || sendingRef.current) {
      await handleAppend()
      return
    }
    // 一旦决定发送，立即置同步闸门——必须早于下方乐观更新，堵住批处理窗口。
    sendingRef.current = true
    // 附件按行拼到消息末尾（与主会话 ChatPanel.handleSubmit 同构）：
    // 后端 system_prompt 已说明 `[附件: 路径]` 是本地绝对路径，用 Read 打开
    const attachmentLines = attachments.map(a => `[附件: ${a.path}]`)
    const finalText = attachmentLines.length > 0 ? `${text}\n${attachmentLines.join('\n')}` : text
    // skill 引用走后端 references 解析（注入 SKILL.md 全文）；无选中时不传该参
    const refs = skills.length > 0 ? skills : undefined
    // 失败要原样退回 chips：先快照再清空（输入框立即清空的手感不变）
    const sentAttachments = attachments
    const sentSkills = skills
    // 轮次 id：刷新后「正常返回 / 完成事件 / state 回填」三路都靠它判重。
    // crypto.randomUUID 在极老 webview / jsdom 下可能缺失，回落时间戳+随机数。
    const turnId =
      typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
        ? crypto.randomUUID()
        : `turn-${Date.now()}-${Math.random().toString(36).slice(2)}`
    setMessages(prev => [...prev, { role: 'user', text: finalText }])
    // 发送即新轮次：强制回底并恢复跟随。用户此前可能上滚翻历史（那时跟随态
    // 已被手势解锁），不显式复位的话新气泡落在视口外，得自己滚下去才看见。
    // 走 followReset 而非 jumpToBottom：这是「新轮次」的确定性落点，要瞬移
    // 不要动画（smoothPull 的平滑追踪是给回底按钮用的）。
    followReset()
    setInput('')
    setAttachments([])
    setSkills([])
    setSending(true)
    setActiveCalls(new Map())
    setToolTrace([])
    setLastLiveKind(null)
    setLiveCollapsed(false)
    setStreamSegments(EMPTY_LIVE_SEGMENTS)
    liveSegmentsRef.current = EMPTY_LIVE_SEGMENTS
    setLiveBlocks([])
    liveBlocksRef.current = []
    // 降级标记：后端 BUSY（在途）时本条消息改走追加，finally 的收口必须整段跳过
    let degradedToAppend = false
    try {
      // references / effort / provider 缺省均不传：后端按 null 兼容。
      // provider 必须与 model 成对传——只给 id 时后端按段序首段猜 provider，
      // 同名模型会连到别的 base_url（本地代理 8317 事故）。
      const activeProvider = model ? modelProvider || undefined : undefined
      const res = refs
        ? await sideChatSend(
            sessionKey,
            finalText,
            fileAccess,
            webSearch,
            model || modelName,
            refs,
            effort,
            activeProvider,
            projectDirArg,
            turnId,
          )
        : await sideChatSend(
            sessionKey,
            finalText,
            fileAccess,
            webSearch,
            model || modelName,
            undefined,
            effort,
            activeProvider,
            projectDirArg,
            turnId,
          )
      // bridge 在无 Tauri 环境下返回 null（mock 兜底）——按空回复处理
      const reply = res?.reply ?? ''
      const steps = Array.isArray(res?.steps) ? res.steps : []
      const thinkingSegments = liveSegmentsRef.current.filter(s => s.kind === 'thinking')
      const thinking = thinkingSegments
        .map(s => s.text)
        .join('\n\n')
        .trim()
      setMessages(prev => {
        // 完成事件可能先于 invoke 返回到达（尤其刷新恢复路径）：同一 turnId
        // 已有气泡就不再追加，否则同一条回复会落成两个。
        if (prev.some(m => m.role === 'assistant' && m.turnId === turnId)) return prev
        return [
          ...prev,
          {
            role: 'assistant',
            text: reply,
            steps,
            thinking: thinking || undefined,
            // 块序列（正文 / 工具按发生顺序）：有它就交错渲染，没有则退回摘要条
            blocks: buildMsgBlocks(reply, liveBlocksRef.current),
            // ctx 快照跟着消息走：后续换模型 / 刷新都能保留「这一轮到底吃了多少」
            ctxUsed: res?.ctx_used ?? 0,
            turnId,
          },
        ]
      })
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err)
      // 后端是「在途」的唯一权威（slot.running），前端 sending 只是它的投影。
      // 投影落后于事实时（派发起点事件未到 / 监听漏事件 / 刷新恢复竞态），这条
      // 消息**不是失败**：降级为追加——队列插入检查在后端，enqueue 即刻生效于
      // 下一个迭代边界；同时把 composer 切到「追加 + 停止」态，用户能停、能继续加。
      // 不这么做，用户看到的只有一个错误气泡：消息没发出，也没入队。
      if (detail === BACKEND_BUSY_MESSAGE) {
        degradedToAppend = true
        // 先撤掉刚才那条乐观气泡：本条消息改走队列，不能同时在列表里留一份
        // （否则同一条消息出现两次——一次像已发出，一次在「已入队」区）。
        // 只撤「最后一条、同文本、非 queued」的那条，绝不碰历史消息。
        setMessages(prev => {
          const last = prev[prev.length - 1]
          return last && last.role === 'user' && last.text === finalText && !last.queued
            ? prev.slice(0, -1)
            : prev
        })
        sendingRef.current = true
        setSending(true)
        await handleAppend()
        return
      }
      // 失败保留 chips 与输入：用户改一眼就能重发，不用重新选文件 / skill
      setInput(text)
      setAttachments(sentAttachments)
      setSkills(sentSkills)
      const kind = explainSideChatError(detail)
      setMessages(prev => [
        ...prev,
        {
          role: 'assistant',
          text: t(`sideChat.error.${kind}.title`),
          error: true,
          errorHint: t(`sideChat.error.${kind}.hint`),
          errorDetail: detail,
        },
      ])
    } finally {
      // 降级为追加时轮次仍在途：sending / 实时过程 / 待入队区必须原样保持，
      // 不能被 finally 的收口清掉（否则追加气泡与停止钮一起消失）。
      if (degradedToAppend) return
      sendingRef.current = false
      setSending(false)
      // 轮次收口：后端已清掉未消费的追加队列，UI 同步丢掉（角标不永驻）
      dropPendingAppends()
      // 交付箱计数刷新：本轮结论可能被 Finalizing 拒收落箱，此刻才知道条数
      refreshPendingDeliveries()
      // 实时态收口：迟到的 side-chat-event 不再渲染（监听仍在，但 sending 已 false）
      setActiveCalls(new Map())
      setToolTrace([])
      setLastLiveKind(null)
      setStreamSegments(EMPTY_LIVE_SEGMENTS)
      liveSegmentsRef.current = EMPTY_LIVE_SEGMENTS
      // 块序列已在完成时写进消息（blocks），实时副本到此清空
      setLiveBlocks([])
      liveBlocksRef.current = []
    }
  }, [
    input,
    sending,
    sessionKey,
    fileAccess,
    webSearch,
    model,
    modelProvider,
    modelName,
    attachments,
    skills,
    effort,
    t,
    handleAppend,
    projectDirArg,
    dropPendingAppends,
    refreshPendingDeliveries,
  ])

  /** 停止在途执行：只置位后端中断标志，不销毁会话、不清消息。
   *  执行体在迭代边界封口，回复气泡呈现「任务已被用户中断」（非错误态）；
   *  下一轮发送由后端复位标志，复用执行体继续。 */
  const handleStop = useCallback(() => {
    void sideChatCancel(sessionKey).catch(err => {
      hudUpdate(err instanceof Error ? err.message : String(err), 'warning')
    })
  }, [sessionKey])

  // 在途执行态上报：父组件的左下角入口角标据此切换（执行中脉冲 / 空闲灰点）
  useEffect(() => {
    onSendingChange?.(sending)
  }, [sending, onSendingChange])

  /** 清空对话：本地消息立即清空，后端丢弃执行体（下次发送重建） */
  const handleClear = useCallback(async () => {
    setMessages([])
    try {
      await sideChatReset(sessionKey)
    } catch (err) {
      console.warn('[SideChat] reset failed:', err)
    }
  }, [sessionKey])

  // ── 附件：隐藏 file input 多选 → chip（name 显示 / path 注入）──
  const onFilesPicked = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(e.target.files ?? [])
      // 复位 input：同一个文件二次选择也要再触发 change
      e.target.value = ''
      if (files.length === 0) return
      const added: SideChatAttachment[] = []
      let missingPath = false
      for (const f of files) {
        // Tauri webview 暴露 File.path；浏览器 / 拿不到时退回文件名并明说
        const raw = (f as File & { path?: string }).path
        const p = typeof raw === 'string' ? raw.trim() : ''
        if (!p) missingPath = true
        added.push({ name: f.name, path: p || f.name })
      }
      setAttachments(prev => [...prev, ...added])
      hudUpdate(t('sideChat.attachChosen'), 'info')
      // 不许静默丢失：路径不可用就告诉用户「只发了文件名，模型可能读不到内容」
      if (missingPath) hudUpdate(t('sideChat.attachNoPath'), 'warning')
      // 文件读写关闭：不禁用入口、不阻断发送，但必须说清「模型读不到，只看得到名」。
      // 与上面是并列的两个 if（不是 else-if）：两种原因同时命中时两条警告都要出。
      if (!fileAccess) hudUpdate(t('sideChat.attachNoRead'), 'warning')
    },
    [t, fileAccess],
  )

  const removeAttachment = useCallback((index: number) => {
    setAttachments(prev => prev.filter((_, i) => i !== index))
  }, [])

  // ── skill：skill_list（与主会话资源选择同款 invoke）→ chip → 发送走 references ──
  const openSkillPicker = useCallback(async () => {
    setPlusMenuOpen(false)
    try {
      const { invoke } = await import('@tauri-apps/api/core')
      const list =
        await invoke<Array<{ name: string; display_name: string; description: string }>>(
          'skill_list',
        )
      setSkillOptions((list ?? []).map(s => ({ name: s.name, label: s.display_name || s.name })))
      setSkillPickerOpen(true)
    } catch (err) {
      console.warn('[SideChat] skill_list failed:', err)
      hudUpdate(t('sideChat.skillListFailed'), 'error')
    }
  }, [t])

  const pickSkill = useCallback(
    (name: string, label: string) => {
      setSkillPickerOpen(false)
      setSkills(prev => {
        if (prev.some(s => s.id === name)) return prev
        return [...prev, { type: 'skill', id: name, label }]
      })
      hudUpdate(t('sideChat.skillChosen'), 'info')
    },
    [t],
  )

  const removeSkill = useCallback((index: number) => {
    setSkills(prev => prev.filter((_, i) => i !== index))
  }, [])

  // ── 保存 / 分享：同一份 md（buildSessionMarkdown 纯函数）──
  const sessionMarkdown = useCallback(
    () => buildSessionMarkdown(messages, model || modelName),
    [messages, model, modelName],
  )

  const handleSave = useCallback(async () => {
    if (messages.length === 0) {
      hudUpdate(t('sideChat.emptySession'), 'warning')
      return
    }
    try {
      // bridge 在无 Tauri 环境返回 null：按保存失败处理，不显示空路径
      const path = await sideChatSave(sessionKey, sessionMarkdown())
      if (!path) {
        hudUpdate(t('sideChat.saveFailed'), 'error')
        return
      }
      hudUpdate(t('sideChat.savedToast').replace('{path}', path), 'done')
    } catch (err) {
      console.warn('[SideChat] save failed:', err)
      hudUpdate(t('sideChat.saveFailed'), 'error')
    }
  }, [messages.length, sessionKey, sessionMarkdown, t])

  // ── 历史（保存 / 清空 / 最近执行归档）：顶栏单图标 + hover 展开 ──
  const [historyOpen, setHistoryOpen] = useState(false)
  const [historyItems, setHistoryItems] = useState<SideChatHistoryEntry[]>([])
  const historyRef = useRef<HTMLDivElement>(null)
  const historyMenuRef = useRef<HTMLDivElement>(null)
  const historyListRef = useRef<HTMLUListElement>(null)
  /** 拉最近 10 条归档（新的在前）；失败静默成空列表，不打断面板 */
  const loadHistory = useCallback(async () => {
    try {
      setHistoryItems((await sideChatHistoryList(10)) ?? [])
    } catch (err) {
      console.warn('[SideChat] history list failed:', err)
      setHistoryItems([])
    }
  }, [])
  const openHistory = useCallback(() => {
    setHistoryOpen(true)
    void loadHistory()
  }, [loadHistory])

  /**
   * hover 进出：图标与浮层之间有 6px 视觉间隙，鼠标穿过时会先离开触发区——
   * 直接 mouseleave 就关会造成「躲猫猫」。两手一起治：
   *   ① 关闭延迟 HISTORY_CLOSE_DELAY（穿过间隙的时间远小于它）；
   *   ② 浮层用 ::before 铺一条透明桥接条，把间隙并入可命中区域（CSS）。
   */
  const historyCloseTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const handleHistoryEnter = useCallback(() => {
    if (historyCloseTimer.current) {
      clearTimeout(historyCloseTimer.current)
      historyCloseTimer.current = null
    }
    openHistory()
  }, [openHistory])
  const handleHistoryLeave = useCallback(() => {
    if (historyCloseTimer.current) clearTimeout(historyCloseTimer.current)
    historyCloseTimer.current = setTimeout(() => {
      historyCloseTimer.current = null
      setHistoryOpen(false)
    }, 160)
  }, [])

  /**
   * 浮层纵向高度上限 = 面板底边 − 浮层顶边 − 12px 内边，再减去列表上方的
   * 固定内容（两个动作 + 分隔 + 标题 + 浮层内边距，实测取 list.offsetTop）。
   *
   * 为什么必须 JS：absolute 的 max-height 百分比相对包含块（.scp-header，
   * 45px 高），表达不了「不超过 .scp-panel 底边」；而裁切恰恰来自 panel 的
   * overflow:hidden——探出底边的部分会被整段裁掉。横向由 CSS（right:12px +
   * max-width）兜底，纵向没有等价的纯 CSS 表达，只能实测。window resize
   * 时重算。jsdom 里 rect 全 0：算出的上限不达标即不留内联值，测试不受影响。
   */
  const fitHistoryMenu = useCallback(() => {
    const menu = historyMenuRef.current
    const list = historyListRef.current
    const panel = panelRef.current
    if (!menu || !list || !panel) return
    const avail = panel.getBoundingClientRect().bottom - menu.getBoundingClientRect().top - 12
    const max = avail - list.offsetTop - 4
    list.style.maxHeight = max >= 80 ? `${Math.floor(max)}px` : ''
  }, [])
  useEffect(() => {
    if (!historyOpen) return
    fitHistoryMenu()
    window.addEventListener('resize', fitHistoryMenu)
    return () => window.removeEventListener('resize', fitHistoryMenu)
  }, [historyOpen, fitHistoryMenu])
  // 卸载时清掉定时器，避免对已卸载组件 setState
  useEffect(
    () => () => {
      if (historyCloseTimer.current) clearTimeout(historyCloseTimer.current)
    },
    [],
  )
  const removeHistory = useCallback(async (id: string) => {
    try {
      await sideChatHistoryDelete(id)
      setHistoryItems(prev => prev.filter(item => item.id !== id))
    } catch (err) {
      hudUpdate(err instanceof Error ? err.message : String(err), 'warning')
    }
  }, [])

  const handleShare = useCallback(async () => {
    if (messages.length === 0) {
      hudUpdate(t('sideChat.emptySession'), 'warning')
      return
    }
    const md = sessionMarkdown()
    try {
      await navigator.clipboard.writeText(md)
      hudUpdate(t('sideChat.copiedToast'), 'done')
    } catch {
      // fallback：隐藏 textarea + execCommand（与 ChatPanel.handleCopy 同款）
      try {
        const ta = document.createElement('textarea')
        ta.value = md
        document.body.appendChild(ta)
        ta.select()
        document.execCommand('copy')
        document.body.removeChild(ta)
        hudUpdate(t('sideChat.copiedToast'), 'done')
      } catch (err) {
        console.warn('[SideChat] share failed:', err)
        hudUpdate(t('sideChat.shareFailed'), 'error')
      }
    }
  }, [messages.length, sessionMarkdown, t])

  // ── 实时过程派生：最新在途工具（Map 保序，末个即最新开始）+ 是否有实时数据 ──
  const liveActive = activeCalls.size > 0 || streamSegments.length > 0 || toolTrace.length > 0
  // 最后一条正常助手回复的下标：底部单行操作钮只挂它（-1 = 暂无）。
  // 过程记录进各自消息的折叠摘要条，动作成行不悬浮（Copilot 式简洁）。
  const lastAssistantIdx = useMemo(
    () => messages.reduce((acc, m, i) => (m.role === 'assistant' && !m.error ? i : acc), -1),
    [messages],
  )
  const latestCall = useMemo(() => {
    let latest: LiveCall | null = null
    for (const call of activeCalls.values()) latest = call
    return latest
  }, [activeCalls])
  // 同名工具并行计数：`Read ×2`（只对最新工具计数，不混算其他工具）
  const latestCallCount = useMemo(() => {
    if (!latestCall) return 0
    let n = 0
    for (const call of activeCalls.values()) {
      if (call.tool === latestCall.tool) n += 1
    }
    return n
  }, [activeCalls, latestCall])

  /** 并行同名工具优先：有「×N」时状态行要让用户看见并发量，而不是恰好最后
   *  开始的那一个（三个在途 Grep/Grep/Read 应显示 `Grep ×2`）。取在途计数最大
   *  的工具；并列时取较晚开始者（Map 保序）。 */
  const headCall = useMemo(() => {
    if (activeCalls.size === 0) return null
    const counts = new Map<string, number>()
    const lastOf = new Map<string, LiveCall>()
    for (const call of activeCalls.values()) {
      counts.set(call.tool, (counts.get(call.tool) ?? 0) + 1)
      lastOf.set(call.tool, call)
    }
    let best: { call: LiveCall; n: number } | null = null
    for (const [tool, n] of counts) {
      if (!best || n >= best.n) best = { call: lastOf.get(tool)!, n }
    }
    return best
  }, [activeCalls])

  /** 状态行显示哪个工具：在途的优先；在途窗口被合批吞掉时退到本轮最后一个
   *  （带 ✓/✗），同样回答「刚刚用了什么工具」。仅在最近事件是工具时才显示——
   *  否则先到的思考/正文会把工具行一直占着。 */
  const shownTool: {
    tool: string
    summary: string
    ok?: boolean
    durationMs?: number
  } | null = latestCall
    ? { tool: latestCall.tool, summary: latestCall.summary }
    : lastLiveKind === 'tool' && toolTrace.length > 0
      ? toolTrace[toolTrace.length - 1]
      : null

  /** 流式最新分段类型：执行中单行的图标与状态文本都按「正文 > 思考 > 工具」优先取它 */
  const lastSegKind = streamSegments[streamSegments.length - 1]?.kind

  const hasMoved = dragOffset.x !== 0 || dragOffset.y !== 0

  /** 当前生效的项目：空 projectPath = 跟随主会话，标签用主会话项目名（可能未设）；
   *  EXPLORE_PROJECT 不属于任何项目路径——activeProject 恒为 null，标签走 Explore。 */
  const isExplore = projectPath === EXPLORE_PROJECT
  const activeProject =
    projectPath && !isExplore
      ? (projects.find(p => p.path === projectPath) ?? {
          name: projectPath.split(/[\\/]/).filter(Boolean).pop() || projectPath,
          path: projectPath,
        })
      : null
  const projectLabel = isExplore
    ? t('sideChat.projectExplore')
    : activeProject
      ? activeProject.name
      : mainProject.name || t('sideChat.projectFollow')

  return (
    <aside
      ref={panelRef}
      className={`scp-panel${minimized ? ' is-minimized' : ''}${dragging ? ' is-dragging' : ''}${hasMoved ? ' is-moved' : ''}${resizing ? ' is-resizing' : ''}`}
      aria-label={t('sideChat.title')}
      style={{
        ...(hasMoved ? { transform: `translate(${dragOffset.x}px, ${dragOffset.y}px)` } : {}),
        // 自定义尺寸：宽高一起给，并顶掉 CSS 的 bottom 推导高度——否则
        // bottom:140px 会与 height 同时生效，实际高度变成两者之差。
        ...(panelSize
          ? { width: panelSize.w, height: panelSize.h, maxWidth: 'none', bottom: 'auto' }
          : {}),
      }}
    >
      {/* ── 顶栏：左组（拖拽手柄 / 项目）· 右组（历史 / 分享 / 放大还原 / 收起 / 关闭）──
          临时伙伴设置（齿轮）已整块移除：权限固定开启，由后端 ToolPermissions
          决定实际可用面，面板不再提供任何开关。 */}
      <header className="scp-header">
        <div className="scp-header-group">
          {/* 拖拽手柄：按住移动面板；拖动中禁用文本选择 */}
          <IconTip text={t('sideChat.drag')} align="left">
            <span
              className="scp-drag-handle"
              role="button"
              tabIndex={-1}
              aria-label={t('sideChat.drag')}
              onMouseDown={onDragHandleDown}
            >
              <IconGrip size={14} />
            </span>
          </IconTip>
          {/* 项目：内置默认项（Explore / 跟随主会话）+ 项目中心书签表（已设项目）。
              选择只决定本面板下发给模型的「工作目录」，
              不调用 set_project_dir——临时伙伴看别的项目不应改写主会话。 */}
          <div className="scp-project-wrap" ref={projectRef}>
            <IconTip
              align="left"
              text={
                projectPickerOpen
                  ? ''
                  : isExplore
                    ? t('sideChat.projectExploreHint')
                    : activeProject
                      ? activeProject.path
                      : t('sideChat.projectFollowHint')
              }
            >
              <button
                type="button"
                className={`scp-project-btn${activeProject || isExplore ? ' is-custom' : ''}`}
                aria-haspopup="menu"
                aria-expanded={projectPickerOpen}
                aria-label={t('sideChat.project')}
                onClick={() => setProjectPickerOpen(o => !o)}
              >
                {isExplore ? (
                  <IconBrowser size={13} className="scp-project-icon" />
                ) : (
                  <IconFolder size={13} className="scp-project-icon" />
                )}
                <span className="scp-project-name">{projectLabel}</span>
                <IconChevronDown size={11} className="scp-project-caret" />
              </button>
            </IconTip>
            {projectPickerOpen && (
              <div className="scp-project-menu" role="menu">
                {/* Explore：内置默认项，不属于任何项目路径。选中后本轮不绑定项目
                    （后端生成「无项目」段，覆盖环境段里主会话的项目目录）。 */}
                <button
                  type="button"
                  role="menuitem"
                  className={`scp-project-opt${isExplore ? ' is-active' : ''}`}
                  onClick={() => {
                    setProjectPath(EXPLORE_PROJECT)
                    setProjectPickerOpen(false)
                  }}
                >
                  <span className="scp-project-opt-name">{t('sideChat.projectExplore')}</span>
                  <span className="scp-project-sub">{t('sideChat.projectExploreHint')}</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className={`scp-project-opt${projectPath ? '' : ' is-active'}`}
                  onClick={() => {
                    setProjectPath('')
                    setProjectPickerOpen(false)
                  }}
                >
                  <span className="scp-project-opt-name">{t('sideChat.projectFollow')}</span>
                  <span className="scp-project-sub">
                    {mainProject.path || t('sideChat.projectNone')}
                  </span>
                </button>
                {projects.map(p => (
                  <button
                    key={p.path}
                    type="button"
                    role="menuitem"
                    className={`scp-project-opt${projectPath === p.path ? ' is-active' : ''}`}
                    title={p.path}
                    onClick={() => {
                      setProjectPath(p.path)
                      setProjectPickerOpen(false)
                    }}
                  >
                    <span className="scp-project-opt-name">{p.name}</span>
                    <span className="scp-project-sub">{p.path}</span>
                  </button>
                ))}
                {projects.length === 0 && (
                  <div className="scp-project-empty">{t('sideChat.projectEmpty')}</div>
                )}
              </div>
            )}
          </div>
        </div>
        {/* 右组：历史（保存/清空/归档）· 分享 · 收起 · 关闭——面板级操作归一，
            最小化用下拉箭头（语义=向下收起），分享用 Share（语义=向外分发） */}
        <div className="scp-header-group">
          {/* 历史：保存 / 清空 / 最近执行归档三合一（hover 展开）。
              三个动作都是面板级低频操作，收进一个入口后顶栏只留一个图标。 */}
          <div
            className="scp-history-wrap"
            ref={historyRef}
            onMouseEnter={handleHistoryEnter}
            onMouseLeave={handleHistoryLeave}
          >
            <IconTip text={historyOpen ? '' : t('sideChat.historyTip')}>
              <IconButton
                variant="raw"
                className="scp-icon-btn"
                label={t('sideChat.history')}
                nativeTitle={false}
                aria-expanded={historyOpen}
                onClick={() => (historyOpen ? setHistoryOpen(false) : void openHistory())}
              >
                <MorphIcon icon={ArchiveIcon} size={15} spring="snappy" />
              </IconButton>
            </IconTip>
            {historyOpen && (
              <div className="scp-history-menu" role="menu" ref={historyMenuRef}>
                <button
                  type="button"
                  role="menuitem"
                  className="scp-history-action"
                  onClick={() => void handleSave()}
                >
                  <IconSave size={13} />
                  {t('sideChat.save')}
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="scp-history-action is-danger"
                  onClick={() => void handleClear()}
                >
                  <IconTrash2 size={13} />
                  {t('sideChat.clear')}
                </button>
                <div className="scp-history-sep" aria-hidden="true" />
                <div className="scp-history-title">{t('sideChat.historyList')}</div>
                {historyItems.length === 0 ? (
                  <div className="scp-history-empty">{t('sideChat.historyEmpty')}</div>
                ) : (
                  <ul className="scp-history-list" ref={historyListRef}>
                    {historyItems.map(item => (
                      <li key={item.id} className="scp-history-item">
                        <span className="scp-history-ts">{item.ts.slice(5, 16)}</span>
                        <span className="scp-history-q" title={item.question}>
                          {item.question}
                        </span>
                        <button
                          type="button"
                          className="scp-history-del"
                          aria-label={t('sideChat.historyDelete')}
                          title={t('sideChat.historyDelete')}
                          onClick={e => {
                            e.stopPropagation()
                            void removeHistory(item.id)
                          }}
                        >
                          <IconX size={11} />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            )}
          </div>
          {/* 分享：同款 md 复制到剪贴板（execCommand 兜底） */}
          <IconTip text={t('sideChat.share')}>
            <IconButton
              variant="raw"
              className="scp-icon-btn"
              label={t('sideChat.share')}
              nativeTitle={false}
              onClick={() => void handleShare()}
            >
              <MorphIcon icon={ShareIcon} size={15} spring="snappy" />
            </IconButton>
          </IconTip>
          {/* 放大 / 还原：一键在「默认几何」与「按应用窗口自适应放大」间切换。
              宽高取宿主比例（62% / 88%），分屏小窗也不会溢出；拉过边缘后
              按钮语义变为「还原」，点它回到初始几何。 */}
          <IconTip text={panelZoomed ? t('sideChat.zoomReset') : t('sideChat.zoomIn')}>
            <IconButton
              variant="raw"
              className="scp-icon-btn"
              label={panelZoomed ? t('sideChat.zoomReset') : t('sideChat.zoomIn')}
              nativeTitle={false}
              onClick={togglePanelZoom}
            >
              {/* 放大 ↔ 还原：同一 MorphIcon 常驻，仅切换 icon prop → 触发形变 */}
              <MorphIcon
                icon={panelZoomed ? MinimizeIcon : MaximizeIcon}
                size={15}
                spring="snappy"
              />
            </IconButton>
          </IconTip>
          {/* 收起：只最小化，不销毁会话、不清空消息（左下角入口出角标） */}
          {onMinimize && (
            <IconTip text={t('sideChat.minimize')}>
              <IconButton
                variant="raw"
                className="scp-icon-btn"
                label={t('sideChat.minimize')}
                nativeTitle={false}
                onClick={onMinimize}
              >
                <IconChevronDown size={15} />
              </IconButton>
            </IconTip>
          )}
          <IconTip text={t('sideChat.close')}>
            <IconButton
              variant="raw"
              className="scp-icon-btn"
              label={t('sideChat.close')}
              nativeTitle={false}
              onClick={onClose}
            >
              <IconX size={15} />
            </IconButton>
          </IconTip>
        </div>
      </header>

      {/* ── 并行状态细提示条（协作契约的双向可见）──
           两条信息共用一条 11px 细条，零新色相（--fg-3 / --accent 既有 token）：
           ① 主会话正在执行的 Exec 标题（父组件从 task_runs 快照透传，只读展示，
              侧边没有任何暂停 / 抢占 Exec 的机制——隔离边界）；
           ② 交付箱积压：结论被 Finalizing 拒收后落箱，等伙伴下一轮完成时补投。
           两条都没有时整块不渲染，不占面板高度。 */}
      {(mainExecTitle || pendingDeliveries > 0) && (
        <div className="scp-parallel-hints">
          {mainExecTitle && (
            <div className="scp-parallel-hint">
              <span className="scp-parallel-hint-text">
                {t('sideChat.mainExecRunning').replace('{title}', mainExecTitle)}
              </span>
            </div>
          )}
          {pendingDeliveries > 0 && (
            <div className="scp-parallel-hint is-pending">
              <span className="scp-parallel-hint-text">
                {t('sideChat.pendingDeliveries').replace('{n}', String(pendingDeliveries))}
              </span>
            </div>
          )}
        </div>
      )}

      {/* ── 消息区：用户右 / 助手左；空态用结构化说明而不是一行小字 ──
           wrap 是回底按钮的定位包含块：按钮必须浮在滚动视口上，不能钉进内容坐标。 */}
      <div className="scp-messages-wrap">
        <div className="scp-messages" ref={scrollRef} onScroll={onScroll}>
          {messages.length === 0 && !sending && (
            /* 空态 DOM：头部（图标 + 标题 + 一句定位）+ 键值对列表。
             排版只有一种规则——全左对齐，标签列固定宽度，每条内容左边界一致。 */
            <div className="scp-empty">
              <div className="scp-empty-head">
                <span className="scp-empty-icon">
                  <IconBot size={18} />
                </span>
                <div className="scp-empty-heading">
                  <h3 className="scp-empty-title">{t('sideChat.emptyTitle')}</h3>
                  <p className="scp-empty-sub">{t('sideChat.emptySub')}</p>
                </div>
              </div>
              <ul className="scp-empty-points">
                <li>
                  <span className="scp-empty-k">{t('sideChat.kUse')}</span>
                  <span>{t('sideChat.pointUse')}</span>
                </li>
                <li>
                  <span className="scp-empty-k">{t('sideChat.kIsolate')}</span>
                  <span>{t('sideChat.pointIsolate')}</span>
                </li>
                <li>
                  <span className="scp-empty-k">{t('sideChat.kLimit')}</span>
                  <span>{t('sideChat.pointLimit')}</span>
                </li>
              </ul>
            </div>
          )}
          {messages.map((msg, i) => {
            // 执行中追加的消息不进列表：改渲染在输入框上方（见 queuedAppends），
            // 流式期间也可见。返回 null 而不是 filter——下标 i 是
            // lastAssistantIdx 的键，过滤会让操作条整体错位。
            if (msg.queued) return null
            const steps = msg.steps ?? []
            const hasSteps = steps.length > 0
            return (
              <Fragment key={i}>
                <div
                  className={`scp-bubble scp-bubble--${msg.role}${msg.error ? ' is-error' : ''}`}
                >
                  {/* 执行过程摘要（2026-11-04 所有者定）：只留一行静态摘要
                      「已用 N 个工具」——展开内容全是工具名与路径、无信息量，
                      展开交互整体移除（完成消息与实时过程区一并取消）。 */}
                  {msg.role === 'assistant' && hasSteps && (
                    <div className="scp-process">
                      <div className="scp-process-row is-static">
                        <IconBookOpen size={13} className="scp-process-icon" />
                        <span className="scp-process-text">
                          {t('sideChat.stepsUsed').replace('{n}', String(steps.length))}
                        </span>
                      </div>
                    </div>
                  )}
                  <div className="scp-bubble-body">
                    {/* Leader 派发来源徽标：用户一眼区分「本人在面板发的」与
                      「Leader 后端派来的」，派发任务不再是黑盒。 */}
                    {msg.role === 'user' && msg.from === 'leader_dispatch' && (
                      <div className="scp-bubble-from">{t('sideChat.fromLeaderDispatch')}</div>
                    )}
                    {/* 助手回复走 Markdown 渲染（与主会话同一组件）；用户消息与
                       错误提示保持纯文本——附件行 / 错误串不该被 Markdown 解释。 */}
                    {msg.role === 'assistant' && msg.error ? (
                      <div className="scp-error">
                        <div className="scp-error-title">{msg.text}</div>
                        {msg.errorHint && <div className="scp-error-hint">{msg.errorHint}</div>}
                        {msg.errorDetail && (
                          <details className="scp-error-detail">
                            <summary>{t('sideChat.errorDetail')}</summary>
                            <code>{msg.errorDetail}</code>
                          </details>
                        )}
                      </div>
                    ) : msg.role === 'assistant' ? (
                      <MarkdownContent content={msg.text} />
                    ) : (
                      msg.text
                    )}
                  </div>
                </div>
                {/* 底部单行操作：只挂在最后一条助手回复下。两个动作都由用户显式
                  触发，送出去的都是全文（chip 显示才截断）。 */}
                {i === lastAssistantIdx && msg.role === 'assistant' && !msg.error && (
                  <div className="scp-msg-actions">
                    {/* ctx 消耗：口径与主会话输入框完全一致（used 染状态色，
                      分母是当前模型容量）。0 = 无官方读数 → 显示 `--`。 */}
                    <IconTip text={t('sideChat.ctxHint')} place="above">
                      <span className="scp-msg-ctx">
                        <span className="scp-msg-ctx-label">ctx</span>
                        <span
                          className="scp-msg-ctx-value"
                          style={{ color: ctxToneOf(msg.ctxUsed ?? 0, ctxCap) }}
                        >
                          {(msg.ctxUsed ?? 0) > 0 ? formatTokens(msg.ctxUsed ?? 0) : '--'}
                        </span>
                        <span className="scp-msg-ctx-sep">/</span>
                        <span className="scp-msg-ctx-cap">
                          {ctxCap > 0 ? formatTokens(ctxCap) : '--'}
                        </span>
                      </span>
                    </IconTip>
                    {/* 两个动作只留纯图标（svg+文字与整体风格不切合）：分工靠图标
                      形状 + tips 说明，结果差异照旧——一个只放进输入栏（可改可删、
                      不触发执行），一个立即发出。 */}
                    <IconTip text={t('sideChat.sendToMainHint')} place="above">
                      <button
                        type="button"
                        className="scp-msg-action-btn"
                        aria-label={t('sideChat.sendToMainHint')}
                        onClick={() => onSendToMain(msg.text)}
                      >
                        <IconArrowRight size={13} />
                      </button>
                    </IconTip>
                    <IconTip text={t('sideChat.submitToMainHint')} place="above">
                      <button
                        type="button"
                        className="scp-msg-action-btn is-submit"
                        aria-label={t('sideChat.submitToMainHint')}
                        onClick={() => onSubmitToMain(msg.text)}
                      >
                        <IconSend size={13} />
                      </button>
                    </IconTip>
                  </div>
                )}
              </Fragment>
            )
          })}
          {/* ── 执行中：有实时数据（工具调用 / 流式文本）显示过程区，
              否则回落「思考中」三点（后端还没推任何事件时）── */}
          {sending && liveActive && (
            <div className="scp-process is-live" aria-live="polite">
              <div className="scp-process-row is-static">
                {shownTool ? (
                  <IconWrench size={13} className="scp-process-icon" />
                ) : lastSegKind === 'text' ? (
                  <IconBookOpen size={13} className="scp-process-icon" />
                ) : (
                  <IconBot size={13} className="scp-process-icon" />
                )}
                <span className="scp-live-tool">
                  {shownTool ? (
                    <>
                      <span className="scp-live-tool-name">
                        {headCall && headCall.n > 1 ? headCall.call.tool : shownTool.tool}
                        {headCall && headCall.n > 1 ? ` ×${headCall.n}` : ''}
                      </span>
                      {(headCall && headCall.n > 1 ? headCall.call.summary : shownTool.summary) && (
                        <span className="scp-live-tool-summary">
                          {headCall && headCall.n > 1 ? headCall.call.summary : shownTool.summary}
                        </span>
                      )}
                      {/* 尾部 meta：与移动端执行过程弹窗同口径——在途旋转图标，
                        结束后成功 / 失败图标 + 事件带回的耗时。 */}
                      <span className="scp-live-tool-meta">
                        {shownTool.ok === undefined ? (
                          <IconLoader2
                            size={13}
                            className="scp-live-tool-spin"
                            aria-label={t('sideChat.toolRunning')}
                          />
                        ) : (
                          <>
                            {shownTool.ok ? (
                              <IconCheck
                                size={13}
                                className="is-ok"
                                aria-label={t('sideChat.toolOk')}
                              />
                            ) : (
                              <IconX
                                size={13}
                                className="is-fail"
                                aria-label={t('sideChat.toolFail')}
                              />
                            )}
                            {shownTool.durationMs != null && (
                              <span className="scp-live-tool-duration">
                                ({formatDuration(shownTool.durationMs)})
                              </span>
                            )}
                          </>
                        )}
                      </span>
                    </>
                  ) : (
                    <span className="scp-live-tool-name is-idle">
                      {lastSegKind === 'thinking'
                        ? t('sideChat.thinking')
                        : lastSegKind === 'text'
                          ? t('sideChat.generating')
                          : t('sideChat.liveRunning')}
                    </span>
                  )}
                </span>
              </div>
              {!liveCollapsed && (
                <div className="scp-live-content">
                  {/* 交错渲染：工具块落在它发生时的正文位置，而不是全部堆到末尾。
                     样式对齐移动端执行过程弹窗：思考为次级色、正文为主色，
                     工具行为「工具名 + 参数代码块 + 状态图标 + 耗时」。 */}
                  {liveBlocks.length > 0 ? (
                    <div className="scp-timeline is-live">
                      {liveBlocks.map((b, i) => {
                        if (b.kind === 'text') {
                          return (
                            <div className="scp-live-text" key={i}>
                              {b.text}
                            </div>
                          )
                        }
                        if (b.kind === 'thinking') {
                          return (
                            <div className="scp-live-text is-thinking" key={i}>
                              {b.text}
                            </div>
                          )
                        }
                        const paramsText = formatToolParams(b.params) || b.summary
                        return (
                          <div className="scp-tl-tool" key={i}>
                            <div className="scp-tl-tool-main">
                              <span className="scp-tl-tool-name">{b.tool}</span>
                              {paramsText && (
                                <code className="scp-tl-tool-params">{paramsText}</code>
                              )}
                            </div>
                            {b.ok === undefined ? (
                              <IconLoader2
                                size={13}
                                className="scp-tl-spin"
                                aria-label={t('sideChat.toolRunning')}
                              />
                            ) : b.ok ? (
                              <IconCheck
                                size={13}
                                className="is-ok"
                                aria-label={t('sideChat.toolOk')}
                              />
                            ) : (
                              <IconX
                                size={13}
                                className="is-fail"
                                aria-label={t('sideChat.toolFail')}
                              />
                            )}
                            {b.ok !== undefined && b.durationMs != null && (
                              <span className="scp-tl-duration">
                                ({formatDuration(b.durationMs)})
                              </span>
                            )}
                          </div>
                        )
                      })}
                    </div>
                  ) : (
                    streamSegments.map((seg, i) => (
                      <div
                        key={i}
                        className={`scp-live-text${seg.kind === 'thinking' ? ' is-thinking' : ''}`}
                      >
                        {seg.text}
                      </div>
                    ))
                  )}
                </div>
              )}
            </div>
          )}
          {/* 流式模块的固定操作条：sticky 贴在滚动视口底部、高度恒定——不随流式
             内容增长跑位（旧实现把收起钮 absolute 在 .scp-live-content 里，内容
             一涨钮就跟着往下漂，平滑追随跟不上时直接滑出视口）。收起/展开二态
             共用这一个钮：收起后只剩上面那一行过程摘要，盯结果时不被刷屏。 */}
          {sending && liveActive && (
            <div className="scp-live-bar">
              <button
                type="button"
                className="scp-live-collapse"
                title={liveCollapsed ? t('sideChat.expand') : t('sideChat.collapse')}
                aria-label={liveCollapsed ? t('sideChat.expand') : t('sideChat.collapse')}
                aria-expanded={!liveCollapsed}
                onClick={() => setLiveCollapsed(v => !v)}
              >
                {liveCollapsed ? <IconChevronDown size={11} /> : <IconChevronUp size={11} />}
                {liveCollapsed ? t('sideChat.expand') : t('sideChat.collapse')}
              </button>
            </div>
          )}
          {sending && !liveActive && (
            <div className="scp-process is-live" aria-live="polite">
              <div className="scp-process-row is-static">
                <IconBot size={13} className="scp-process-icon" />
                <span className="scp-thinking-wrap">
                  <span className="scp-dots" aria-hidden="true">
                    <span className="scp-dot" />
                    <span className="scp-dot" />
                    <span className="scp-dot" />
                  </span>
                  <span className="scp-thinking-label">{t('sideChat.thinking')}</span>
                </span>
              </div>
            </div>
          )}
          {/* 回底按钮：跟随态被用户手势解锁（正在上翻阅读）时才出现，
            点击恢复跟随并平滑回底——与主对话窗口同一交互语义。 */}
          {showJumpButton && (
            <button
              type="button"
              className={`scp-jump-bottom${sending && liveActive ? ' is-raised' : ''}`}
              aria-label={t('sideChat.jumpBottom')}
              title={t('sideChat.jumpBottom')}
              onClick={jumpToBottom}
            >
              <IconChevronDown size={14} />
            </button>
          )}
          {/* 流式执行「活着」指示：浮在消息内容区左下角（2026-11-04 所有者定——
              放 .scp-live-bar 里会被它的实色背景遮挡，故回到内容区浮层）。
              左缘 22px：比贴边再让 10px，避开窗体圆角/边缘。
              **只在 liveActive 时出现**（流式/工具活动期，即最初的「流式执行底部」
              需求）：连接/思考阶段（sending && !liveActive）由「思考中…」三点动画
              传达存活——那时 pill 会盖住那行，故不渲染。
              z-index:10 + bottom:10px（CSS）：盖在 live bar（z-index:6）之上、
              略离视口底、压条的上沿——实色底+投影读作浮层芯片。
              sending 是后端 slot.running 的前端镜像——它真才显示，绝不伪造活跃。 */}
          {sending && liveActive && (
            <span className="scp-running" role="status">
              <span className="scp-running-dot" aria-hidden="true" />
              <span className="scp-running-text">Running..</span>
              <span className="scp-running-sweep" aria-hidden="true" />
            </span>
          )}
        </div>
      </div>

      {/* ── 输入区：输入框与发送钮同一个容器，内部不做区域分割 ── */}
      <footer className="scp-composer">
        {/* 执行中追加的消息：固定在输入框上方（不随消息列表滚动），流式输出
            期间始终可见。每条可单独取消——取消按内容从后端隔离队列移除，
            已被迭代边界消费则明示「已生效」。空态 / 非执行态整块不渲染。 */}
        {sending && queuedAppends.length > 0 && (
          <div className="scp-append-queue">
            <div className="scp-append-queue-hint">
              <IconClock3 size={11} aria-hidden="true" />
              <span>{t('sideChat.appendQueued')}</span>
            </div>
            {queuedAppends.map((msg, i) => (
              <div className="scp-append-queue-item" key={`${i}-${msg.text}`}>
                <span className="scp-append-queue-text" title={msg.text}>
                  {msg.text}
                </span>
                {/* Leader 派发来的追加：与用户自己追加区分来源 */}
                {msg.from === 'leader_dispatch' && (
                  <span className="scp-append-queue-from">{t('sideChat.fromLeaderDispatch')}</span>
                )}
                <button
                  type="button"
                  className="scp-append-queue-x"
                  aria-label={t('sideChat.appendCancel')}
                  title={t('sideChat.appendCancel')}
                  onClick={() => void cancelAppend(msg.text)}
                >
                  <IconX size={11} />
                </button>
              </div>
            ))}
          </div>
        )}
        {/* 附件 / skill chips：输入框上方成行（380px 内换行），单 chip 单行 ellipsis */}
        {(attachments.length > 0 || skills.length > 0) && (
          <div className="scp-chips">
            {attachments.map((a, i) => (
              <span className="scp-chip" key={`att-${i}-${a.name}`}>
                <IconPaperclip size={11} className="scp-chip-icon" />
                <span className="scp-chip-label" title={a.name}>
                  {a.name}
                </span>
                <button
                  type="button"
                  className="scp-chip-x"
                  aria-label={t('sideChat.removeAttach')}
                  title={t('sideChat.removeAttach')}
                  onClick={() => removeAttachment(i)}
                >
                  <IconX size={11} />
                </button>
              </span>
            ))}
            {skills.map((s, i) => (
              <span className="scp-chip" key={`skill-${s.id}`}>
                <IconPuzzle size={11} className="scp-chip-icon" />
                <span className="scp-chip-label" title={s.label}>
                  {s.label}
                </span>
                <button
                  type="button"
                  className="scp-chip-x"
                  aria-label={t('sideChat.removeSkill')}
                  title={t('sideChat.removeSkill')}
                  onClick={() => removeSkill(i)}
                >
                  <IconX size={11} />
                </button>
              </span>
            ))}
          </div>
        )}
        <textarea
          className="scp-input"
          rows={2}
          value={input}
          placeholder={t('sideChat.placeholder')}
          aria-label={t('sideChat.placeholder')}
          onChange={e => setInput(e.target.value)}
          onKeyDown={e => {
            if (
              e.key === 'Enter' &&
              !e.shiftKey &&
              !e.ctrlKey &&
              !e.metaKey &&
              !e.nativeEvent.isComposing
            ) {
              e.preventDefault()
              void handleSend()
            }
          }}
        />
        {/* 隐藏文件选择：Tauri 下 File.path 即绝对路径；浏览器回落文件名（hud 明示） */}
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="scp-file-input"
          aria-hidden="true"
          tabIndex={-1}
          onChange={onFilesPicked}
        />
        <div className="scp-composer-bar">
          {/* 「+」小菜单：添加附件 / 选择技能（菜单绝对定位，不超出面板） */}
          <div className="scp-plus-wrap" ref={plusMenuRef}>
            <IconButton
              variant="raw"
              className="scp-icon-btn"
              label={t('sideChat.add')}
              title={t('sideChat.add')}
              aria-expanded={plusMenuOpen}
              onClick={() => setPlusMenuOpen(o => !o)}
            >
              <IconPlus size={15} />
            </IconButton>
            {plusMenuOpen && (
              <div className="scp-plus-menu" role="menu">
                <button
                  type="button"
                  role="menuitem"
                  className="scp-plus-item"
                  onClick={() => {
                    setPlusMenuOpen(false)
                    fileInputRef.current?.click()
                  }}
                >
                  <IconPaperclip size={13} />
                  {t('sideChat.attachFile')}
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="scp-plus-item"
                  onClick={() => void openSkillPicker()}
                >
                  <IconPuzzle size={13} />
                  {t('sideChat.pickSkill')}
                </button>
              </div>
            )}
          </div>
          {/* 模型选择：点击弹提供商浮层（懒加载：打开才拉列表，展开才渲染该商模型） */}
          <div className="scp-model-picker" ref={modelPickerRef}>
            <button
              type="button"
              className="scp-model-trigger"
              aria-label={t('sideChat.model')}
              aria-expanded={modelPickerOpen}
              onClick={() => setModelPickerOpen(o => !o)}
            >
              <span className="scp-model-value">
                {model || modelName || t('sideChat.modelFollow')}
              </span>
              <IconChevronDown size={12} className="scp-model-caret" />
            </button>
            {modelPickerOpen && (
              <div className="scp-model-menu" role="listbox" aria-label={t('sideChat.model')}>
                <button
                  type="button"
                  className={`scp-model-opt${model === '' ? ' is-active' : ''}`}
                  onClick={() => pickModel('')}
                >
                  <span className="scp-model-opt-name">{t('sideChat.modelDefault')}</span>
                  <span className="scp-model-opt-sub">{t('sideChat.modelFollow')}</span>
                </button>
                {providerGroups.map(group => (
                  <div className="scp-provider" key={group.id}>
                    <button
                      type="button"
                      className="scp-provider-row"
                      aria-expanded={expandedProvider === group.id}
                      onClick={() =>
                        setExpandedProvider(cur => (cur === group.id ? null : group.id))
                      }
                    >
                      {hasProviderIcon(group.id) ? (
                        <ProviderIcon provider={group.id} size={16} />
                      ) : (
                        <span className="scp-provider-fallback">
                          {group.id.slice(0, 1).toUpperCase()}
                        </span>
                      )}
                      <span className="scp-provider-name">{group.id}</span>
                      <span className="scp-provider-count">{group.models.length}</span>
                      <IconChevronRight
                        size={13}
                        className={`scp-provider-caret${
                          expandedProvider === group.id ? ' is-open' : ''
                        }`}
                      />
                    </button>
                    {/* 懒加载：仅展开中的提供商渲染模型子列表，未展开不进 DOM */}
                    {expandedProvider === group.id && (
                      <div className="scp-provider-models">
                        {group.models.map(m => (
                          <button
                            key={m.id}
                            type="button"
                            className={`scp-model-opt${model === m.id ? ' is-active' : ''}`}
                            onClick={() => pickModel(m.id, group.id)}
                          >
                            <span className="scp-model-id">{m.id}</span>
                            {/* 能力标注：上下文窗口（紧凑读数）+ 多模态（视觉） */}
                            <span className="scp-model-tags">
                              <span className="scp-model-tag">
                                {m.context_window
                                  ? formatTokens(m.context_window)
                                  : t('sideChat.ctxUnknown')}
                              </span>
                              {m.supports_vision && (
                                <span className="scp-model-tag is-vision">
                                  {t('sideChat.vision')}
                                </span>
                              )}
                            </span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
          {/* 推理强度：仅当模型声明 reasoning_efforts 时出现（能力契约，不伪造等级）。
                触发器 = 纯文本（当前档位）+ 下拉箭头；绝不带多余的 SVG 图标。 */}
          {effortAvailable && (
            <div className="scp-effort-picker" ref={effortRef}>
              <button
                type="button"
                className="scp-effort-trigger"
                aria-label={t('sideChat.effort')}
                title={t('sideChat.effortTitle')}
                aria-expanded={effortOpen}
                onClick={() => setEffortOpen(o => !o)}
              >
                <span className="scp-effort-value">{effort ?? t('sideChat.effortDefault')}</span>
                <IconChevronDown size={11} className="scp-effort-caret" />
              </button>
              {effortOpen && (
                <div className="scp-effort-menu" role="menu" aria-label={t('sideChat.effort')}>
                  <button
                    type="button"
                    className={`scp-effort-opt${effort === null ? ' is-active' : ''}`}
                    onClick={() => {
                      setEffort(null)
                      setEffortOpen(false)
                    }}
                  >
                    {t('models.reasoningDefault')}
                  </button>
                  {supportedEfforts.map(e => (
                    <button
                      key={e}
                      type="button"
                      className={`scp-effort-opt${effort === e ? ' is-active' : ''}`}
                      onClick={() => {
                        setEffort(e)
                        setEffortOpen(false)
                      }}
                    >
                      {e}
                      {e === 'high' ? ` (${t('models.reasoningHighHint')})` : ''}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
          <div className="scp-composer-actions">
            <span className="scp-composer-divider" aria-hidden="true" />
            {/* 发送中：发送钮让位给「追加 + 停止」并排。
                - 追加（有内容时才出现）：不打断本轮，入队后由下一个迭代边界注入；
                - 停止：置位后端中断，会话与消息保留，结算后发送钮回来。
                两者视觉分离（追加是常规动作，停止是危险动作），不合并成一个键。 */}
            {sending ? (
              <>
                {input.trim() && (
                  <IconButton
                    variant="raw"
                    className="scp-send is-append"
                    label={t('sideChat.append')}
                    title={t('sideChat.appendHint')}
                    onClick={() => void handleAppend()}
                  >
                    <IconSend size={15} />
                  </IconButton>
                )}
                <IconButton
                  variant="raw"
                  className="scp-send is-stop"
                  label={t('sideChat.stop')}
                  onClick={handleStop}
                >
                  <IconSquare size={14} />
                </IconButton>
              </>
            ) : (
              <IconButton
                variant="raw"
                className="scp-send"
                label={t('sideChat.send')}
                disabled={!input.trim()}
                onClick={() => void handleSend()}
              >
                <IconSend size={16} />
              </IconButton>
            )}
          </div>
          {/* 技能选择浮层：贴图标行向上展开（与模型/思考强度浮层同一包含块），长列表内部滚动 */}
          {skillPickerOpen && (
            <div
              className="scp-skill-menu"
              role="listbox"
              aria-label={t('sideChat.pickSkill')}
              ref={skillPickerRef}
            >
              {skillOptions.length === 0 ? (
                <div className="scp-skill-empty">{t('sideChat.skillEmpty')}</div>
              ) : (
                skillOptions.map(s => (
                  <button
                    key={s.name}
                    type="button"
                    role="option"
                    aria-selected={false}
                    className="scp-skill-opt"
                    onClick={() => pickSkill(s.name, s.label)}
                  >
                    <IconPuzzle size={13} className="scp-chip-icon" />
                    <span className="scp-skill-opt-name">{s.label}</span>
                  </button>
                ))
              )}
            </div>
          )}
        </div>
      </footer>
      {/* 边缘拉伸把手：右下角逐像素拖拽改宽高（上限=宿主容器-40px）。
          放在面板最上层，避开滚动条与消息区命中。 */}
      <button
        type="button"
        className="scp-resize-handle"
        aria-label={t('sideChat.resize')}
        title={t('sideChat.resize')}
        onMouseDown={onResizeHandleDown}
      />
    </aside>
  )
}

/**
 * memo：本面板是 ChatPanel 的子节点。主会话每个流式 token / 工具事件都会让
 * ChatPanel 重渲染，不隔断的话伙伴面板整套 JSX（消息列表 + 实时过程 + 输入区）
 * 跟着每秒几十次重渲染——体感就是「主会话一执行，伙伴窗口输入发沉、点击迟钝，
 * 指令像是卡住，等主会话跑完才动弹」。ChatPanel 传入的九个 prop 全是稳定引用
 * （useCallback / setState / 原始值），memo 因此真能挡住父级重渲染；
 * 若今后给面板加内联箭头 prop，需重新评估这一条。
 */
export const SideChatPanel = memo(SideChatPanelImpl)
export default SideChatPanel
