import { describe, expect, it } from 'vitest'
import {
  DEFAULT_GROUP_LIMIT,
  DEFAULT_SESSION_SORT_PREFS,
  UNGROUPED_GROUP_KEY,
  buildSessionGroups,
  dirDisplayName,
  normalizeGroupLimit,
  normalizePathKey,
  normalizeSessionSortPrefs,
  visibleGroupSessions,
  type GroupProjectLike,
  type GroupSessionLike,
  type SessionGroup,
} from '../main-window/chat/sessionGroups'

/**
 * 会话条目构造（只保留分组关心的字段，其余由真实返回体提供）。
 * `created_at` 省略时按会话真实语义模拟「老后端/未落盘」→ 排序退化为 `updated_at`。
 */
function session(
  id: string,
  project_path: string | null,
  updated_at: number,
  created_at?: number,
): GroupSessionLike {
  return { id, project_path, updated_at, created_at }
}

function project(
  path: string,
  name: string,
  opts: { is_current?: boolean; auto?: boolean } = {},
): GroupProjectLike {
  return { path, name, is_current: !!opts.is_current, auto: !!opts.auto }
}

/**
 * 会话工作台项目文件夹分组纯函数回归。
 * 规则出处：任务决策 2/6/7/8 + 后端 list_shelf_sessions 返回体（items/projects/
 * archived_projects/collapsed_limit）。
 */
