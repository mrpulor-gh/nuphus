/**
 * sessionGroups.ts — 会话工作台「项目文件夹分组」纯函数（无副作用、无依赖）
 *
 * 桌面 SessionRail 与移动端 NavBar 共用同一套分组规则，避免两端各写一套漂移。
 * 数据**全部来自** `list_shelf_sessions` 返回体的 `items / projects / archived_projects /
 * collapsed_limit` 四个字段——本模块不做任何归属推断（绝不用「当前工作目录」给
 * `project_path === null` 的会话补归属）。
 *
 * 分组规则（与任务决策一一对应）：
 * 1. 组顺序 = `projects[]` 顺序（书签顺序 → 未收藏但有会话的 auto 只读组）；
 * 2. `project_path === null/空` 的会话 → 「未分组」兜底组，**固定末位**（决策 6）；
 * 3. 出现在 `archived_projects[]` 的路径 → **整组隐藏**，其下会话一并隐藏，
 *    不得落进「未分组」（决策 8）；
 * 4. 组内按排序键排：`updated` = `updated_at` 倒序（默认）；`created` = `created_at`
 *    升序（早的在上）。同值保持传入顺序（显式下标比较，不依赖引擎稳定性）；
 * 5. 组内折叠上限 = 全局单值 `collapsed_limit`（决策 9），超出部分由 UI 折叠为
 *    「展开其余 N 个会话」——折叠/展开是运行时 UI 状态，故此处只给切片结果。
 *
 * 排序偏好（两个**独立**维度，落 preferences、桌面/移动同一读数）：
 * - **组序维度** `groupOrder`：`bookmark` = 组顺序 = 书签顺序（默认，= 现状）；
 *   `recent` = 仍按项目分组，但组顺序改为「组内最近一次会话时间」倒序，
 *   无会话的空组恒末位（稳定序）。**不管组内顺序**。
 * - **组内键** `sortKey`：`updated` / `created`。**不影响组顺序**。
 * - 「未分组」兜底组不受组序维度影响：始终固定末位（决策 6，未被排序偏好推翻）。
 */

/** 全局折叠上限读数兜底（后端 `session_group_limit()` 的 0→默认值同义，前端二次兜底） */
export const DEFAULT_GROUP_LIMIT = 6

/** 未分组兜底组的 key（绝对路径恒非空 → 空串不会与任何真实组路径冲突） */
export const UNGROUPED_GROUP_KEY = ''

/** 组序维度：`bookmark` = 按书签顺序（默认）；`recent` = 按组内最近会话时间倒序 */
export type SessionGroupOrder = 'bookmark' | 'recent'

/** 组内排序键：`updated` = 更新时间倒序（默认）；`created` = 创建时间早的在上 */
export type SessionSortKey = 'updated' | 'created'

/** 排序偏好（值域与后端 `UserPreferences.session_group_order / session_sort_key` 对齐） */
export interface SessionSortPrefs {
  groupOrder: SessionGroupOrder
  sortKey: SessionSortKey
}

/** 排序偏好默认值（= 分组功能引入前的行为；后端缺字段/非法值也归一到这里） */
export const DEFAULT_SESSION_SORT_PREFS: SessionSortPrefs = {
  groupOrder: 'bookmark',
  sortKey: 'updated',
}

/** 后端 `sort_prefs` 载荷的字段名（snake_case，与 Rust 序列化一致） */
interface RawSortPrefs {
  group_order?: unknown
  sort_key?: unknown
}

/** 单个取值归一：只认目标 token（去空白 + 忽略大小写），其余回落默认 */
function pickToken<T extends string>(raw: unknown, token: T, fallback: T): T {
  return typeof raw === 'string' && raw.trim().toLowerCase() === token ? token : fallback
}

/**
 * 后端 `sort_prefs` → 前端偏好（缺字段 / 非法值 → 默认）。
 *
 * 后端已做过同义归一，这里再兜一层：老版本后端（无该字段）或异常响应不会让
 * 排序控件出现「无选中项」的空状态。
 */
export function normalizeSessionSortPrefs(raw: unknown): SessionSortPrefs {
  const prefs = (raw ?? {}) as RawSortPrefs
  return {
    groupOrder: pickToken(prefs.group_order, 'recent', 'bookmark'),
    sortKey: pickToken(prefs.sort_key, 'created', 'updated'),
  }
}

/** 分组所需的最小会话形状（桌面 ShelfSessionItem / 移动端 ShelfSessionItem 均结构兼容） */
export interface GroupSessionLike {
  id: string
  /** Unix 毫秒；旧数据/移动端类型可能是字符串 → 统一按 0 兜底参与排序 */
  updated_at?: number | string
  /**
   * Unix 毫秒（后端 `created_at`，来源 sessions.created_at；尚无落盘行的 active 会话
   * 取首条消息时间戳）。缺失/非法 → 排序时退化为 `updated_at`（见 [`createdMillis`]）。
   */
  created_at?: number | string
  /** 会话归属目录（诞生时快照）；null/缺失 = 无归属 → 「未分组」 */
  project_path?: string | null
}

