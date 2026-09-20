import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconEdit3,
  IconFolder,
  IconMoreHorizontal,
  IconPlus,
  IconRestore,
  IconTrash2,
  IconX,
} from '../../ui/Icons'
import { playUiSound } from '../../ui/sound'
import { CompactModal } from '../layout/CompactModal'
import { useLanguage } from '../../locales'
import {
  listShelfSessions,
  switchSession,
  renameSession,
  archiveSession,
  setProjectBookmarks,
  setProjectFolderArchived,
  SESSION_GROUP_LIMIT_CHANGED_EVENT,
  type ProjectBookmark,
  type ShelfProjectEntry,
  type ShelfSessionItem,
} from '../lib/api'
import {
  DEFAULT_GROUP_LIMIT,
  buildSessionGroups,
  normalizeGroupLimit,
  normalizePathKey,
  visibleGroupSessions,
  type SessionGroup,
} from './sessionGroups'
import '../../styles/session-rail.css'

const POLL_INTERVAL_MS = 5000
/** 会话变更去抖：2s 内只触发一次 onSessionChanged，防轮询翻转连续触发风暴 */
const SWITCH_NOTICE_THROTTLE_MS = 2000

/** updated_at（Unix 毫秒）→ 行尾相对时间（刚刚 / N分钟 / N小时 / N天） */
function relativeTime(ms: number, t: (key: string, ...args: string[]) => string): string {
  const minutes = Math.floor((Date.now() - ms) / 60_000)
  if (minutes < 1) return t('sessionRail.timeJustNow')
  if (minutes < 60) return t('sessionRail.timeMinutes', String(minutes))
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return t('sessionRail.timeHours', String(hours))
  return t('sessionRail.timeDays', String(Math.floor(hours / 24)))
}

/** mode → 标识首字母（方形小标只放一个字母：L=leader / W=workflow / C=custom） */
function modeToLetter(mode: string): string {
  if (mode === 'workflow') return 'W'
  if (mode === 'leader') return 'L'
  if (mode === 'custom') return 'C'
  return '·'
}

interface SessionRailProps {
  /** 切换成功后由父级重拉 get_chat_history 整体替换气泡 */
  onSessionChanged: () => void
  /** 新建对话（复用桌面统一入口 handleNewChat / Ctrl+N 同一逻辑源；执行中禁用） */
  onNewChat?: () => void
  /** 打开项目中心弹窗（复用输入框项目 chip 的同一入口：ChatPanel setDirOpen(true)） */
  onOpenProjectDir?: () => void
  /**
   * 切换工作目录（**复用** ChatPanel.switchProject 单一实现：落盘 + 后端向活跃槽注入
   * 变更提醒 + HUD 反馈）；返回 true = 已切到目标目录。
   * 组内「新建对话」/ 点击组内会话依赖此入口，不另起一套切目录逻辑。
   */
  onSwitchProjectDir?: (path: string) => Promise<boolean>
  /** 跨 mode 会话切换成功后同步前端 mode state（后端原子切换不单独广播 mode_changed，
   *  mode chip 依赖此回调保持一致） */
  onModeSwitched?: (mode: string) => void
  /** 后端执行态实时镜像（App isProcessing）：执行中条目禁用（后端 guard 双保险） */
  locked?: boolean
  /** 当前情绪（App mood）：执行错误（'error'）时结束翻转不播完成音效，
   *  避免与 execution_error 的错误音效重叠 */
  mood?: string
}

type NoticeTone = 'info' | 'warning' | 'error'

/** 错误码 → i18n key（按业务/系统错误拆分，业务等待有专属细分文案） */
function codeToI18n(code: string): string {
  if (code === 'busy') return 'sessionRail.switchFailBusy'
  if (code === 'append_pending') return 'sessionRail.switchFailAppend'
  if (code === 'mode_mismatch') return 'sessionRail.switchFailMode'
  if (code === 'archiveFailGeneric') return 'sessionRail.archiveFailGeneric'
  return 'sessionRail.switchFailGeneric'
}

/** 错误码 → 视觉 tone：业务等待用 info（蓝），模式不匹配用 warning（橙），真错误用 error（红） */
function codeToTone(code: string): NoticeTone {
  if (code === 'busy' || code === 'append_pending') return 'info'
  if (code === 'mode_mismatch') return 'warning'
  return 'error'
}

/** 通知浮层图标：按 tone 切换内嵌 SVG，避免引入额外 icon 包污染主图标库 */
function NoticeIcon({ tone }: { tone: NoticeTone }) {
  if (tone === 'warning') {
    // 三角警示
    return (
      <svg className="sr-notice-icon" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path
          d="M8 2 L14.5 13.5 L1.5 13.5 Z"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinejoin="round"
        />
        <path d="M8 6 V9.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        <circle cx="8" cy="11.5" r="0.9" fill="currentColor" />
      </svg>
    )
  }
  if (tone === 'error') {
    // 圆 + 叹号
    return (
      <svg className="sr-notice-icon" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <circle cx="8" cy="8" r="6.2" stroke="currentColor" strokeWidth="1.4" />
        <path d="M8 5 V9.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        <circle cx="8" cy="11.5" r="0.9" fill="currentColor" />
      </svg>
    )
  }
  // 圆 + i（业务等待：正常拒绝而非崩溃）
  return (
    <svg className="sr-notice-icon" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <circle cx="8" cy="8" r="6.2" stroke="currentColor" strokeWidth="1.4" />
      <path d="M8 7.2 V11" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      <circle cx="8" cy="4.9" r="0.9" fill="currentColor" />
    </svg>
  )
}