describe('sessionGroups 分组规则', () => {
  it('组顺序 = projects[] 顺序（书签序 → auto），未分组固定末位', () => {
    const groups = buildSessionGroups(
      [
        session('s-auto', 'E:\\NUS\\2', 300),
        session('s-none', null, 400),
        session('s-bm1', 'E:\\NUS\\1', 200),
        session('s-bm2', 'E:\\NUS\\Nuphus', 100),
      ],
      [
        project('E:\\NUS\\1', '一号'),
        project('E:\\NUS\\Nuphus', 'Nuphus', { is_current: true }),
        project('E:\\NUS\\2', '二号', { auto: true }),
      ],
      [],
    )

    expect(groups.map(g => g.name)).toEqual(['一号', 'Nuphus', '二号', ''])
    expect(groups[3].key).toBe(UNGROUPED_GROUP_KEY)
    expect(groups[3].path).toBeNull()
    expect(groups[3].sessions.map(s => s.id)).toEqual(['s-none'])
    // 当前工作目录组不上浮：仍是书签序第 2 位（isCurrent 只是数据标记，不再驱动视觉，
    // 但仍是「组内会话点击跳过重复 set_project_dir」的幂等判据）
    expect(groups[1].isCurrent).toBe(true)
    // auto 只读组标记保留（供 UI 关闭重命名/归档入口）
    expect(groups.map(g => g.auto)).toEqual([false, false, true, false])
  })

  it('归档文件夹整组隐藏，其下会话不得落进「未分组」', () => {
    const groups = buildSessionGroups(
      [
        session('s-archived', 'E:\\work\\Old', 500),
        session('s-live', 'E:\\NUS\\1', 400),
        session('s-none', null, 100),
      ],
      [project('E:\\NUS\\1', '一号')],
      [project('E:\\work\\Old', '已归档目录')],
    )

    // 归档组不出现在可见组
    expect(groups.some(g => g.path === 'E:\\work\\Old')).toBe(false)
    // 归档组下的会话不被任何组收留（尤其不能出现在「未分组」）
    const allIds = groups.flatMap(g => g.sessions.map(s => s.id))
    expect(allIds).not.toContain('s-archived')
    const ungrouped = groups.find(g => g.key === UNGROUPED_GROUP_KEY)!
    expect(ungrouped.sessions.map(s => s.id)).toEqual(['s-none'])
  })

  it('无归属会话（project_path=null）落「未分组」，有归属的每一组内按 updated_at 倒序', () => {
    const groups = buildSessionGroups(
      [
        session('a-old', 'E:\\NUS\\1', 100),
        session('a-new', 'E:\\NUS\\1', 900),
        session('a-mid', 'E:\\NUS\\1', 500),
        session('n-1', null, 10),
        session('n-2', null, 20),
      ],
      [project('E:\\NUS\\1', '一号')],
      [],
    )

    expect(groups[0].sessions.map(s => s.id)).toEqual(['a-new', 'a-mid', 'a-old'])
    expect(groups[1].key).toBe(UNGROUPED_GROUP_KEY)
    expect(groups[1].sessions.map(s => s.id)).toEqual(['n-2', 'n-1'])
  })

  it('空文件夹（书签存在但无会话）仍建组；无未分组会话时不产生「未分组」组', () => {
    const groups = buildSessionGroups(
      [session('s1', 'E:\\NUS\\1', 1)],
      [project('E:\\NUS\\1', '一号'), project('E:\\NUS\\empty', '空目录')],
      [],
    )

    expect(groups).toHaveLength(2)
    expect(groups[1].name).toBe('空目录')
    expect(groups[1].sessions).toEqual([])
    expect(groups.some(g => g.key === UNGROUPED_GROUP_KEY)).toBe(false)
  })

  it('路径归一：大小写/尾分隔符差异可归组；分隔符风格不同不强行合并', () => {
    const groups = buildSessionGroups(
      [
        session('case', 'e:\\nus\\2\\', 2),
        session('slash', 'E:/NUS/1', 1),
        session('archived-case', 'E:\\WORK\\OLD\\', 3),
      ],
      [project('E:\\NUS\\1', '一号'), project('E:\\NUS\\2', '二号')],
      [project('E:\\work\\Old', '已归档目录')],
    )

    // 大小写 + 尾分隔符差异 → 命中书签组（Windows 下后端 same_project_path 同样忽略大小写）
    expect(groups[1].sessions.map(s => s.id)).toEqual(['case'])
    // 分隔符风格不同（/ vs \）两层都不归一：后端 same_project_path 亦不折叠，
    // 前端不擅自放宽（否则会话会落进后端未认可的组）→ 兜底进「未分组」
    const ungrouped = groups.find(g => g.key === UNGROUPED_GROUP_KEY)!
    expect(ungrouped.sessions.map(s => s.id)).toEqual(['slash'])
    // 归档组大小写不同也必须隐藏，不得落「未分组」
    expect(groups.flatMap(g => g.sessions.map(s => s.id))).not.toContain('archived-case')
    expect(groups.map(g => g.name)).toEqual(['一号', '二号', ''])
  })

  it('字段缺失/异常值不抛错：空 projects + 无 updated_at 会话', () => {
    const groups = buildSessionGroups(
      [{ id: 'no-ts' }, { id: 'bad-ts', project_path: '  ', updated_at: 'oops' }],
      [],
      [],
    )

    expect(groups).toHaveLength(1)
    expect(groups[0].key).toBe(UNGROUPED_GROUP_KEY)
    expect(groups[0].sessions.map(s => s.id)).toEqual(['no-ts', 'bad-ts'])
  })

  it('normalizePathKey / dirDisplayName 边界', () => {
    expect(normalizePathKey(null)).toBe('')
    expect(normalizePathKey('  E:\\a\\  ')).toBe('E:\\a')
    expect(dirDisplayName('E:\\NUS\\Nuphus\\')).toBe('Nuphus')
    expect(dirDisplayName('Nuphus')).toBe('Nuphus')
  })
})