/** 分组所需的最小项目形状（对齐后端 ProjectEntry） */
export interface GroupProjectLike {
  path: string
  name: string
  is_current: boolean
  auto: boolean
}

export interface SessionGroup<T extends GroupSessionLike = GroupSessionLike> {
  /** 分组键：组路径；未分组组为 UNGROUPED_GROUP_KEY（空串） */
  key: string
  /** 组路径（可用于 set_project_dir / 归档）；未分组组为 null */
  path: string | null
  /** 展示名：书签自定义名 / 自动组取目录末段；未分组组为空串（由 UI 按 i18n 命名） */
  name: string
  /** 当前工作目录所在组（仅高亮，不上浮——决策 2） */
  isCurrent: boolean
  /** true = 未收藏但有会话的自动组（只读：不可重命名/归档——决策 7） */
  auto: boolean
  /** 组内会话，已按排序偏好排好（`updated` 倒序 / `created` 升序） */
  sessions: T[]
}

/** 路径末段名（自动组无 name 时的兜底展示名，沿用项目中心 nameFromPath 规则） */
export function dirDisplayName(path: string): string {
  const trimmed = (path ?? '').replace(/[\\/]+$/, '')
  return trimmed.split(/[\\/]/).pop() || trimmed
}

/**
 * 路径归一化：trim + 去尾部分隔符（对齐后端 `same_project_path` 的 norm）。
 * 只做「同一路径的不同书写」归一，不改变路径本身语义。
 */
export function normalizePathKey(path: string | null | undefined): string {
  return (path ?? '').trim().replace(/[\\/]+$/, '')
}

/**
 * 折叠上限读数校验：非数值 / <1 / 0（后端显式拒绝 0）→ 回落到默认值。
 * 与后端 `UserPreferences::session_group_limit()` 的 0→默认值语义一致，前端再兜一层，
 * 避免异常响应把每个分组都折叠成空列表。
 */
export function normalizeGroupLimit(limit: unknown): number {
  const n = typeof limit === 'number' ? limit : Number(limit)
  return Number.isFinite(n) && n >= 1 ? Math.floor(n) : DEFAULT_GROUP_LIMIT
}

/** updated_at → 可比毫秒（number 直取；字符串尝试解析，失败按 0） */
function toMillis(v: number | string | undefined): number {
  if (typeof v === 'number') return Number.isFinite(v) ? v : 0
  if (typeof v === 'string') {
    const n = Date.parse(v)
    return Number.isFinite(n) ? n : 0
  }
  return 0
}

/** 组内排序：updated_at 倒序；同时间保持原顺序（显式下标比较，不依赖引擎稳定性） */
function sortByUpdatedDesc<T extends GroupSessionLike>(list: T[]): T[] {
  return list
    .map((item, index) => ({ item, index, ts: toMillis(item.updated_at) }))
    .sort((a, b) => b.ts - a.ts || a.index - b.index)
    .map(x => x.item)
}

/**
 * 创建时间读数（Unix 毫秒）：`created_at` 优先，缺失/非法（老后端、mock 夹具、
 * 未落盘会话）退化为 `updated_at`——绝不用 0 / now 之类凭空值参与排序。
 */
function createdMillis(item: GroupSessionLike): number {
  const created = toMillis(item.created_at)
  return created > 0 ? created : toMillis(item.updated_at)
}

/** 组内排序：`updated` = 更新时间倒序（默认）；`created` = 创建时间升序（早的在上） */
function sortSessions<T extends GroupSessionLike>(list: T[], sortKey: SessionSortKey): T[] {
  if (sortKey !== 'created') return sortByUpdatedDesc(list)
  return list
    .map((item, index) => ({ item, index, ts: createdMillis(item) }))
    .sort((a, b) => a.ts - b.ts || a.index - b.index)
    .map(x => x.item)
}

/** 组内最近一次会话时间（组序维度 `recent` 用）；空组返回 null（恒末位） */
function latestSessionMillis(group: SessionGroup<GroupSessionLike>): number | null {
  let latest: number | null = null
  for (const s of group.sessions) {
    const ts = toMillis(s.updated_at)
    if (latest === null || ts > latest) latest = ts
  }
  return latest
}

/**
 * 组序维度 `recent`：按「组内最近一次会话时间」倒序；空组恒末位，同值保持原序（稳定）。
 * 不改动组内会话顺序（组内顺序只由 [`sortSessions`] 决定）。
 */
function sortGroupsByRecency<T extends GroupSessionLike>(
  groups: SessionGroup<T>[],
): SessionGroup<T>[] {
  return groups
    .map((group, index) => ({ group, index, latest: latestSessionMillis(group) }))
    .sort((a, b) => {
      if ((a.latest === null) !== (b.latest === null)) return a.latest === null ? 1 : -1
      if (a.latest !== null && b.latest !== null && a.latest !== b.latest) {
        return b.latest - a.latest
      }
      return a.index - b.index
    })
    .map(x => x.group)
}