/** 归档确认弹窗目标：会话（单条）或项目文件夹（整组隐藏） */
type ArchiveTarget =
  { kind: 'session'; id: string } | { kind: 'project'; path: string; name: string }

/**
 * 会话工作台（Session Rail）：聊天面板左缘的滑动抽屉。
 * - 收起态：左缘常驻一枚色块（会话图标 + 当前会话 mode 首字母），是唯一可见元素。
 * - 展开态：点击色块 → 左侧列表滑出（**按项目文件夹分组**渲染：组头=文件夹名 +
 *   展开/折叠 + 组内新建/重命名/归档，组内会话沿用小胶囊行样式）。
 * - 开合入口只有三个：色块点击、面板外点击、Esc；**不做 hover 感应唤出，
 *   执行完成也不自动弹出**（2026-09-15 大王反馈：隐藏式选择看不到会话标题）。
 * - 数据源与切换逻辑完全沿用：list_shelf_sessions（5s 轮询 + 可见性刷新），
 *   分组完全来自返回体的 items/projects/archived_projects/collapsed_limit
 *   （**不推断归属**：project_path=null 者进「未分组」兜底组；归档文件夹整组隐藏）。
 * - busy / 追加队列非空时后端拒绝 → 错误码映射文案在抽屉底部短暂浮现。
 */