describe('sessionGroups 折叠上限', () => {
  const groups = buildSessionGroups(
    Array.from({ length: 8 }, (_, i) => session(`s${i}`, 'E:\\NUS\\1', 1000 - i)),
    [project('E:\\NUS\\1', '一号')],
    [],
  )
  const group = groups[0]

  it('折叠态只给前 limit 条，并报出「展开其余 N 个会话」的条数', () => {
    const slice = visibleGroupSessions(group, 6, false)
    expect(slice.sessions).toHaveLength(6)
    expect(slice.sessions.map(s => s.id)).toEqual(['s0', 's1', 's2', 's3', 's4', 's5'])
    expect(slice.hiddenCount).toBe(2)
  })

  it('展开后全显、hiddenCount 归零', () => {
    const slice = visibleGroupSessions(group, 6, true)
    expect(slice.sessions).toHaveLength(8)
    expect(slice.hiddenCount).toBe(0)
  })

  it('未超出上限时不折叠；上限非法（0/负数/NaN）回落默认值 6', () => {
    expect(visibleGroupSessions(group, 20, false)).toEqual({
      sessions: group.sessions,
      hiddenCount: 0,
    })
    expect(normalizeGroupLimit(0)).toBe(DEFAULT_GROUP_LIMIT)
    expect(normalizeGroupLimit(-3)).toBe(DEFAULT_GROUP_LIMIT)
    expect(normalizeGroupLimit(Number.NaN)).toBe(DEFAULT_GROUP_LIMIT)
    expect(normalizeGroupLimit(undefined)).toBe(DEFAULT_GROUP_LIMIT)
    expect(normalizeGroupLimit(3.9)).toBe(3)
    expect(DEFAULT_GROUP_LIMIT).toBe(6)
  })
})
/**
 * 排序偏好：两个**独立**维度（组序维度 × 组内键）。
 *
 * 夹具刻意让两个维度岔开，否则「换了排序却看不出变化」，等于没测：
 * - 一号组：updated 倒序 = [a-old(500), a-new(400), a-mid(100)]；
 *   created 升序 = [a-old(100), a-mid(500), a-new(900)]；
 * - 空文件夹：书签存在但无会话 → 组序维度 `recent` 下必须落末位；
 * - 未分组：updated 最大（900）却**恒末位**（决策 6，不受排序偏好推翻）。
 */