/**
 * 组装分组视图。
 *
 * 折叠上限不参与分组本身（只影响渲染切片，见 [`visibleGroupSessions`]），
 * 故此处不接收 limit——避免出现「传了却不生效」的假参数。
 *
 * @param items            会话条目（顺序不参与分组，仅作同时间兜底序）
 * @param projects         可见项目文件夹（书签 → auto），顺序即组顺序（`bookmark` 维度）
 * @param archivedProjects 已归档项目文件夹（整组隐藏，其会话不落「未分组」）
 * @param prefs            排序偏好（组序维度 + 组内键）；缺省 = 默认（按项目 / 更新时间）
 */
export function buildSessionGroups<T extends GroupSessionLike>(
  items: readonly T[],
  projects: readonly GroupProjectLike[],
  archivedProjects: readonly GroupProjectLike[],
  prefs: SessionSortPrefs = DEFAULT_SESSION_SORT_PREFS,
): SessionGroup<T>[] {
  const archivedKeys = new Set<string>()
  const archivedLowerKeys = new Set<string>()
  for (const p of archivedProjects) {
    const key = normalizePathKey(p.path)
    if (!key || archivedKeys.has(key)) continue
    archivedKeys.add(key)
    archivedLowerKeys.add(key.toLowerCase())
  }
  /** 归档判定：精确优先，大小写兜底（Windows 下后端 same_project_path 忽略大小写） */
  const isArchived = (key: string) =>
    archivedKeys.has(key) || archivedLowerKeys.has(key.toLowerCase())

  const groups: SessionGroup<T>[] = []
  const byKey = new Map<string, SessionGroup<T>>()
  const byLowerKey = new Map<string, SessionGroup<T>>()
  for (const p of projects) {
    const key = normalizePathKey(p.path)
    if (!key || byKey.has(key)) continue
    const group: SessionGroup<T> = {
      key,
      path: key,
      name: p.name?.trim() || dirDisplayName(key),
      isCurrent: !!p.is_current,
      auto: !!p.auto,
      sessions: [],
    }
    byKey.set(key, group)
    const lower = key.toLowerCase()
    if (!byLowerKey.has(lower)) byLowerKey.set(lower, group)
    groups.push(group)
  }

  const ungrouped: T[] = []
  for (const item of items) {
    const key = normalizePathKey(item.project_path)
    // 无归属：不猜测，进「未分组」兜底组
    if (!key) {
      ungrouped.push(item)
      continue
    }
    // 归档文件夹下的会话：随组一并隐藏（不得落「未分组」）
    if (isArchived(key)) continue
    const group = byKey.get(key) ?? byLowerKey.get(key.toLowerCase())
    if (group) {
      group.sessions.push(item)
    } else {
      // 契约外路径（后端 projects[] 已覆盖全部未归档归属，正常不会走到）：
      // 宁可归入「未分组」也不按当前目录推断归属。
      ungrouped.push(item)
    }
  }

  // 组内排序键（两个维度互相独立：此处只动组内顺序）
  for (const g of groups) g.sessions = sortSessions(g.sessions, prefs.sortKey)
  // 组序维度：'recent' 时按组内最近会话倒序（空组末位）；'bookmark' 保持书签顺序
  const ordered = prefs.groupOrder === 'recent' ? sortGroupsByRecency(groups) : groups
  if (ungrouped.length > 0) {
    // 「未分组」兜底组恒定末位（决策 6），不参与组序维度重排
    ordered.push({
      key: UNGROUPED_GROUP_KEY,
      path: null,
      name: '',
      isCurrent: false,
      auto: false,
      sessions: sortSessions(ungrouped, prefs.sortKey),
    })
  }
  return ordered
}

/** 组内折叠切片结果 */
export interface GroupSlice<T extends GroupSessionLike> {
  /** 该组当前应渲染的会话 */
  sessions: T[]
  /** 被折叠的条数（0 = 无折叠，「展开其余 N 个会话」用） */
  hiddenCount: number
}

/**
 * 组内展示切片：未展开且超出上限时只给前 limit 条。
 * 折叠/展开由 UI 逐组持有（运行时状态，不持久化——决策 11）。
 */
export function visibleGroupSessions<T extends GroupSessionLike>(
  group: SessionGroup<T>,
  limit: number,
  expanded: boolean,
): GroupSlice<T> {
  const lim = normalizeGroupLimit(limit)
  if (expanded || group.sessions.length <= lim) {
    return { sessions: group.sessions, hiddenCount: 0 }
  }
  return {
    sessions: group.sessions.slice(0, lim),
    hiddenCount: group.sessions.length - lim,
  }
}