export default function SessionRail({
  onSessionChanged,
  onNewChat,
  onOpenProjectDir,
  onSwitchProjectDir,
  onModeSwitched,
  locked = false,
  mood,
}: SessionRailProps) {
  const { t } = useLanguage()
  const [items, setItems] = useState<ShelfSessionItem[]>([])
  /** 可见项目文件夹（组顺序 = 此数组顺序：书签序 → auto） */
  const [projects, setProjects] = useState<ShelfProjectEntry[]>([])
  /** 已归档项目文件夹（整组隐藏；菜单内提供恢复入口） */
  const [archivedProjects, setArchivedProjects] = useState<ShelfProjectEntry[]>([])
  /** 全局组内折叠上限（后端 collapsed_limit；设置中心可改，事件即时生效） */
  const [groupLimit, setGroupLimit] = useState(DEFAULT_GROUP_LIMIT)
  const [canSwitch, setCanSwitch] = useState(true)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draftTitle, setDraftTitle] = useState('')
  /** 文件夹重命名编辑态（组 key = 归一化路径）+ 草稿名 */
  const [editingProjectKey, setEditingProjectKey] = useState<string | null>(null)
  const [projectDraft, setProjectDraft] = useState('')
  const [notice, setNotice] = useState<{ text: string; tone: NoticeTone } | null>(null)
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** 归档确认弹窗目标（会话 / 项目文件夹；null = 关闭） */
  const [archiveTarget, setArchiveTarget] = useState<ArchiveTarget | null>(null)
  /** 抽屉开合态：默认收起（只露色块），点击色块才伸出 */
  const [open, setOpen] = useState(false)
  /** 组头折叠态（key → true=收起整组）：默认全部展开，运行时状态不持久化 */
  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({})
  /** 组内「展开其余 N 个会话」态（key → true=全显）：默认按全局上限折叠 */
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({})
  /** 项目文件夹菜单（新建文件夹 / 已归档文件夹恢复）开合 */
  const [menuOpen, setMenuOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  /** 执行中点色块的轻提示（色块旁浮出，数秒后自动消失） */
  const [chipHint, setChipHint] = useState(false)
  const chipHintTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const chipRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLElement>(null)
  const stoppedRef = useRef(false)
  /** 外部会话变化检测基准：上轮轮询的 active 会话 id（null=无 active；首轮回填不触发） */
  const lastActiveIdRef = useRef<string | null>(null)
  const initializedRef = useRef(false)
  /** 上次 canSwitch 状态：检测「执行开始/结束」的状态翻转轮（只校准基准，不触发重拉） */
  const prevCanSwitchRef = useRef(true)
  /** 检测触发去抖：上次 fire 时间戳（2s 内不重复触发重拉） */
  const lastFireAtRef = useRef(0)
  /** 列表签名：上轮渲染数据指纹（id+active+标题+顺序/分组/上限），结构未变不重绘 */
  const listSigRef = useRef('')

  // ── 执行态锁定：canSwitch（轮询后端权威）或 locked（实时镜像）任一执行中 → 条目禁用 ──
  const hardLocked = !canSwitch || locked

  // 翻转方向检测：记录上一帧 hardLocked，区分「执行开始」（false→true）与
  // 「执行完成」（true→false）——初始挂载 false→false 不触发任何动作
  const prevHardLockedRef = useRef(hardLocked)
  useEffect(() => {
    const was = prevHardLockedRef.current
    prevHardLockedRef.current = hardLocked
    if (hardLocked) {
      // 执行开始：收起抽屉（执行期面板只读，不遮挡消息流）
      setOpen(false)
      setEditingId(null)
      setEditingProjectKey(null)
      setMenuOpen(false)
    } else if (was) {
      // 执行完成：只播完成音效，不再自动弹出工作台（大王 2026-09-15：完成也不自动弹出）。
      // 错误结束（mood='error'）不播完成音——execution_error 已播错误音效，避免重叠
      if (mood !== 'error') playUiSound('done')
      // 执行结束：清掉残留的「执行中不可切换」提示
      setChipHint(false)
    }
  }, [hardLocked, mood])

  // 收起抽屉：保留编辑态草稿（重新展开后仍在编辑中），不做静默丢弃
  const closeDrawer = useCallback(() => setOpen(false), [])

  /** 执行中点色块的轻提示：色块旁浮出，3s 自动消失（重复点击重置计时） */
  const flashChipHint = useCallback(() => {
    setChipHint(true)
    if (chipHintTimer.current) clearTimeout(chipHintTimer.current)
    chipHintTimer.current = setTimeout(() => {
      chipHintTimer.current = null
      setChipHint(false)
    }, 3000)
  }, [])

  // 抽屉展开期：Esc 收起（编辑中先退编辑）、点击面板与色块之外收起
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (menuOpen) {
        setMenuOpen(false)
        return
      }
      if (editingProjectKey) {
        setEditingProjectKey(null)
        return
      }
      if (editingId) {
        setEditingId(null)
        return
      }
      setOpen(false)
    }
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node | null
      if (!target) return
      // 菜单开合优先级高于抽屉：点菜单外先收菜单，不连带收起抽屉
      if (menuOpen && !menuRef.current?.contains(target)) setMenuOpen(false)
      if (panelRef.current?.contains(target)) return
      if (chipRef.current?.contains(target)) return
      setOpen(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onDown)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onDown)
    }
  }, [open, editingId, editingProjectKey, menuOpen])

  useEffect(
    () => () => {
      if (noticeTimer.current) clearTimeout(noticeTimer.current)
      if (chipHintTimer.current) clearTimeout(chipHintTimer.current)
    },
    [],
  )

  /** onSessionChanged 经 ref 间接持有：保持 refresh 引用稳定（deps 不加回调），
   *  否则父组件每次渲染重建回调会导致轮询 effect 反复重启 */
  const onSessionChangedRef = useRef(onSessionChanged)
  onSessionChangedRef.current = onSessionChanged

  const refresh = useCallback(async () => {
    try {
      const r = await listShelfSessions()
      if (!stoppedRef.current && r) {
        const list = r.items || []
        const canSwitch = r.can_switch !== false
        // 折叠上限独立于列表签名：读数变化必须立即生效（不能等条目变化）
        setGroupLimit(normalizeGroupLimit(r.collapsed_limit))
        // 签名守卫：id+active+标题+分钟桶/分组/上限未变则不 setItems——提炼/追加等后台写入只改
        // 消息内容与 updated_at，列表视图零重绘（消除轮询期闪动）；activeId 检测
        // 仍基于本轮新数据，不受影响。签名含顺序（数组序）与分组数据，新建/归档/改归属必然变化。
        // ⚠️ updated_at 以「分钟桶」入签名（非原始毫秒）：行尾相对时间需随分钟自增，
        //    否则无其它变化时列表永不重绘、时间会僵在旧值；分钟粒度最多每分钟一次重绘。
        const sig = [
          list
            .map(
              i =>
                `${i.id}|${i.is_active ? 1 : 0}|${i.title}|${i.preview || ''}|${Math.floor(
                  (i.updated_at || 0) / 60_000,
                )}|${i.project_path || ''}`,
            )
            .join(';'),
          (r.projects || [])
            .map(p => `${p.path}|${p.name}|${p.is_current ? 1 : 0}|${p.auto ? 1 : 0}`)
            .join(';'),
          (r.archived_projects || []).map(p => `${p.path}|${p.name}|${p.auto ? 1 : 0}`).join(';'),
        ].join('§')
        if (sig !== listSigRef.current) {
          listSigRef.current = sig
          setItems(list)
          setProjects(r.projects || [])
          setArchivedProjects(r.archived_projects || [])
        }
        setCanSwitch(canSwitch)
        // ── 外部会话变化检测 ──
        // 手机端「新会话」/ 遥控切换只广播给移动 WS，桌面前端没有该事件通道；
        // 本轮询是桌面感知外部会话变化的唯一信息源。
        // ⚠️ 仅在空闲态（can_switch）检测：执行期后端 get_chat_history 的会话解析源
        // 会在 session_backup 与 live agent 间漂移（实测日志交替返回），active id
        // 随之抖动——若此时比对会把执行期快照切换误判为外部变更，每次收发都整表
        // 重拉聊天区（实测回归）。执行中守卫本就禁止任何切换，不存在外部变更，
        // 直接冻结检测与基准更新。
        // ⚠️ 状态翻转轮（busy↔idle）：执行开始/结束时 active id 天然变化（agent
        // take/放回），此轮只校准基准不触发——否则「新会话/切换后第一轮回复完成」
        // 会因基准过期误判为外部变更，必然整表重拉（实测 2026-08-25）。
        const flip = prevCanSwitchRef.current !== canSwitch
        prevCanSwitchRef.current = canSwitch
        // 执行完成不再有弹窗/弹层动作；此处轮询只做整表重拉判定，flip 轮只校准基准
        const activeId = list.find(i => i.is_active)?.id ?? null
        if (flip) {
          lastActiveIdRef.current = activeId
          return
        }
        if (!canSwitch) return
        if (!initializedRef.current) {
          initializedRef.current = true
          lastActiveIdRef.current = activeId
        } else if (list.length > 0 && activeId !== lastActiveIdRef.current) {
          // 去抖：2s 内只触发一次，防连续变更风暴
          const now = Date.now()
          if (now - lastFireAtRef.current >= SWITCH_NOTICE_THROTTLE_MS) {
            lastFireAtRef.current = now
            lastActiveIdRef.current = activeId
            onSessionChangedRef.current()
          } else {
            lastActiveIdRef.current = activeId
          }
        } else {
          lastActiveIdRef.current = activeId
        }
      }
    } catch {
      /* 后端不可达：保留当前数据 */
    }
  }, [])

  useEffect(() => {
    stoppedRef.current = false
    let timer: ReturnType<typeof setTimeout> | null = null

    const poll = async () => {
      await refresh()
      if (!stoppedRef.current && document.visibilityState === 'visible') {
        timer = setTimeout(poll, POLL_INTERVAL_MS)
      }
    }

    const onVisibility = () => {
      if (document.visibilityState === 'visible') {
        if (!timer) void poll()
      } else if (timer) {
        clearTimeout(timer)
        timer = null
      }
    }

    document.addEventListener('visibilitychange', onVisibility)
    if (document.visibilityState === 'visible') void poll()
    return () => {
      stoppedRef.current = true
      if (timer) clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [refresh])

  // 设置中心改折叠上限 → 立即生效（不等下一轮 5s 轮询），并重拉一次保证数据一致
  useEffect(() => {
    const onLimitChanged = (e: Event) => {
      const detail = (e as CustomEvent<number>).detail
      setGroupLimit(normalizeGroupLimit(detail))
      void refresh()
    }
    window.addEventListener(SESSION_GROUP_LIMIT_CHANGED_EVENT, onLimitChanged)
    return () => window.removeEventListener(SESSION_GROUP_LIMIT_CHANGED_EVENT, onLimitChanged)
  }, [refresh])

  const flashNotice = useCallback(
    (code: string) => {
      const tone = codeToTone(code)
      setNotice({ text: t(codeToI18n(code)), tone })
      if (noticeTimer.current) clearTimeout(noticeTimer.current)
      // 业务等待（info）延长阅读时间；模式不匹配/真错误保留短促反馈
      const duration = tone === 'info' ? 3500 : 2400
      noticeTimer.current = setTimeout(() => setNotice(null), duration)
    },
    [t],
  )

  /** 分组视图：全部由返回体四字段派生（组顺序 / 归档隐藏 / 未分组末位 / 组内 updated_at 倒序） */
  const groups = useMemo(
    () => buildSessionGroups(items, projects, archivedProjects),
    [items, projects, archivedProjects],
  )

  /**
   * 点击组内会话 = 切 mode + 切工作目录 + 装载（决策 4）。
   *
   * **顺序：先装载会话（switch_session 原子切 mode），成功后再切工作目录。** 依据：
   * 1. `switch_session` 是唯一会**稳定失败**的一步（busy / append_pending / mode_mismatch，
   *    见 src-tauri/src/commands/process/shelf.rs:907 guard_switch、:920-928 mode_mismatch），
   *    放在前面 → 失败时全局工作目录保持原样，不会出现「目录已切、会话没换」的脏状态；
   * 2. `set_project_dir` 只写 prefs + 把变更提醒推入全局待注入队列
   *    （src-tauri/.../config/preferences.rs:402-424），提醒在**下一轮次边界**才被 drain
   *    （src/runtime/react_loop.rs:377-387）。执行中 guard_switch 已禁止任何切换，
   *    两次调用之间不存在轮次边界，故「提醒注入到即将归档的旧槽」不成立；装载完成后再
   *    切目录，保证该提醒必然由目标会话在下一轮消费。
   */
  const handleSwitch = useCallback(
    async (
      id: string,
      isActive: boolean,
      projectPath: string | null,
      dirAlreadyCurrent: boolean,
    ) => {
      if (isActive) return
      try {
        // 输入框 mode 以 session 选择为准：点击列表后，session 存储的 mode 是哪个，
        // 输入框 mode 就切到哪个。目标 mode 直接取条目存储归属（list 已保证
        // mode 来自 SQLite 快照，不依赖 currentMode 推断），无条件传给后端原子切换
        // （归档原槽→切 current_mode→安装目标槽；同 mode 点击走同槽切换，无副作用）。
        // 后端是唯一权威：mode_mismatch 安全失败，不污染状态。
        const target = items.find(i => i.id === id)
        if (!target) return
        await switchSession(id, target.mode)
        // 会话切换成功：轻触反馈（导航定位）
        playUiSound('session')
        // 同步前端输入框 mode（后端原子切换后前端 mode chip 保持一致）
        onModeSwitched?.(target.mode)
        // 主动切换：先登记基准，避免下轮轮询把这次变化再判成外部变更重复触发重拉
        lastActiveIdRef.current = id
        // 切工作目录到该会话的归属目录（无归属会话**不猜目录**：project_path=null 时跳过）；
        // 目录未变（组带 is_current）时后端 set_project_dir 本就幂等，跳过省一次 IPC
        if (projectPath && !dirAlreadyCurrent && onSwitchProjectDir) {
          // 失败已在 switchProject 内 HUD 反馈（单一通道），此处不阻断已完成装载的会话
          await onSwitchProjectDir(projectPath)
        }
        onSessionChanged()
        void refresh()
        // 切换完成即收起抽屉：立刻让出消息区视野（唯一自动收起场景，非弹出）
        setOpen(false)
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
      }
    },
    [items, onSessionChanged, onModeSwitched, onSwitchProjectDir, refresh, flashNotice],
  )

  /** 手动归档会话：确认弹窗后移出展示台（元数据+文本记忆保留可查）；失败映射稳定错误码 */
  const handleArchive = useCallback(
    async (id: string) => {
      setArchiveTarget(null)
      try {
        await archiveSession(id)
        void refresh()
      } catch {
        flashNotice('archiveFailGeneric')
      }
    },
    [refresh, flashNotice],
  )

  /** 归档项目文件夹：整组隐藏（含其下会话），可从「已归档文件夹」恢复 */
  const handleArchiveProject = useCallback(
    async (path: string) => {
      setArchiveTarget(null)
      setEditingProjectKey(null)
      try {
        await setProjectFolderArchived(path, true)
        void refresh()
      } catch {
        flashNotice('archiveFailGeneric')
      }
    },
    [refresh, flashNotice],
  )

  /** 恢复已归档项目文件夹（书签归档标记置回 false → 重新出现在组列表） */
  const handleRestoreProject = useCallback(
    async (path: string) => {
      setMenuOpen(false)
      try {
        await setProjectFolderArchived(path, false)
        void refresh()
      } catch {
        flashNotice('archiveFailGeneric')
      }
    },
    [refresh, flashNotice],
  )

  /** 归档确认弹窗「确认」：按目标类型分派 */
  const confirmArchive = useCallback(() => {
    if (!archiveTarget) return
    if (archiveTarget.kind === 'session') void handleArchive(archiveTarget.id)
    else void handleArchiveProject(archiveTarget.path)
  }, [archiveTarget, handleArchive, handleArchiveProject])

  /**
   * 重命名项目文件夹：复用书签整表替换（后端按路径去重/名称兜底，同路径自动沿用归档标记）。
   *
   * ⚠️ 提交表必须包含**已归档书签**（用 `archived_projects[]` 还原为 bookmark 条目）：
   * `set_project_bookmarks` 是整表替换，漏掉归档项等于把「已归档文件夹」记录抹掉、
   * 恢复入口随之丢失。
   */
  const saveProjectRename = useCallback(
    async (group: SessionGroup<ShelfSessionItem>) => {
      const name = projectDraft.trim()
      setEditingProjectKey(null)
      if (!name || !group.path) return
      if (name === group.name) return
      try {
        const next: ProjectBookmark[] = [
          // 书签组（auto=false，顺序即书签顺序）；auto 只读组不写回书签表
          ...projects
            .filter(p => !p.auto)
            .map(p => ({
              name: normalizePathKey(p.path) === group.key ? name : p.name,
              path: p.path,
            })),
          // 归档书签原样回填（否则整表替换会丢掉归档记录）
          ...archivedProjects.map(p => ({ name: p.name, path: p.path, archived: true })),
        ]
        await setProjectBookmarks(next)
        void refresh()
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
      }
    },
    [projectDraft, projects, archivedProjects, refresh, flashNotice],
  )

  /** 组头「+」：先切到该文件夹，再走与 Ctrl+N 同一新建入口（新会话归属即该文件夹） */
  const handleNewChatInGroup = useCallback(
    async (path: string) => {
      setOpen(false)
      if (!onNewChat || !onSwitchProjectDir) return
      // 切目录失败 → 不新建：否则新会话会快照到旧目录，落在别的组
      const ok = await onSwitchProjectDir(path)
      if (!ok) return
      onNewChat()
    },
    [onNewChat, onSwitchProjectDir],
  )

  const handleNewChat = useCallback(() => {
    setOpen(false)
    onNewChat?.()
  }, [onNewChat])

  /** 项目中心：复用输入框项目 chip 的同一入口（ChatPanel 的 setDirOpen(true)），
   *  打开前先收起抽屉，避免抽屉叠在弹窗后面 */
  const handleOpenProjectDir = useCallback(() => {
    setOpen(false)
    setMenuOpen(false)
    onOpenProjectDir?.()
  }, [onOpenProjectDir])

  const saveRename = useCallback(
    async (id: string) => {
      const draft = draftTitle.trim()
      setEditingId(null)
      if (!draft) return
      try {
        await renameSession(id, draft)
        void refresh()
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
      }
    },
    [draftTitle, refresh, flashNotice],
  )

  // hover 音效节流：鼠标扫过列表时避免连续爆音（120ms 内只响一次）
  const hoverSoundAt = useRef(0)
  const playHoverSound = useCallback(() => {
    const now = Date.now()
    if (now - hoverSoundAt.current < 120) return
    hoverSoundAt.current = now
    playUiSound('session')
  }, [])

  /** 组头展开/收起（整组）：运行时状态，不持久化 */
  const toggleGroup = useCallback((key: string) => {
    setCollapsedGroups(m => ({ ...m, [key]: !m[key] }))
  }, [])

  /** 渲染单条会话行（组内复用：L/W/C 标识 + 标题 + 相对时间 + 行内重命名/归档） */
  const renderSessionItem = (it: ShelfSessionItem, dirAlreadyCurrent: boolean) => {
    const modeText =
      it.mode === 'workflow'
        ? t('input.mode.workflow')
        : it.mode === 'leader'
          ? t('input.mode.leader')
          : it.mode === 'custom'
            ? t('input.mode.custom')
            : ''
    return (
      <div
        key={it.id}
        className={`sr-item${it.is_active ? ' active' : ''}${
          editingId === it.id ? ' editing' : ''
        }`}
        onMouseEnter={() => {
          // hover 可切换项（非当前、非编辑态、未锁定）响轻音反馈
          if (!it.is_active && editingId !== it.id && !hardLocked) playHoverSound()
        }}
      >
        {editingId === it.id ? (
          <div className="sr-head">
            <input
              className="sr-rename-input"
              value={draftTitle}
              autoFocus
              maxLength={60}
              placeholder={it.title || t('sessionRail.untitled')}
              onChange={e => setDraftTitle(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Enter' && draftTitle.trim()) void saveRename(it.id)
                if (e.key === 'Escape') {
                  // 只退编辑、不收起抽屉（拦下冒泡给 document 的 Esc 收起监听）
                  e.stopPropagation()
                  setEditingId(null)
                }
              }}
            />
            <button
              type="button"
              className="sr-edit-btn"
              onClick={() => void saveRename(it.id)}
              disabled={!draftTitle.trim()}
              title={t('sessionRail.save')}
              aria-label={t('sessionRail.save')}
            >
              <IconCheck size={13} />
            </button>
            <button
              type="button"
              className="sr-edit-btn"
              onClick={() => setEditingId(null)}
              title={t('common.cancel')}
              aria-label={t('common.cancel')}
            >
              <IconX size={12} />
            </button>
          </div>
        ) : (
          <>
            <span
              className={`sr-mode-badge mode-${it.mode}`}
              title={modeText}
              aria-label={modeText}
            >
              {modeToLetter(it.mode)}
            </span>
            <button
              type="button"
              className={`sr-title-btn${it.is_active ? ' active' : ''}`}
              disabled={it.is_active || hardLocked}
              aria-current={it.is_active ? 'true' : undefined}
              title={it.title || t('sessionRail.untitled')}
              onClick={() =>
                void handleSwitch(
                  it.id,
                  it.is_active,
                  normalizePathKey(it.project_path) || null,
                  dirAlreadyCurrent,
                )
              }
            >
              {it.title || t('sessionRail.untitled')}
            </button>
            {it.is_active && (
              <span className="sr-current-badge" aria-hidden="true">
                {t('sessionRail.current')}
              </span>
            )}
            {/* 行尾相对时间：hover 时淡出让位给操作按钮，避免按钮挤动布局 */}
            <span className="sr-time">{relativeTime(it.updated_at, t)}</span>
            <span className="sr-actions">
              <button
                type="button"
                className="sr-edit-btn"
                onClick={() => {
                  setDraftTitle(it.title)
                  setEditingId(it.id)
                }}
                title={t('sessionRail.rename')}
                aria-label={t('sessionRail.rename')}
              >
                <IconEdit3 size={12} />
              </button>
              {!it.is_active && (
                <button
                  type="button"
                  className="sr-edit-btn sr-archive-btn"
                  onClick={() => setArchiveTarget({ kind: 'session', id: it.id })}
                  disabled={!canSwitch}
                  title={t('sessionRail.archive')}
                  aria-label={t('sessionRail.archive')}
                >
                  <IconTrash2 size={12} />
                </button>
              )}
            </span>
          </>
        )}
      </div>
    )
  }

  return (
    <>
      {/* ── 收起态色块：贴左缘的 8×58 竖条，右侧圆帽、实心 fg-5，hover 转 accent + 微光。
          点击展开/收起左侧抽屉，是收起态唯一可见元素（无 hover 感应唤出、无执行完成自动弹出）。
          执行中不可开合：点击只浮出「执行中不可切换」轻提示。 ── */}
      <button
        ref={chipRef}
        type="button"
        className="session-rail-chip"
        onClick={() => {
          // 执行中不可开合：给轻提示，不弹抽屉（后端 guard 之外再给个明确反馈）
          if (hardLocked) {
            flashChipHint()
            return
          }
          setOpen(o => !o)
        }}
        aria-expanded={open ? true : undefined}
        aria-disabled={hardLocked || undefined}
        tabIndex={open ? -1 : undefined}
        aria-label={t('sessionRail.title')}
        title={t('sessionRail.title')}
      />

      {/* 执行中点色块的轻提示：色块右侧浮出，3s 自动消失 */}
      {chipHint && (
        <div className="sr-chip-hint" role="status" aria-live="polite">
          <NoticeIcon tone="info" />
          <span>{t('sessionRail.busyHint')}</span>
        </div>
      )}

      {/* 点击面板外收起：透明承接层，不压暗消息流 */}
      <div
        className={`session-rail-scrim${open ? ' is-open' : ''}`}
        aria-hidden="true"
        onMouseDown={closeDrawer}
      />

      {/* ── 左侧滑动栏（抽屉）：默认平移出面板左缘外，点击色块后滑出。
          常驻挂载以保留滑动过渡；visibility 断开关闭态的 tab 焦点与命中测试。 ── */}
      <aside
        ref={panelRef}
        className={`session-rail-drawer${open ? ' is-open' : ''}`}
        role="navigation"
        aria-label={t('sessionRail.title')}
        aria-hidden={open ? undefined : true}
      >
        <div className="sr-drawer-head">
          <span className="sr-drawer-title">{t('sessionRail.projectsTitle')}</span>
          {/* 项目文件夹菜单：新建文件夹（项目中心）+ 已归档文件夹恢复入口 */}
          <div className="sr-menu-wrap" ref={menuRef}>
            <button
              type="button"
              className="sr-head-btn"
              onClick={() => setMenuOpen(o => !o)}
              title={t('sessionRail.folderMenu')}
              aria-label={t('sessionRail.folderMenu')}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
            >
              <IconMoreHorizontal size={14} />
            </button>
            {menuOpen && (
              <div className="sr-menu" role="menu">
                {onOpenProjectDir && (
                  <button
                    type="button"
                    className="sr-menu-item"
                    role="menuitem"
                    onClick={handleOpenProjectDir}
                  >
                    <IconFolder size={13} />
                    <span>{t('sessionRail.newProjectFolder')}</span>
                  </button>
                )}
                <div className="sr-menu-divider" />
                <div className="sr-menu-label">{t('sessionRail.archivedFolders')}</div>
                {archivedProjects.length === 0 ? (
                  <div className="sr-menu-empty">{t('sessionRail.archivedEmpty')}</div>
                ) : (
                  archivedProjects.map(p => (
                    <div key={p.path} className="sr-menu-row">
                      <span className="sr-menu-row-name" title={p.path}>
                        {p.name}
                      </span>
                      <button
                        type="button"
                        className="sr-menu-restore"
                        onClick={() => void handleRestoreProject(p.path)}
                        title={t('sessionRail.restoreFolder')}
                        aria-label={t('sessionRail.restoreFolder')}
                      >
                        <IconRestore size={12} />
                        <span>{t('sessionRail.restoreFolder')}</span>
                      </button>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>
          <button
            type="button"
            className="sr-head-btn"
            onClick={closeDrawer}
            title={t('sessionRail.collapse')}
            aria-label={t('sessionRail.collapse')}
          >
            <IconX size={14} />
          </button>
        </div>
        {/* 新会话：整行浅色实心按钮（与 Ctrl+N / TitleBar 同一逻辑源），落在当前工作目录下 */}
        {onNewChat && (
          <div className="sr-new-chat-wrap">
            <button
              type="button"
              className="sr-new-chat-btn"
              onClick={handleNewChat}
              disabled={hardLocked}
              title={t('sessionRail.newChat')}
            >
              <IconPlus size={15} />
              <span>{t('sessionRail.newChat')}</span>
            </button>
          </div>
        )}
        <div className="sr-list">
          {groups.length === 0 && (
            <div className="sr-group-empty sr-group-empty--top">
              {t('sessionRail.emptySessions')}
            </div>
          )}
          {groups.map(group => {
            const collapsed = !!collapsedGroups[group.key]
            const expanded = !!expandedGroups[group.key]
            const slice = visibleGroupSessions(group, groupLimit, expanded)
            const ungrouped = group.path === null
            return (
              <div className="sr-group" key={group.key || '__ungrouped__'}>
                <div className={`sr-group-head${group.isCurrent ? ' is-current' : ''}`}>
                  {editingProjectKey === group.key && group.path ? (
                    <div className="sr-head">
                      <input
                        className="sr-rename-input"
                        value={projectDraft}
                        autoFocus
                        maxLength={60}
                        placeholder={group.name}
                        onChange={e => setProjectDraft(e.target.value)}
                        onKeyDown={e => {
                          if (e.key === 'Enter' && projectDraft.trim()) {
                            void saveProjectRename(group)
                          }
                          if (e.key === 'Escape') {
                            // 只退编辑、不收起抽屉（拦下冒泡给 document 的 Esc 收起监听）
                            e.stopPropagation()
                            setEditingProjectKey(null)
                          }
                        }}
                      />
                      <button
                        type="button"
                        className="sr-edit-btn"
                        onClick={() => void saveProjectRename(group)}
                        disabled={!projectDraft.trim()}
                        title={t('sessionRail.save')}
                        aria-label={t('sessionRail.save')}
                      >
                        <IconCheck size={13} />
                      </button>
                      <button
                        type="button"
                        className="sr-edit-btn"
                        onClick={() => setEditingProjectKey(null)}
                        title={t('common.cancel')}
                        aria-label={t('common.cancel')}
                      >
                        <IconX size={12} />
                      </button>
                    </div>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="sr-group-toggle"
                        onClick={() => toggleGroup(group.key)}
                        aria-expanded={!collapsed}
                        title={group.path ?? t('sessionRail.ungrouped')}
                      >
                        {collapsed ? <IconChevronRight size={13} /> : <IconChevronDown size={13} />}
                        <span className={`sr-group-name${ungrouped ? ' is-ungrouped' : ''}`}>
                          {group.name || t('sessionRail.ungrouped')}
                        </span>
                        {group.isCurrent && (
                          <span className="sr-group-badge">{t('sessionRail.current')}</span>
                        )}
                        {group.auto && (
                          <span className="sr-group-tag" title={t('sessionRail.autoGroupHint')}>
                            {t('sessionRail.autoTag')}
                          </span>
                        )}
                      </button>
                      <span className="sr-group-count" aria-hidden="true">
                        {group.sessions.length}
                      </span>
                      <span className="sr-group-actions">
                        {/* 组内新建对话：先切该文件夹再新建（无路径的「未分组」组不提供） */}
                        {group.path && onNewChat && onSwitchProjectDir && (
                          <button
                            type="button"
                            className="sr-edit-btn"
                            onClick={() => void handleNewChatInGroup(group.path as string)}
                            disabled={hardLocked}
                            title={t('sessionRail.newChatInFolder')}
                            aria-label={t('sessionRail.newChatInFolder')}
                          >
                            <IconPlus size={12} />
                          </button>
                        )}
                        {/* auto 只读组不提供重命名 / 归档（避免为未收藏目录写入书签） */}
                        {group.path && !group.auto && (
                          <>
                            <button
                              type="button"
                              className="sr-edit-btn"
                              onClick={() => {
                                setProjectDraft(group.name)
                                setEditingProjectKey(group.key)
                              }}
                              title={t('sessionRail.renameFolder')}
                              aria-label={t('sessionRail.renameFolder')}
                            >
                              <IconEdit3 size={12} />
                            </button>
                            <button
                              type="button"
                              className="sr-edit-btn sr-archive-btn"
                              onClick={() =>
                                setArchiveTarget({
                                  kind: 'project',
                                  path: group.path as string,
                                  name: group.name,
                                })
                              }
                              disabled={!canSwitch}
                              title={t('sessionRail.archiveFolder')}
                              aria-label={t('sessionRail.archiveFolder')}
                            >
                              <IconTrash2 size={12} />
                            </button>
                          </>
                        )}
                      </span>
                    </>
                  )}
                </div>
                {!collapsed && (
                  <div className="sr-group-body">
                    {slice.sessions.map(it => renderSessionItem(it, group.isCurrent))}
                    {/* 折叠行：默认只列全局上限条，其余点开后展开（每组独立） */}
                    {slice.hiddenCount > 0 && (
                      <button
                        type="button"
                        className="sr-more-btn"
                        onClick={() => setExpandedGroups(m => ({ ...m, [group.key]: true }))}
                      >
                        {t('sessionRail.expandMore', String(slice.hiddenCount))}
                      </button>
                    )}
                    {/* 空文件夹仍显示该组（书签是用户主动维护的集合），组内给一行弱提示 */}
                    {group.sessions.length === 0 && (
                      <div className="sr-group-empty">{t('sessionRail.groupEmpty')}</div>
                    )}
                  </div>
                )}
              </div>
            )
          })}
        </div>

        {notice && (
          <div
            className={`sr-notice sr-notice--${notice.tone}`}
            role={notice.tone === 'error' ? 'alert' : 'status'}
            aria-live={notice.tone === 'error' ? 'assertive' : 'polite'}
          >
            <NoticeIcon tone={notice.tone} />
            <span>{notice.text}</span>
          </div>
        )}
      </aside>

      {archiveTarget &&
        createPortal(
          <CompactModal
            open
            onClose={() => setArchiveTarget(null)}
            title={
              archiveTarget.kind === 'session'
                ? t('sessionRail.archiveConfirmTitle')
                : t('sessionRail.archiveFolderConfirmTitle')
            }
            size="sm"
            className="compact-modal--fit"
            footer={
              <>
                <button
                  type="button"
                  className="btn btn-ghost"
                  onClick={() => setArchiveTarget(null)}
                >
                  {t('common.cancel')}
                </button>
                <button type="button" className="btn btn-danger" onClick={confirmArchive}>
                  {t('sessionRail.archive')}
                </button>
              </>
            }
          >
            <div className="sr-confirm-desc">
              {archiveTarget.kind === 'session'
                ? t('sessionRail.archiveConfirmDesc')
                : t('sessionRail.archiveFolderConfirmDesc', archiveTarget.name)}
            </div>
          </CompactModal>,
          document.body,
        )}
    </>
  )
}