describe('sessionGroups 排序偏好（组序维度 × 组内键）', () => {
  const items = [
    session('a-old', 'E:\\A', 500, 100),
    session('a-new', 'E:\\A', 400, 900),
    session('a-mid', 'E:\\A', 100, 500),
    session('b-1', 'E:\\B', 300, 300),
    session('n-1', null, 900, 900),
  ]
  const projects = [
    project('E:\\A', '一号'),
    project('E:\\Empty', '空文件夹'),
    project('E:\\B', '二号'),
  ]

  const build = (groupOrder: 'bookmark' | 'recent', sortKey: 'updated' | 'created') =>
    buildSessionGroups(items, projects, [], { groupOrder, sortKey })

  const names = (groups: SessionGroup<GroupSessionLike>[]) => groups.map(g => g.name)
  const idsIn = (groups: SessionGroup<GroupSessionLike>[], name: string) =>
    groups.find(g => g.name === name)!.sessions.map(s => s.id)

  it('省略 opts = 默认（按项目 + 更新时间），与显式传默认值结果一致', () => {
    const implicit = buildSessionGroups(items, projects, [])
    const explicit = build('bookmark', 'updated')

    expect(names(implicit)).toEqual(['一号', '空文件夹', '二号', ''])
    expect(implicit.map(g => g.sessions.map(s => s.id))).toEqual(
      explicit.map(g => g.sessions.map(s => s.id)),
    )
    expect(DEFAULT_SESSION_SORT_PREFS).toEqual({ groupOrder: 'bookmark', sortKey: 'updated' })
  })

  it('① 按项目 + 更新时间：组序 = 书签序，组内 updated 倒序', () => {
    const groups = build('bookmark', 'updated')

    expect(names(groups)).toEqual(['一号', '空文件夹', '二号', ''])
    expect(idsIn(groups, '一号')).toEqual(['a-old', 'a-new', 'a-mid'])
    expect(idsIn(groups, '二号')).toEqual(['b-1'])
    expect(groups[3].key).toBe(UNGROUPED_GROUP_KEY)
    expect(idsIn(groups, '')).toEqual(['n-1'])
  })

  it('② 按项目 + 创建时间：组序不变，组内 created 升序（早的在上）', () => {
    const groups = build('bookmark', 'created')

    expect(names(groups)).toEqual(['一号', '空文件夹', '二号', ''])
    expect(idsIn(groups, '一号')).toEqual(['a-old', 'a-mid', 'a-new'])
    expect(idsIn(groups, '二号')).toEqual(['b-1'])
  })

  it('③ 近期项目 + 更新时间：组序按组内最近会话倒序，空组末位，未分组恒末位', () => {
    const groups = build('recent', 'updated')

    // 组内最新会话时间：一号 500 > 二号 300 > 空文件夹（无会话 → 末位）
    expect(names(groups)).toEqual(['一号', '二号', '空文件夹', ''])
    // 未分组会话 updated=900（全场最大）仍固定末位：组序维度不作用于兜底组
    expect(groups[3].key).toBe(UNGROUPED_GROUP_KEY)
    // 组内顺序仍按组内键（更新时间倒序）
    expect(idsIn(groups, '一号')).toEqual(['a-old', 'a-new', 'a-mid'])
  })

  it('④ 近期项目 + 创建时间：两个维度互相独立', () => {
    const groups = build('recent', 'created')

    expect(names(groups)).toEqual(['一号', '二号', '空文件夹', ''])
    expect(idsIn(groups, '一号')).toEqual(['a-old', 'a-mid', 'a-new'])
  })

  it('近期项目稳定序：组内最近时间相同时保持书签顺序', () => {
    const groups = buildSessionGroups(
      [session('x1', 'E:\\X', 100, 1), session('y1', 'E:\\Y', 100, 1)],
      [project('E:\\X', 'X'), project('E:\\Y', 'Y')],
      [],
      { groupOrder: 'recent', sortKey: 'updated' },
    )

    expect(names(groups)).toEqual(['X', 'Y'])
  })

  it('created_at 缺失（老后端/未落盘）退化为 updated_at，不造值', () => {
    const groups = buildSessionGroups(
      [session('newer', 'E:\\X', 900), session('older', 'E:\\X', 100)],
      [project('E:\\X', 'X')],
      [],
      { groupOrder: 'bookmark', sortKey: 'created' },
    )

    expect(groups[0].sessions.map(s => s.id)).toEqual(['older', 'newer'])
  })

  it('归档文件夹整组隐藏的规则不因排序偏好改变', () => {
    const groups = buildSessionGroups(
      [session('arch', 'E:\\Old', 900, 900), session('a-old', 'E:\\A', 500, 100)],
      projects,
      [project('E:\\Old', '旧归档')],
      { groupOrder: 'recent', sortKey: 'created' },
    )

    expect(groups.flatMap(g => g.sessions.map(s => s.id))).not.toContain('arch')
  })

  it('normalizeSessionSortPrefs：缺字段 / 非法值回落默认，合法值取回', () => {
    expect(normalizeSessionSortPrefs(undefined)).toEqual(DEFAULT_SESSION_SORT_PREFS)
    expect(normalizeSessionSortPrefs({})).toEqual(DEFAULT_SESSION_SORT_PREFS)
    expect(normalizeSessionSortPrefs(null)).toEqual(DEFAULT_SESSION_SORT_PREFS)
    expect(normalizeSessionSortPrefs({ group_order: 'recent', sort_key: 'created' })).toEqual({
      groupOrder: 'recent',
      sortKey: 'created',
    })
    // 大小写/空白容错（与后端归一语义一致）
    expect(normalizeSessionSortPrefs({ group_order: ' Recent ', sort_key: 'CREATED' })).toEqual({
      groupOrder: 'recent',
      sortKey: 'created',
    })
    // 非法值 → 默认，不把怪值透传进排序
    expect(normalizeSessionSortPrefs({ group_order: 'newest', sort_key: 42 })).toEqual(
      DEFAULT_SESSION_SORT_PREFS,
    )
  })
})
