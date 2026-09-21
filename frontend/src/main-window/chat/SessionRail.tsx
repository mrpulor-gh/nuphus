import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { RefObject } from 'react'
import { createPortal } from 'react-dom'
// `open` 是系统目录选择器；别名避免与抽屉开合态 `open` 同名遮蔽
import { open as openDirDialog } from '@tauri-apps/plugin-dialog'
import {
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconEdit3,
  IconFolder,
  IconFolderPlus,
  IconMoreHorizontal,
  IconPlus,
  IconTrash2,
  IconX,
} from '../../ui/Icons'
import { playUiSound } from '../../ui/sound'
import { CompactModal } from '../layout/CompactModal'
import { NewChatModal, type NewChatProjectOption } from './NewChatModal'
import { CreateProjectModal } from './CreateProjectModal'
import { useLanguage } from '../../locales'
import {
  listShelfSessions,
  switchSession,
  renameSession,
  archiveSession,
  setProjectBookmarks,
  setProjectFolderArchived,
  setSessionSortPrefs,
  createProjectChat,
  SESSION_GROUP_LIMIT_CHANGED_EVENT,
  type ProjectBookmark,
  type ShelfProjectEntry,
  type ShelfSessionItem,
} from '../lib/api'
import {
  DEFAULT_GROUP_LIMIT,
  DEFAULT_SESSION_SORT_PREFS,
  buildSessionGroups,
  normalizeGroupLimit,
  normalizePathKey,
  normalizeSessionSortPrefs,
  visibleGroupSessions,
  type SessionGroup,
  type SessionSortPrefs,
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

/** 路径末段名（新建书签的默认名；与后端 `utils::dir_display_name` 同为「路径末段」规则） */
function projectNameFromPath(p: string): string {
  return (
    p
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() || p
  )
}

/**
 * 书签整表构建（`set_project_bookmarks` 是**整表替换**，不是增量）：未归档书签按原序 →
 * 本次目标路径 → 已归档书签原样带回（否则归档记录被抹掉、「恢复隐藏项目」入口丢失）。
 *
 * 同路径已存在时**不产生重复项**：沿用原位置，名称取本次提交值；若该路径原本已归档，
 * 此次同时**解除归档**——用户此刻明确要把这个文件夹当项目用，否则新建的组会被隐藏、
 * 看不到刚生成的那条对话。（auto 只读组不写回书签表：它在 `projects[]` 里带 `auto`。）
 *
 * 判重与后端同口径：这里按 `normalizePathKey`（Windows 大小写不敏感）；后端
 * `set_project_bookmarks` 再做一次按路径去重兜底（config/preferences.rs:445-447）。
 */
function buildBookmarkTable(
  projects: readonly ShelfProjectEntry[],
  archivedProjects: readonly ShelfProjectEntry[],
  name: string,
  path: string,
): ProjectBookmark[] {
  const key = normalizePathKey(path)
  const table: ProjectBookmark[] = projects
    .filter(p => !p.auto)
    .map(p => ({ name: p.name, path: p.path }))
  const existing = table.find(b => normalizePathKey(b.path) === key)
  if (existing) existing.name = name
  else table.push({ name, path })
  for (const a of archivedProjects) {
    if (normalizePathKey(a.path) === key) continue // 同路径：本次解除归档，不重复带回
    table.push({ name: a.name, path: a.path, archived: true })
  }
  return table
}

interface SessionRailProps {
  /** 切换成功后由父级重拉 get_chat_history 整体替换气泡 */
  onSessionChanged: () => void
  /**
   * 新建对话（复用桌面统一入口 handleNewChat / Ctrl+N 同一逻辑源；执行中禁用）。
   * 可带弹窗填写的标题——后端只记录（会话仍在首条消息那一刻诞生）；
   * 返回 false = 后端拒绝，调用方据此保持弹窗打开。
   */
  onNewChat?: (title?: string) => Promise<boolean>
  /**
   * 切换工作目录（**复用** ChatPanel.switchProject 单一实现：落盘 + 后端向活跃槽注入
   * 变更提醒 + HUD 反馈）；返回 true = 已切到目标目录。
   * 组内「新建对话」/ 点击组内会话 / 📁+ 创建项目都依赖此入口，不另起一套切目录逻辑。
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
  if (code === 'restoreFailGeneric') return 'sessionRail.restoreFailGeneric'
  if (code === 'sortPrefsFailGeneric') return 'sessionRail.sortPrefsFailGeneric'
  if (code === 'newChatSwitchFail') return 'sessionRail.newChatSwitchFail'
  if (code === 'browseDirFail') return 'sessionRail.newChatBrowseFail'
  if (code === 'no_project_dir') return 'sessionRail.createProjectFail'
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

/** 「项目」行 ⋯ 菜单当前展开的子菜单（排序条件里再套一级 `time`） */
type ProjectMenuPane = 'arrange' | 'sort' | 'time' | 'restore'

interface ProjectLabelRowProps {
  /** 当前排序偏好（✓ 选中态依据；读数来自后端 `sort_prefs`） */
  prefs: SessionSortPrefs
  /** 已归档文件夹：`恢复隐藏项目 (N)` 的子菜单数据源 */
  archivedProjects: readonly ShelfProjectEntry[]
  /** 整理侧边栏 · 全部展开（清空整组折叠表） */
  onExpandAll: () => void
  /** 整理侧边栏 · 全部关闭（把当前每个可见组标记为收起） */
  onCollapseAll: () => void
  /** 选择排序偏好（宿主负责立即重排 + 落盘 + 失败回滚） */
  onSelectSort: (prefs: SessionSortPrefs) => void
  /** 恢复某个已归档文件夹（归档标记置回 false） */
  onRestore: (path: string) => void
  /** 新建项目文件夹（📁+）：打开「创建项目」弹窗（选目录 + 命名 → 书签 + 切目录 + 空对话） */
  onCreateProject?: () => void
  /** 📁+ 按钮句柄：关弹窗后把焦点还回去（与「新建对话」弹窗同一纪律） */
  plusBtnRef?: RefObject<HTMLButtonElement>
}

/**
 * 「项目」标签行 + 右端两个图标：`⋯`（菜单）在前、`📁+`（新建项目文件夹）在后。
 *
 * ⋯ 菜单三项（ZPY 终审设计）：
 * 1. 整理侧边栏 › ：全部展开 / 全部关闭（操作所有分组的折叠态；运行时状态，不持久化）；
 * 2. 排序条件 › ：按项目 / 近期项目 / 按时间顺序 ›（创建时间 / 更新时间）。
 *    **两个维度独立**（组序维度 & 组内键），当前项前有 ✓（`menuitemradio` + `aria-checked`）；
 * 3. 恢复隐藏项目 (N) › ：子菜单直接列出可恢复的归档文件夹，逐项恢复——保持菜单打开
 *    以便连续恢复（N = 已归档文件夹数量；N = 0 时该项禁用，不再给空子菜单）。
 *
 * 交互契约：Esc 关闭（先收子菜单，`stopPropagation` 拦下抽屉的 document 级 Esc，
 * 不连带收起抽屉）、点击外部关闭、打开时焦点落到首个菜单项（键盘可达：Tab 遍历 +
 * Enter/Space 触发；子菜单项随父项展开后进入 Tab 序）。
 */
function ProjectLabelRow({
  prefs,
  archivedProjects,
  onExpandAll,
  onCollapseAll,
  onSelectSort,
  onRestore,
  onCreateProject,
  plusBtnRef,
}: ProjectLabelRowProps) {
  const { t } = useLanguage()
  const [open, setOpen] = useState(false)
  const [pane, setPane] = useState<ProjectMenuPane | null>(null)
  const rowRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const firstItemRef = useRef<HTMLButtonElement>(null)

  const close = useCallback(() => {
    setOpen(false)
    setPane(null)
  }, [])

  // 打开后焦点落到首个菜单项：键盘用户不必先 Tab 穿过整个会话列表
  useEffect(() => {
    if (open) firstItemRef.current?.focus()
  }, [open])

  // 点击外部关闭：菜单在抽屉内，抽屉自己的「面板外点击」不覆盖「菜单外点击」
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node | null
      if (!target) return
      if (rowRef.current?.contains(target)) return
      close()
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [open, close])

  /**
   * Esc：逐层收（按时间顺序 → 排序条件 → 主菜单），最后交还焦点。
   * `stopPropagation` 拦下抽屉的 document 级 Esc —— 收菜单不连带收起抽屉。
   */
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== 'Escape') return
    e.stopPropagation()
    if (pane === 'time') {
      setPane('sort')
      return
    }
    if (pane) {
      setPane(null)
      return
    }
    close()
    triggerRef.current?.focus()
  }

  /**
   * 子菜单开关：hover 进入即展开；点同一项再点一次收起。
   * 收起「按时间顺序」只退到父级「排序条件」（不是一把收掉整个菜单）。
   */
  const openPane = useCallback((next: ProjectMenuPane) => {
    setPane(prev => (prev === next ? (next === 'time' ? 'sort' : null) : next))
  }, [])

  /** 单选菜单项（排序两个维度的共同形态）：✓ 槽常驻占位，避免选中态左右跳动 */
  const radioItem = (checked: boolean, label: string, onPick: () => void) => (
    <button
      type="button"
      className="sr-menu-item"
      role="menuitemradio"
      aria-checked={checked}
      onClick={onPick}
    >
      <span className="sr-menu-check" aria-hidden="true">
        {checked ? <IconCheck size={12} /> : null}
      </span>
      <span className="sr-menu-text">{label}</span>
    </button>
  )

  /**
   * 各子菜单展开态：在 JSX 收窄之外算好（同一表达式写在收窄作用域里会被 TS 判为
   * 「不可能的比较」，如 `sort` 分支内再比 `time`）。
   *
   * ⚠️ `sort` 取「自身或其后代子菜单打开」：第三层「按时间顺序」是嵌套在「排序条件」
   * 卡片里的，若父级卡片不渲染，子菜单也会随之消失。
   */
  const expandedPanes = {
    arrange: pane === 'arrange',
    sort: pane === 'sort' || pane === 'time',
    time: pane === 'time',
    restore: pane === 'restore',
  }

  /** 带子菜单的菜单项（aria-haspopup / aria-expanded 表达层级） */
  const submenuItem = (
    target: ProjectMenuPane,
    label: string,
    opts: { first?: boolean; disabled?: boolean } = {},
  ) => (
    <button
      type="button"
      ref={opts.first ? firstItemRef : undefined}
      className="sr-menu-item"
      role="menuitem"
      aria-haspopup="menu"
      aria-expanded={expandedPanes[target]}
      disabled={opts.disabled}
      onClick={() => openPane(target)}
      onMouseEnter={() => setPane(target)}
    >
      <span className="sr-menu-text">{label}</span>
      <IconChevronRight size={12} className="sr-menu-arrow" />
    </button>
  )

  return (
    <div className="sr-list-label" ref={rowRef} onKeyDown={onKeyDown}>
      <span className="sr-list-label-text">{t('sessionRail.projectsTitle')}</span>
      <span className="sr-list-actions">
        <button
          ref={triggerRef}
          type="button"
          className="sr-icon-btn"
          aria-label={t('sessionRail.projectsMenu')}
          title={t('sessionRail.projectsMenu')}
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={() => (open ? close() : setOpen(true))}
        >
          <IconMoreHorizontal size={14} />
        </button>
        {onCreateProject && (
          <button
            ref={plusBtnRef}
            type="button"
            className="sr-icon-btn"
            aria-label={t('sessionRail.newProjectFolder')}
            title={t('sessionRail.newProjectFolder')}
            onClick={() => {
              close()
              onCreateProject()
            }}
          >
            <IconFolderPlus size={14} />
          </button>
        )}

        {open && (
          <div className="sr-menu" role="menu" aria-label={t('sessionRail.projectsMenu')}>
            <div className="sr-menu-group">
              {submenuItem('arrange', t('sessionRail.menuArrange'), { first: true })}
              {expandedPanes.arrange && (
                <div
                  className="sr-menu sr-submenu"
                  role="menu"
                  aria-label={t('sessionRail.menuArrange')}
                >
                  <button
                    type="button"
                    className="sr-menu-item"
                    role="menuitem"
                    onClick={() => {
                      onExpandAll()
                      close()
                    }}
                  >
                    <span className="sr-menu-text">{t('sessionRail.menuExpandAll')}</span>
                  </button>
                  <button
                    type="button"
                    className="sr-menu-item"
                    role="menuitem"
                    onClick={() => {
                      onCollapseAll()
                      close()
                    }}
                  >
                    <span className="sr-menu-text">{t('sessionRail.menuCollapseAll')}</span>
                  </button>
                </div>
              )}
            </div>

            <div className="sr-menu-group">
              {submenuItem('sort', t('sessionRail.menuSort'))}
              {expandedPanes.sort && (
                <div
                  className="sr-menu sr-submenu"
                  role="menu"
                  aria-label={t('sessionRail.menuSort')}
                >
                  {radioItem(
                    prefs.groupOrder === 'bookmark',
                    t('sessionRail.menuSortByProject'),
                    () => {
                      onSelectSort({ ...prefs, groupOrder: 'bookmark' })
                      close()
                    },
                  )}
                  {radioItem(
                    prefs.groupOrder === 'recent',
                    t('sessionRail.menuSortByRecent'),
                    () => {
                      onSelectSort({ ...prefs, groupOrder: 'recent' })
                      close()
                    },
                  )}
                  {/* 按时间顺序：只管组内顺序，不影响组序（独立维度的第三层子菜单） */}
                  <div className="sr-menu-group">
                    {submenuItem('time', t('sessionRail.menuSortByTime'))}
                    {expandedPanes.time && (
                      <div
                        className="sr-menu sr-submenu"
                        role="menu"
                        aria-label={t('sessionRail.menuSortByTime')}
                      >
                        {radioItem(
                          prefs.sortKey === 'created',
                          t('sessionRail.menuSortByCreated'),
                          () => {
                            onSelectSort({ ...prefs, sortKey: 'created' })
                            close()
                          },
                        )}
                        {radioItem(
                          prefs.sortKey === 'updated',
                          t('sessionRail.menuSortByUpdated'),
                          () => {
                            onSelectSort({ ...prefs, sortKey: 'updated' })
                            close()
                          },
                        )}
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>

            <div className="sr-menu-group">
              {submenuItem(
                'restore',
                t('sessionRail.menuRestore', String(archivedProjects.length)),
                { disabled: archivedProjects.length === 0 },
              )}
              {expandedPanes.restore && archivedProjects.length > 0 && (
                <div
                  className="sr-menu sr-submenu"
                  role="menu"
                  aria-label={t('sessionRail.menuRestore', String(archivedProjects.length))}
                >
                  {archivedProjects.map(p => (
                    <button
                      key={p.path}
                      type="button"
                      className="sr-menu-item"
                      role="menuitem"
                      title={p.path}
                      onClick={() => onRestore(p.path)}
                    >
                      <IconFolder size={12} className="sr-menu-icon" />
                      <span className="sr-menu-text">{p.name}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}
      </span>
    </div>
  )
}

/**
 * 会话工作台（Session Rail）：聊天面板左缘的滑动抽屉。
 * - 收起态：左缘常驻一枚色块（会话图标 + 当前会话 mode 首字母），是唯一可见元素。
 * - 展开态：点击色块 → 左侧列表滑出（**按项目文件夹分组**渲染：组头=文件夹名 +
 *   展开/折叠 + 组内新建/重命名/归档，组内会话沿用小胶囊行样式）。
 * - 开合入口只有三个：色块点击、面板外点击、Esc；**不做 hover 感应唤出，
 *   执行完成也不自动弹出**（2026-09-15 大王反馈：隐藏式选择看不到会话标题）。
 * - 抽屉头部只有「会话工作台」标题（无任何按钮）；文件夹管理入口收敛到「项目」标签行右端
 *   的两个图标：`⋯`（整理侧边栏 / 排序条件 / 恢复隐藏项目）与 `📁+`（新建项目文件夹 →
 *   打开「创建项目」弹窗：选目录 + 命名 → 写书签 + 切当前目录 + 立刻生成一条空对话）。
 * - 「创建项目」弹窗确认后，rail 里该文件夹下立刻出现 1 条**当前对话**（草稿，纯内存态：
 *   未发消息就切换会话/退出进程即消失，发出首条消息才走既有诞生路径落库）。
 * - 数据源与切换逻辑完全沿用：list_shelf_sessions（5s 轮询 + 可见性刷新），
 *   分组完全来自返回体的 items/projects/archived_projects/collapsed_limit/sort_prefs
 *   （**不推断归属**：project_path=null 者进「未分组」兜底组；归档文件夹整组隐藏）。
 * - busy / 追加队列非空时后端拒绝 → 错误码映射文案在抽屉底部短暂浮现。
 */
export default function SessionRail({
  onSessionChanged,
  onNewChat,
  onSwitchProjectDir,
  onModeSwitched,
  locked = false,
  mood,
}: SessionRailProps) {
  const { t } = useLanguage()
  const [items, setItems] = useState<ShelfSessionItem[]>([])
  /** 可见项目文件夹（组顺序 = 此数组顺序：书签序 → auto） */
  const [projects, setProjects] = useState<ShelfProjectEntry[]>([])
  /** 已归档项目文件夹（整组隐藏；恢复入口 = 「项目」行 ⋯ → 恢复隐藏项目） */
  const [archivedProjects, setArchivedProjects] = useState<ShelfProjectEntry[]>([])
  /** 全局组内折叠上限（后端 collapsed_limit；设置中心可改，事件即时生效） */
  const [groupLimit, setGroupLimit] = useState(DEFAULT_GROUP_LIMIT)
  /**
   * 排序偏好（组序维度 + 组内键）：**唯一权威是后端 `sort_prefs`**（落 preferences，
   * 重启保持）；本地只在选择瞬间乐观更新，成功后以后端返回的归一值为准。
   * 用「值相等则复用旧对象」避免每次轮询都触发重绘。
   */
  const [sortPrefs, setSortPrefs] = useState<SessionSortPrefs>(DEFAULT_SESSION_SORT_PREFS)
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
  /** 新建对话弹窗开合（入口 = 列表首位动作行；会话标题 + 归属项目确认后创建） */
  const [newChatOpen, setNewChatOpen] = useState(false)
  /** 「创建项目」弹窗开合（入口 = 「项目」行右端 📁+；不再进项目中心） */
  const [createProjectOpen, setCreateProjectOpen] = useState(false)
  /** 抽屉开合态：默认收起（只露色块），点击色块才伸出 */
  const [open, setOpen] = useState(false)
  /** 组头折叠态（key → true=收起整组）：默认全部展开，运行时状态不持久化 */
  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({})
  /** 组内「展开其余 N 个会话」态（key → true=全显）：默认按全局上限折叠 */
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({})
  /** 执行中点色块的轻提示（色块旁浮出，数秒后自动消失） */
  const [chipHint, setChipHint] = useState(false)
  const chipHintTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const chipRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLElement>(null)
  /** 新建对话入口动作行：弹窗关窗后焦点回到这里 */
  const newChatRowRef = useRef<HTMLButtonElement>(null)
  /** 「项目」行右端 📁+：创建项目弹窗关窗后焦点回到这里 */
  const plusBtnRef = useRef<HTMLButtonElement>(null)
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
      if (panelRef.current?.contains(target)) return
      if (chipRef.current?.contains(target)) return
      // 弹窗（CompactModal portal 到 body）是**独立层级**：点弹窗内部不算「点抽屉外」。
      // 否则在新建对话弹窗里选项目会顺手收起抽屉，关窗后入口行与焦点都不在场。
      if (target instanceof Element && target.closest('.compact-overlay')) return
      setOpen(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onDown)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onDown)
    }
  }, [open, editingId, editingProjectKey])

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
        // 排序偏好同款处理：后端是唯一权威，读数变化立即重排；值未变则复用旧对象不重绘
        const nextPrefs = normalizeSessionSortPrefs(r.sort_prefs)
        setSortPrefs(prev =>
          prev.groupOrder === nextPrefs.groupOrder && prev.sortKey === nextPrefs.sortKey
            ? prev
            : nextPrefs,
        )
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

  /**
   * 分组视图：全部由返回体字段派生（组序维度 / 组内排序键 / 归档隐藏 / 未分组末位）。
   * 排序语义**只在 sessionGroups 纯函数里**（移动端 NavBar 复用同一实现），组件不重算。
   */
  const groups = useMemo(
    () => buildSessionGroups(items, projects, archivedProjects, sortPrefs),
    [items, projects, archivedProjects, sortPrefs],
  )

  /**
   * 整理侧边栏 · 全部展开：清空整组折叠表。
   * 只动「整组收起」这一层；组内「展开其余 N 个会话」按各自状态保留（不越权代管）。
   */
  const expandAllGroups = useCallback(() => setCollapsedGroups({}), [])

  /** 整理侧边栏 · 全部关闭：把**当前可见的每个组**（含未分组兜底组）标记为收起 */
  const collapseAllGroups = useCallback(() => {
    setCollapsedGroups(Object.fromEntries(groups.map(g => [g.key, true])))
  }, [groups])

  /**
   * 切换排序偏好：乐观更新（列表立即按新规则重排）→ 落盘 → 以后端返回的归一值为准。
   * 失败回滚到切换前并给出可感知提示（沿用既有 flashNotice 通道）。
   */
  const handleSelectSort = useCallback(
    async (next: SessionSortPrefs) => {
      const prev = sortPrefs
      setSortPrefs(next)
      try {
        const applied = await setSessionSortPrefs(next.groupOrder, next.sortKey)
        setSortPrefs(normalizeSessionSortPrefs(applied))
        void refresh()
      } catch {
        setSortPrefs(prev)
        flashNotice('sortPrefsFailGeneric')
      }
    },
    [sortPrefs, refresh, flashNotice],
  )

  /**
   * 恢复已归档文件夹：「项目」行 ⋯ → 恢复隐藏项目 → 点某一项。
   * 归档标记置回 false → 组重新出现在列表；菜单保持打开，可连续恢复多个。
   */
  const handleRestoreProject = useCallback(
    async (path: string) => {
      try {
        await setProjectFolderArchived(path, false)
        void refresh()
      } catch {
        flashNotice('restoreFailGeneric')
      }
    },
    [refresh, flashNotice],
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

  /** 组头「+」：先切到该文件夹，再走与 Ctrl+N 同一新建入口（新会话归属即该文件夹）。
   *  无标题：该入口不经弹窗，会话仍按既有派生标题语义命名。 */
  const handleNewChatInGroup = useCallback(
    async (path: string) => {
      setOpen(false)
      if (!onNewChat || !onSwitchProjectDir) return
      // 切目录失败 → 不新建：否则新会话会快照到旧目录，落在别的组
      const ok = await onSwitchProjectDir(path)
      if (!ok) return
      void onNewChat()
    },
    [onNewChat, onSwitchProjectDir],
  )

  /** 入口动作行「新建对话」：打开弹窗（列表首位 —— 创建路径显式存在，但不把创建混进项目目录） */
  const openNewChatModal = useCallback(() => setNewChatOpen(true), [])

  /**
   * 关窗（取消 / Esc / 点遮罩三条路径统一入口）：焦点还给入口动作行。
   * 抽屉保持展开——关窗后用户还要继续在列表里操作，入口行本身也在抽屉里。
   */
  const closeNewChatModal = useCallback(() => {
    setNewChatOpen(false)
    newChatRowRef.current?.focus()
  }, [])

  /**
   * 「浏览本地目录…」：系统目录选择器 → 取回的目录不在 projects[] 时**先追加书签**
   * （即新建一个项目分组，落盘后由 refresh 读回），再按该目录归属。
   *
   * 书签表是整表替换：提交时必须带回已归档书签（否则归档记录被抹掉、恢复入口丢失），
   * auto 只读组不写回书签表——与文件夹重命名同一规则。
   */
  const handleBrowseNewChatDir = useCallback(async (): Promise<NewChatProjectOption | null> => {
    let picked: string | null = null
    try {
      const dir = await openDirDialog({
        directory: true,
        multiple: false,
        title: t('sessionRail.newChatProjectLabel'),
      })
      picked = typeof dir === 'string' && dir ? dir : null
    } catch {
      // 选择器不可用（无桌面环境 / 权限）→ 明确提示，不静默失败
      flashNotice('browseDirFail')
      return null
    }
    if (!picked) return null // 用户取消：不是错误，不提示
    const known = projects.find(p => normalizePathKey(p.path) === normalizePathKey(picked))
    if (known) return { name: known.name, path: known.path }
    try {
      const saved = await setProjectBookmarks([
        // 书签组（auto=false，顺序即书签顺序）；auto 只读组不写回书签表
        ...projects.filter(p => !p.auto).map(p => ({ name: p.name, path: p.path })),
        { name: projectNameFromPath(picked), path: picked },
        // 归档书签原样回填（否则整表替换会丢掉归档记录）
        ...archivedProjects.map(p => ({ name: p.name, path: p.path, archived: true })),
      ])
      void refresh()
      // 名称以落库返回值为准（后端有去空/去重/名称兜底规则）
      const entry = saved.find(b => normalizePathKey(b.path) === normalizePathKey(picked))
      return entry
        ? { name: entry.name, path: entry.path }
        : { name: projectNameFromPath(picked), path: picked }
    } catch (e) {
      flashNotice(typeof e === 'string' ? e : String(e))
      return null
    }
  }, [projects, archivedProjects, refresh, flashNotice, t])

  /**
   * 弹窗「创建对话」。顺序不可乱：
   * ① 先切目录——复用 ChatPanel.switchProject 单一实现（落盘 + 状态同步 + HUD 反馈）；
   *    失败即中止并提示，**不新建**：新会话的归属在诞生时快照当前目录，先建后切会落错组；
   * ② 再走与 Ctrl+N / TitleBar / 组头「+」同一新建入口（后端 `new_chat_session_cmd`），
   *    把弹窗标题一并交给它；
   * ③ 刷新列表（当前目录 chip / is_current 高亮同步）。
   *
   * ⚠️ 会话**不在此时创建**（这是刻意的产品语义）：`new_chat_session_cmd` 只把当前槽置回
   * 欢迎页 + **记录标题**，真实会话仍在欢迎页直发首条消息那一刻诞生；后端在诞生点把记录的
   * 标题写成该会话的标题（展示台覆盖表 + sessions.summary），于是首条消息发完，会话卡带着
   * 这个标题落到所选项目分组首位。此前「确认即出卡」需要后端凭空造一条空会话并占槽，
   * 牵动 archive_active 跳过空会话等既有语义——不做。
   *
   * 失败处理：① 失败 → 弹窗保持打开（用户可改选项目重试）；② 失败（执行中/追加队列非空）
   * → 同样保持打开，后端已由 HUD 给出原因，不另起一套提示。
   */
  const handleCreateNewChat = useCallback(
    async (title: string, project: NewChatProjectOption): Promise<boolean> => {
      if (!onNewChat || !onSwitchProjectDir) return false
      const ok = await onSwitchProjectDir(project.path)
      if (!ok) {
        // 切目录失败已由 HUD 反馈；工作台内再给一条提示，避免用户以为会话已创建。
        // 弹窗**保持打开**（返回 false）：用户可改选项目后重试，不必重新填标题。
        flashNotice('newChatSwitchFail')
        return false
      }
      const accepted = await onNewChat(title)
      if (!accepted) return false
      setNewChatOpen(false)
      setOpen(false)
      void refresh()
      return true
    },
    [onNewChat, onSwitchProjectDir, refresh, flashNotice],
  )

  /** 「项目」行 📁+：打开「创建项目」弹窗（**不再进项目中心**——本次改动后的唯一语义） */
  const openCreateProjectModal = useCallback(() => setCreateProjectOpen(true), [])

  /**
   * 关窗（取消 / Esc / 点遮罩 / 右上角 ✕ 四条路径统一入口）：焦点还给 📁+。
   * 抽屉保持展开——关窗后用户还要继续在列表里操作，入口按钮本身也在抽屉里。
   */
  const closeCreateProjectModal = useCallback(() => {
    setCreateProjectOpen(false)
    plusBtnRef.current?.focus()
  }, [])

  /**
   * 「选择项目文件夹」：系统目录选择器（与「新建对话 → 浏览本地目录…」同款调用：
   * `@tauri-apps/plugin-dialog` 的 `open({ directory: true })`）。返回 null = 用户取消
   * （不是错误，不提示）或选择器不可用（已提示）。
   */
  const handlePickProjectDir = useCallback(async (): Promise<string | null> => {
    try {
      const dir = await openDirDialog({
        directory: true,
        multiple: false,
        title: t('sessionRail.createProjectTitle'),
      })
      return typeof dir === 'string' && dir ? dir : null
    } catch {
      flashNotice('browseDirFail')
      return null
    }
  }, [flashNotice, t])

  /**
   * 「创建项目」弹窗确认。顺序不可乱（与「创建对话」同一纪律）：
   * ① 写书签——书签即 rail 的分组来源，新文件夹因此立刻成组（同路径已有书签时不产生
   *    重复项：只更新名称并解除归档，见 [`buildBookmarkTable`]）；
   * ② 切当前项目目录——**必须早于**生成空对话：归属是诞生时的目录快照，先建后切会落错组；
   *    失败即中止（弹窗保持打开，用户可改路径重试）；
   * ③ `create_project_chat`——生成一条**内存态**空对话并成为当前对话（不落库）。
   * ④ 关弹窗 + 收起抽屉 + 刷新 rail（分组 + 该对话的当前高亮）+ 重拉聊天区（空态可立刻开说）。
   */
  const handleCreateProject = useCallback(
    async (name: string, path: string): Promise<boolean> => {
      if (!onSwitchProjectDir) return false
      try {
        await setProjectBookmarks(buildBookmarkTable(projects, archivedProjects, name, path))
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
        return false
      }
      const ok = await onSwitchProjectDir(path)
      if (!ok) {
        // 切目录失败已由 HUD 反馈；这里再给一条，避免用户以为项目已创建
        flashNotice('newChatSwitchFail')
        return false
      }
      try {
        await createProjectChat()
      } catch (e) {
        flashNotice(typeof e === 'string' ? e : String(e))
        return false
      }
      setCreateProjectOpen(false)
      setOpen(false)
      void refresh()
      // 当前对话换成刚生成的草稿 → 聊天区回空态（后端无历史，欢迎页可直接开说）
      onSessionChanged()
      return true
    },
    [projects, archivedProjects, onSwitchProjectDir, refresh, flashNotice, onSessionChanged],
  )

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
    // 草稿对话（新建项目文件夹后尚未开说的那条）：标题文案固定为「新建对话」，
    // 且不提供行内重命名/归档——重命名会写 sessions 行，违反「草稿不落库」。
    const titleText = it.draft ? t('sessionRail.newChat') : it.title || t('sessionRail.untitled')
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
              title={titleText}
              onClick={() =>
                void handleSwitch(
                  it.id,
                  it.is_active,
                  normalizePathKey(it.project_path) || null,
                  dirAlreadyCurrent,
                )
              }
            >
              {titleText}
            </button>
            {it.is_active && (
              <span className="sr-current-badge" aria-hidden="true">
                {t('sessionRail.current')}
              </span>
            )}
            {/* 行尾相对时间：hover 时淡出让位给操作按钮，避免按钮挤动布局 */}
            <span className="sr-time">{relativeTime(it.updated_at, t)}</span>
            {/* 草稿对话无行内操作：重命名会写 sessions 行（违反不落库），归档对内存态无意义 */}
            {!it.draft && (
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
            )}
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
          {/* 头部只有标题：文件夹管理入口已全部迁至项目中心，收起走 Esc / 面板外点击 / 再点色块 */}
          <span className="sr-drawer-title">{t('sessionRail.title')}</span>
        </div>
        {/* 新建对话入口 = 列表首位的**动作行**：复用会话行骨架（文字左缘与会话标题对齐、
            右端 + 号），虚线描边 + 弱文字把「动作」与上方「数据」区分开。
            不放面板右上角 —— 那里已定稿为「每模块唯一关闭按钮」，不新增按钮。 */}
        {onNewChat && onSwitchProjectDir && (
          <div className="sr-new-chat-wrap">
            <button
              ref={newChatRowRef}
              type="button"
              className="sr-new-chat-btn"
              onClick={openNewChatModal}
              disabled={hardLocked}
              aria-haspopup="dialog"
              title={t('sessionRail.newChat')}
            >
              <span className="sr-new-chat-label">{t('sessionRail.newChat')}</span>
              <span className="sr-new-chat-plus" aria-hidden="true">
                <IconPlus size={14} />
              </span>
            </button>
          </div>
        )}
        {/* 「项目」标签行：右端 ⋯ 菜单 + 📁+（新建项目文件夹 → 「创建项目」弹窗）；
            视觉弱于上方主操作按钮、区别于组头（次级字号 / 弱色 / 左对齐）。
            📁+ 需要「切当前目录」这一入口才能完成创建（归属 = 切好目录后生成的空对话），
            缺它时不渲染该按钮——与「新建对话」动作行同一可用性判据 */}
        <ProjectLabelRow
          prefs={sortPrefs}
          archivedProjects={archivedProjects}
          onExpandAll={expandAllGroups}
          onCollapseAll={collapseAllGroups}
          onSelectSort={handleSelectSort}
          onRestore={handleRestoreProject}
          onCreateProject={onSwitchProjectDir ? openCreateProjectModal : undefined}
          plusBtnRef={plusBtnRef}
        />
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

      {/* 新建对话弹窗：会话标题 + 归属项目（常驻列表，末位「浏览本地目录…」）。
          确认后由 handleCreateNewChat 按「切目录 → 记录标题并回欢迎页 → 刷新」执行：
          会话本身在欢迎页直发首条消息时诞生，标题随之落成 */}
      <NewChatModal
        open={newChatOpen}
        projects={projects}
        onClose={closeNewChatModal}
        onBrowseDir={handleBrowseNewChatDir}
        onCreate={handleCreateNewChat}
      />

      {/* 创建项目弹窗（「项目」行 📁+ 唯一入口）：项目名称 + 源文件夹。
          确认后由 handleCreateProject 按「写书签 → 切当前目录 → 生成空对话 → 刷新」执行：
          rail 里该文件夹下立刻出现 1 条当前对话（草稿，纯内存态，不落库） */}
      <CreateProjectModal
        open={createProjectOpen}
        onClose={closeCreateProjectModal}
        onPickDir={handlePickProjectDir}
        onSubmit={handleCreateProject}
      />
    </>
  )
}
