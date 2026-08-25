import { useState, useEffect, useCallback, useMemo, useRef } from 'react'
import { listMemories, deleteMemory } from '../lib/api-memory'
import { getSessionHistory, getSessionDetail, getMemoryOverview } from '../lib/api'
import { invoke } from '../../core/bridge'
import type { UserMemory, MemoryOverview } from '../../core/types-memory'
import type { SessionSummary, SessionDetailEntry } from '../../core/types'
import {
  IconSearch,
  IconMessageCircle,
  IconStar,
  IconX,
  IconChevronDown,
  IconShield,
} from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { loadRelation } from '../lib/relation'
import '../../styles/memories.css'
import { useLanguage } from '../../locales'

// ============================================================================
// Helpers
// ============================================================================

function formatTime(ts: string, t?: (key: string, ...args: string[]) => string): string {
  try {
    const d = new Date(ts)
    const now = new Date()
    const diff = now.getTime() - d.getTime()
    if (diff < 60000) return t ? t('time.justNow') : 'Just now'
    if (diff < 3600000)
      return t
        ? t('time.minAgo', String(Math.floor(diff / 60000)))
        : `${Math.floor(diff / 60000)} min ago`
    if (diff < 86400000)
      return t
        ? t('time.hrAgo', String(Math.floor(diff / 3600000)))
        : `${Math.floor(diff / 3600000)} hr ago`
    return d.toLocaleDateString('default', { month: 'short', day: 'numeric' })
  } catch {
    return ''
  }
}

function truncate(text: string, max = 120): string {
  if (!text) return ''
  return text.length <= max ? text : text.slice(0, max) + '...'
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

function groupSessionsByDate(
  items: SessionSummary[],
  t: (key: string, ...args: string[]) => string,
): { label: string; items: SessionSummary[] }[] {
  const now = new Date()
  const today = now.toDateString()
  const yesterday = new Date(now.getTime() - 86400000).toDateString()

  const todayItems: SessionSummary[] = []
  const yesterdayItems: SessionSummary[] = []
  const weekItems: SessionSummary[] = []
  const earlierItems: SessionSummary[] = []

  for (const item of items) {
    const d = new Date(item.timestamp)
    const ds = d.toDateString()
    if (ds === today) todayItems.push(item)
    else if (ds === yesterday) yesterdayItems.push(item)
    else if (now.getTime() - d.getTime() < 7 * 86400000) weekItems.push(item)
    else earlierItems.push(item)
  }

  const result: { label: string; items: SessionSummary[] }[] = []
  if (todayItems.length) result.push({ label: t('time.today'), items: todayItems })
  if (yesterdayItems.length) result.push({ label: t('time.yesterday'), items: yesterdayItems })
  if (weekItems.length) result.push({ label: t('time.week'), items: weekItems })
  if (earlierItems.length) result.push({ label: t('time.earlier'), items: earlierItems })
  return result
}

// ============================================================================
// MemoriesPage — 概览 | 会话 | 经验（原则/标注管理在输入框 + 菜单的弹窗中）
// ============================================================================

export function MemoriesPage() {
  const { t } = useLanguage()
  const [activeTab, setActiveTab] = useState<'overview' | 'sessions' | 'experience'>('overview')

  // ── Overview state ──
  const [overview, setOverview] = useState<MemoryOverview | null>(null)
  const [overviewLoading, setOverviewLoading] = useState(false)

  // ── Sessions state ──
  const [sessions, setSessions] = useState<SessionSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
  // 列表/详情分离：selectedSessionId 非 null → 显示详情视图（header 固定返回按钮 + body 独立滚动）
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  const [sessionDetails, setSessionDetails] = useState<Map<string, SessionDetailEntry[]>>(new Map())
  const [loadingDetails, setLoadingDetails] = useState<Set<string>>(new Set())
  const [detailErrors, setDetailErrors] = useState<Map<string, string>>(new Map())

  // ── Experience state（三分类：快照=leader/workflow memory_update、提炼=refine、点评=用户评分）──
  const [experience, setExperience] = useState<UserMemory[]>([])
  const [expLoading, setExpLoading] = useState(false)
  const [expKind, setExpKind] = useState<'snapshot' | 'distill' | 'review'>('snapshot')
  // 列表/详情分离：selectedExpId 非 null → 显示快照/提炼详情视图
  const [selectedExpId, setSelectedExpId] = useState<string | null>(null)

  // ── 经验 → 会话 跨 tab 跳转（distill 条目回溯来源会话）──
  const [pendingSessionJump, setPendingSessionJump] = useState<string | null>(null)
  const [sessionJumpMsg, setSessionJumpMsg] = useState('')

  const relation = useMemo(() => loadRelation(), [])

  // ── Loaders ──

  const loadOverview = useCallback(async () => {
    setOverviewLoading(true)
    try {
      const res = await getMemoryOverview()
      if (res) setOverview(res)
    } catch (e) {
      console.error('Failed to load memory overview:', e)
    } finally {
      setOverviewLoading(false)
    }
  }, [])

  const loadHistory = useCallback(async () => {
    try {
      const sessRes = await getSessionHistory()
      setSessions(sessRes || [])
    } catch (e) {
      console.error('Failed to load history:', e)
    }
  }, [])

  const loadExperience = useCallback(async () => {
    setExpLoading(true)
    try {
      const [patternRes, distillRes, snapshotRes] = await Promise.all([
        listMemories({ kind: 'pattern', limit: 200 }),
        listMemories({ kind: 'distill', limit: 100 }),
        listMemories({ kind: 'snapshot', limit: 100 }),
      ])
      const merged = [
        ...(patternRes?.memories || []),
        ...(distillRes?.memories || []),
        ...(snapshotRes?.memories || []),
      ]
      merged.sort((a, b) => b.created_at.localeCompare(a.created_at))
      setExperience(merged)
    } catch (e) {
      console.error('Failed to load experience:', e)
    } finally {
      setExpLoading(false)
    }
  }, [])

  const loadData = useCallback(async () => {
    setLoading(true)
    try {
      await Promise.all([loadOverview(), loadHistory(), loadExperience()])
    } finally {
      setLoading(false)
    }
  }, [loadOverview, loadHistory, loadExperience])

  useEffect(() => {
    loadData()
  }, [loadData])

  // ── Session detail：空数组不写缓存（允许重试），失败记错误态 ──
  const fetchSessionDetail = useCallback(async (sessionId: string) => {
    setLoadingDetails(prev => new Set(prev).add(sessionId))
    setDetailErrors(prev => {
      const next = new Map(prev)
      next.delete(sessionId)
      return next
    })
    try {
      const detail = await getSessionDetail(sessionId)
      if (detail && detail.length > 0) {
        setSessionDetails(prev => new Map(prev).set(sessionId, detail))
      }
      // 空数组不缓存：渲染层显示空态，下次展开允许重新拉取
    } catch (e) {
      console.error('[MemoriesPage] getSessionDetail 异常', sessionId, e)
      setDetailErrors(prev =>
        new Map(prev).set(sessionId, e instanceof Error ? e.message : String(e)),
      )
    } finally {
      setLoadingDetails(prev => {
        const next = new Set(prev)
        next.delete(sessionId)
        return next
      })
    }
  }, [])

  const selectSession = useCallback(
    (sessionId: string) => {
      setSelectedSessionId(sessionId)
      if (
        !sessionDetails.has(sessionId) &&
        !loadingDetails.has(sessionId) &&
        !detailErrors.has(sessionId)
      ) {
        fetchSessionDetail(sessionId)
      }
    },
    [sessionDetails, loadingDetails, detailErrors, fetchSessionDetail],
  )

  // ── 经验 → 会话 跳转：切 tab + 清空搜索 + 选中 session ──
  const jumpToSession = useCallback((sessionId: string) => {
    setSessionJumpMsg('')
    setSearch('')
    setActiveTab('sessions')
    setPendingSessionJump(sessionId)
  }, [])

  useEffect(() => {
    if (activeTab !== 'sessions' || !pendingSessionJump) return
    if (loading) return
    const sid = pendingSessionJump
    setPendingSessionJump(null)
    if (!sessions.some(s => s.session_id === sid)) {
      setSessionJumpMsg(t('memory.exp.sessionNotFound'))
      return
    }
    setSelectedSessionId(sid)
    if (!sessionDetails.has(sid) && !loadingDetails.has(sid) && !detailErrors.has(sid)) {
      fetchSessionDetail(sid)
    }
  }, [
    activeTab,
    pendingSessionJump,
    loading,
    sessions,
    sessionDetails,
    loadingDetails,
    detailErrors,
    fetchSessionDetail,
    t,
  ])

  // ── Experience 操作 ──
  const handleDeleteExp = useCallback(async (id: string) => {
    try {
      await deleteMemory(id)
      setExperience(prev => prev.filter(m => m.id !== id))
    } catch (e) {
      console.error(e)
    }
  }, [])

  // ── Derived ──

  const filteredSessions = sessions.filter(item => {
    if (!search.trim()) return true
    const q = search.toLowerCase()
    return item.user_message?.toLowerCase().includes(q) || item.intent?.toLowerCase().includes(q)
  })
  const sessionGroups = groupSessionsByDate(filteredSessions, t)

  // 点评 = kind=pattern 且为用户评分产出（tags 含 high_quality|strategy）；
  // StateChecker 历史沉淀 pattern 不在三分类内，留库供 memory_search 检索
  const isUserReview = (m: UserMemory) =>
    m.kind === 'pattern' && m.tags.some(tag => tag === 'high_quality' || tag === 'strategy')
  const filteredExperience = experience.filter(m => {
    if (expKind === 'snapshot' && m.kind !== 'snapshot') return false
    if (expKind === 'distill' && m.kind !== 'distill') return false
    if (expKind === 'review' && !isUserReview(m)) return false
    return true
  })

  const embeddedPct =
    overview && overview.total_entries > 0
      ? Math.round((overview.embedded_count / overview.total_entries) * 100)
      : 0

  const timeSpanDays =
    overview && overview.oldest_ms > 0 && overview.newest_ms > 0
      ? Math.max(1, Math.ceil((overview.newest_ms - overview.oldest_ms) / 86400000))
      : 0
  const formatMsDate = (ms: number) => {
    const d = new Date(ms)
    return `${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
  }
  const timeSpanLabel =
    overview && timeSpanDays > 0
      ? `${timeSpanDays} ${t('memory.overview.days')}（${formatMsDate(overview.oldest_ms)} ~ ${formatMsDate(overview.newest_ms)}）`
      : t('memory.overview.empty')

  // ── Render ──

  return (
    <div className="page unified-timeline">
      <div className="page-tabs">
        <button
          className={`page-tab ${activeTab === 'overview' ? 'active' : ''}`}
          onClick={() => setActiveTab('overview')}
        >
          <IconShield size={14} />
          {t('memory.tab.overview')}
        </button>
        <button
          className={`page-tab ${activeTab === 'sessions' ? 'active' : ''}`}
          onClick={() => setActiveTab('sessions')}
        >
          <IconMessageCircle size={14} />
          {t('memory.tab.sessions')}
        </button>
        <button
          className={`page-tab ${activeTab === 'experience' ? 'active' : ''}`}
          onClick={() => setActiveTab('experience')}
        >
          <IconStar size={14} />
          {t('memory.tab.experience')}
        </button>
      </div>

      {activeTab === 'overview' ? (
        /* ══════════ 概览 Tab ══════════ */
        <div className="ut-timeline">
          {overviewLoading && !overview ? (
            <div className="ut-empty">{t('common.loading')}</div>
          ) : !overview ? (
            <div className="ut-empty">{t('memory.overview.empty')}</div>
          ) : (
            <>
              <div className="ov-grid">
                <div className="ov-card">
                  <div className="ov-card-value">{overview.total_entries}</div>
                  <div className="ov-card-label">{t('memory.overview.total')}</div>
                </div>
                <div className="ov-card">
                  <div className="ov-card-value">{formatBytes(overview.db_size_bytes)}</div>
                  <div className="ov-card-label">{t('memory.overview.dbSize')}</div>
                </div>
                <div className="ov-card">
                  <div className="ov-card-value">{overview.success_rate}%</div>
                  <div className="ov-card-label">{t('memory.overview.successRate')}</div>
                </div>
                <div className="ov-card">
                  <div className="ov-card-value">{embeddedPct}%</div>
                  <div className="ov-card-label">
                    {t('memory.overview.embedded')} ({overview.embedded_count}/
                    {overview.total_entries})
                  </div>
                </div>
              </div>

              <div className="ov-section-title">{t('memory.overview.dataProfile')}</div>
              <div className="ov-grid">
                <div className="ov-card">
                  <div className="ov-card-value">{timeSpanLabel}</div>
                  <div className="ov-card-label">{t('memory.overview.timeSpan')}</div>
                </div>
                <div className="ov-card">
                  <div className="ov-card-value">
                    {overview.distill_count + overview.pattern_count}
                  </div>
                  <div className="ov-card-label">
                    {t('memory.overview.reusable')}（{t('memory.kind.distill')}{' '}
                    {overview.distill_count} + {t('memory.kind.pattern')} {overview.pattern_count}）
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
      ) : activeTab === 'sessions' ? (
        /* ══════════ 会话 Tab ══════════ */
        selectedSessionId ? (
          /* ── 详情视图 ── */
          <SessionDetailView
            sessionId={selectedSessionId}
            details={sessionDetails.get(selectedSessionId)}
            isLoading={loadingDetails.has(selectedSessionId)}
            error={detailErrors.get(selectedSessionId)}
            onBack={() => setSelectedSessionId(null)}
            onRetry={() => fetchSessionDetail(selectedSessionId)}
            userLabel={relation.userLabel}
            assistantName={relation.assistantName}
          />
        ) : (
          /* ── 列表视图 ── */
          <>
            <div className="ut-search-bar">
              <div className="ut-search-wrap">
                <IconSearch size={14} className="ut-search-icon" />
                <input
                  ref={searchRef}
                  className="ut-search-input input-reset"
                  placeholder={t('memory.search')}
                  value={search}
                  onChange={e => setSearch(e.target.value)}
                />
                {search && (
                  <IconButton
                    variant="ghost"
                    label={t('common.clear')}
                    onClick={() => setSearch('')}
                  >
                    <IconX size={14} />
                  </IconButton>
                )}
              </div>
            </div>
            {sessionJumpMsg && (
              <div className="ut-jump-msg">
                <span>{sessionJumpMsg}</span>
                <IconButton
                  variant="ghost"
                  label={t('common.close')}
                  onClick={() => setSessionJumpMsg('')}
                >
                  <IconX size={12} />
                </IconButton>
              </div>
            )}
            <div className="ut-timeline">
              {loading ? (
                <div className="ut-empty">{t('common.loading')}</div>
              ) : sessionGroups.length === 0 ? (
                <div className="ut-empty">
                  {search ? t('cmd.noResults') : t('memory.noRecords')}
                </div>
              ) : (
                sessionGroups.map(group => (
                  <div key={group.label} className="ut-day-group">
                    <div className="ut-day-label">{group.label}</div>
                    {group.items.map((item, i) => (
                      <SessionCard
                        key={`s-${item.session_id}-${i}`}
                        session={item}
                        onClick={() => selectSession(item.session_id)}
                        userLabel={relation.userLabel}
                        assistantName={relation.assistantName}
                      />
                    ))}
                  </div>
                ))
              )}
            </div>
          </>
        )
      ) : activeTab === 'experience' ? (
        /* ══════════ 经验 Tab（三分类：快照 / 提炼 / 点评）══════════ */
        selectedExpId ? (
          /* ── 详情视图 ── */
          <SnapshotDetailView
            memory={filteredExperience.find(m => m.id === selectedExpId)}
            onBack={() => setSelectedExpId(null)}
            onDelete={handleDeleteExp}
            onJumpToSession={jumpToSession}
          />
        ) : (
          /* ── 列表视图 ── */
          <div className="ut-timeline">
            <div className="exp-filters">
              <div className="segmented set-subtabs" role="tablist">
                <button
                  className={`segmented-item ${expKind === 'snapshot' ? 'active' : ''}`}
                  onClick={() => setExpKind('snapshot')}
                >
                  {t('memory.exp.tab.snapshot')}
                </button>
                <button
                  className={`segmented-item ${expKind === 'distill' ? 'active' : ''}`}
                  onClick={() => setExpKind('distill')}
                >
                  {t('memory.exp.tab.distill')}
                </button>
                <button
                  className={`segmented-item ${expKind === 'review' ? 'active' : ''}`}
                  onClick={() => setExpKind('review')}
                >
                  {t('memory.exp.tab.review')}
                </button>
              </div>
            </div>

            {expLoading ? (
              <div className="ut-empty">{t('common.loading')}</div>
            ) : filteredExperience.length === 0 ? (
              <div className="ut-empty">{t('memory.exp.empty')}</div>
            ) : expKind === 'snapshot' ? (
              /* ── 快照列表：按 session 分组 ── */
              <SnapshotSessionList
                snapshots={filteredExperience}
                sessions={sessions}
                onSelect={(id: string) => setSelectedExpId(id)}
                onDelete={handleDeleteExp}
                onJumpToSession={jumpToSession}
              />
            ) : (
              /* ── 提炼 / 点评列表 ── */
              <div className="note-list">
                {filteredExperience.map(m => (
                  <ExperienceCard
                    key={m.id}
                    memory={m}
                    onSelect={() => setSelectedExpId(m.id)}
                    onDelete={() => handleDeleteExp(m.id)}
                    onJumpToSession={jumpToSession}
                  />
                ))}
              </div>
            )}
          </div>
        )
      ) : null}
    </div>
  )
}

// ============================================================================
// SessionDetailView — 会话详情视图（header 固定返回按钮 + body 独立滚动）
// ============================================================================

function SessionDetailView({
  sessionId,
  details,
  isLoading,
  error,
  onBack,
  onRetry,
  userLabel,
  assistantName,
}: {
  sessionId: string
  details?: SessionDetailEntry[]
  isLoading: boolean
  error?: string
  onBack: () => void
  onRetry: () => void
  userLabel: string
  assistantName: string
}) {
  const { t } = useLanguage()
  const title =
    details && details.length > 0
      ? details[0].user_message.substring(0, 80)
      : sessionId.substring(0, 8)
  // 统计 conversation 条目数作为轮次
  const entryCount =
    details?.filter(e => e.kind === 'conversation' || e.kind === 'task_trace').length || 0

  return (
    <div className="ut-detail-page">
      <div className="ut-detail-header">
        <div className="ut-detail-header-row">
          <button className="ut-back-btn" onClick={onBack}>
            <IconChevronDown size={14} style={{ transform: 'rotate(90deg)' }} />
            <span>{t('memory.detail.back')}</span>
          </button>
        </div>
        <div className="ut-detail-header-row">
          <div className="ut-detail-title">{title}</div>
          {entryCount > 0 && (
            <span className="ut-detail-meta">{t('memory.rounds', String(entryCount))}</span>
          )}
        </div>
      </div>
      <div className="ut-detail-body">
        {isLoading ? (
          <div className="ut-loading">{t('common.loading')}</div>
        ) : error ? (
          <div className="ut-detail-state">
            <span className="ut-detail-state-text">
              {t('memory.detail.error')}: {error}
            </span>
            <Button variant="default" size="sm" onClick={onRetry}>
              {t('memory.detail.retry')}
            </Button>
          </div>
        ) : !details || details.length === 0 ? (
          <div className="ut-detail-state">
            <span className="ut-detail-state-text">{t('memory.detail.empty')}</span>
            <Button variant="default" size="sm" onClick={onRetry}>
              {t('memory.detail.retry')}
            </Button>
          </div>
        ) : (
          details.map((entry, i) => {
            const isDistill = entry.kind === 'distill'
            const isSnapshot = entry.kind === 'snapshot'
            const isPattern = entry.kind === 'pattern'
            if (isDistill || isSnapshot || isPattern) {
              const labelKey = isDistill
                ? 'memory.kind.distill'
                : isSnapshot
                  ? 'memory.kind.snapshot'
                  : 'memory.kind.pattern'
              const cardClass = isDistill
                ? 'ut-distill-card'
                : isSnapshot
                  ? 'ut-distill-card ut-snapshot-card'
                  : 'ut-distill-card ut-pattern-card'
              return (
                <div key={entry.id || i} className={cardClass}>
                  <div className="ut-distill-label">{t(labelKey)}</div>
                  <div className="ut-distill-text">
                    {entry.user_message.replace(/^\[提炼\]\s*/, '')}
                  </div>
                </div>
              )
            }
            return (
              <div key={entry.id || i} className="ut-entry">
                <div className="ut-entry-user">
                  <span className="ut-entry-label">{userLabel}</span>
                  <span className="ut-entry-text">{entry.user_message}</span>
                </div>
                <div className="ut-entry-msg">
                  <span className="ut-entry-label">{assistantName}</span>
                  {entry.assistant_message.trim() === '' ? (
                    <div className="ut-entry-text ut-entry-text--empty">
                      {t('memory.detail.noReply')}
                    </div>
                  ) : (
                    <div className="ut-entry-text">{entry.assistant_message}</div>
                  )}
                </div>
                {(entry.steps_summary.length > 0 || entry.goal_type) && (
                  <div className="ut-entry-tags">
                    {entry.goal_type && <span className="ann-tag">{entry.goal_type}</span>}
                    {entry.steps_summary.map((tool, ti) => (
                      <span key={ti} className="ann-tag ann-tag--sm">
                        {tool}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}

// ============================================================================
// SessionCard — 会话卡片（列表视图，点击跳转到详情）
// ============================================================================

function SessionCard({
  session,
  onClick,
  userLabel,
  assistantName,
}: {
  session: SessionSummary
  onClick: () => void
  userLabel: string
  assistantName: string
}) {
  const { t } = useLanguage()
  return (
    <div
      className="ut-session ut-session--clickable"
      data-session-id={session.session_id}
      onClick={onClick}
    >
      <div className="ut-session-header">
        <div className="ut-session-left">
          <IconMessageCircle size={14} className="ut-session-icon" />
          <div className="ut-session-info">
            <div className="ut-session-title">
              {session.user_message || t('memory.emptySession')}
            </div>
            {session.last_assistant_message && (
              <div className="ut-session-preview">{session.last_assistant_message}</div>
            )}
            <div className="ut-session-meta">
              <span>{formatTime(session.timestamp, t)}</span>
              {session.entry_count > 0 && (
                <>
                  <span className="ut-meta-sep">·</span>
                  <span>{t('memory.rounds', String(session.entry_count))}</span>
                </>
              )}
              {session.tool_call_count > 0 && (
                <>
                  <span className="ut-meta-sep">·</span>
                  <span>{t('memory.toolCalls', String(session.tool_call_count))}</span>
                </>
              )}
            </div>
          </div>
        </div>
        <IconChevronDown size={14} className="ut-chevron" style={{ transform: 'rotate(-90deg)' }} />
      </div>
    </div>
  )
}

// ============================================================================
// SnapshotSessionList — 快照按 session 分组（Leader/WorkflowAgent 分别展示）
// ============================================================================
// 快照以会话为粒度展示：每个卡片对应一个 session，复用会话标签页的 SessionSummary 元数据。
// 没有对应 session 记录的孤立快照（历史数据）不展示。

function SnapshotSessionList({
  snapshots,
  sessions,
  onSelect,
  onDelete,
  onJumpToSession,
}: {
  snapshots: UserMemory[]
  sessions: SessionSummary[]
  onSelect: (id: string) => void
  onDelete: (id: string) => void
  onJumpToSession: (sessionId: string) => void
}) {
  const { t } = useLanguage()
  const sessionMap = useMemo(() => {
    const map = new Map<string, SessionSummary>()
    for (const s of sessions) map.set(s.session_id, s)
    return map
  }, [sessions])

  const sessionGroups = useMemo(() => {
    const map = new Map<string, { snapshot: UserMemory; session: SessionSummary }>()
    for (const s of snapshots) {
      const sess = sessionMap.get(s.session_id)
      if (!sess) continue
      const existing = map.get(s.session_id)
      if (!existing || s.created_at > existing.snapshot.created_at) {
        map.set(s.session_id, { snapshot: s, session: sess })
      }
    }
    return Array.from(map.values()).sort((a, b) =>
      b.session.timestamp.localeCompare(a.session.timestamp),
    )
  }, [snapshots, sessionMap])

  if (sessionGroups.length === 0) {
    return <div className="ut-empty">{t('memory.exp.empty')}</div>
  }

  return (
    <>
      {sessionGroups.map(({ snapshot, session }) => (
        <div
          key={session.session_id}
          className="ut-session ut-session--clickable"
          onClick={() => onSelect(snapshot.id)}
        >
          <div className="ut-session-header">
            <div className="ut-session-left">
              <IconMessageCircle size={14} className="ut-session-icon" />
              <div className="ut-session-info">
                <div className="ut-session-title">
                  {session.user_message || t('memory.emptySession')}
                </div>
                <div className="ut-session-meta">
                  <span>{formatTime(session.timestamp, t)}</span>
                  <span className="ut-meta-sep">·</span>
                  <span className="kind-badge kind-badge--distill">
                    {t('memory.kind.snapshot')}
                  </span>
                </div>
              </div>
            </div>
            <IconChevronDown
              size={14}
              className="ut-chevron"
              style={{ transform: 'rotate(-90deg)' }}
            />
          </div>
        </div>
      ))}
    </>
  )
}

// ============================================================================
// ExperienceCard — 经验卡片（快照/提炼/点评 三分类：同名徽标 + 来源信息 + 删除）
// ============================================================================

function ExperienceCard({
  memory,
  onSelect,
  onDelete,
  onJumpToSession,
}: {
  memory: UserMemory
  onSelect: () => void
  onDelete: () => void
  onJumpToSession: (sessionId: string) => void
}) {
  const { t } = useLanguage()
  const sourceBadge =
    memory.kind === 'snapshot'
      ? { label: t('memory.exp.tab.snapshot'), cls: 'kind-badge--distill' }
      : memory.kind === 'pattern'
        ? { label: t('memory.exp.tab.review'), cls: 'kind-badge--user-review' }
        : { label: t('memory.exp.tab.distill'), cls: 'kind-badge--system' }

  return (
    <div className="note-card ut-session--clickable" onClick={onSelect}>
      <div className="note-card-header">
        <div className="note-card-left">
          <div className="note-card-title">
            <span className={`kind-badge ${sourceBadge.cls}`}>{sourceBadge.label}</span>
            {memory.intent || memory.summary || t('memory.noTitle')}
          </div>
          <div className="note-card-meta">
            <span>{formatTime(memory.created_at, t)}</span>
            {memory.goal_type && (
              <>
                <span className="ut-meta-sep">·</span>
                <span>{memory.goal_type}</span>
              </>
            )}
          </div>
        </div>
        <IconChevronDown size={14} className="ut-chevron" style={{ transform: 'rotate(-90deg)' }} />
      </div>
    </div>
  )
}

// ============================================================================
// SnapshotDetailView — 快照/提炼详情页（header 固定返回 + body 独立滚动）
// ============================================================================

function SnapshotDetailView({
  memory,
  onBack,
  onDelete,
  onJumpToSession,
}: {
  memory?: UserMemory
  onBack: () => void
  onDelete: (id: string) => void
  onJumpToSession: (sessionId: string) => void
}) {
  const { t } = useLanguage()
  if (!memory) {
    return (
      <div className="ut-detail-page">
        <div className="ut-detail-header">
          <button className="ut-back-btn" onClick={onBack}>
            <IconChevronDown size={14} style={{ transform: 'rotate(90deg)' }} />
            <span>{t('memory.detail.back')}</span>
          </button>
        </div>
        <div className="ut-detail-body">
          <div className="ut-empty">{t('memory.exp.empty')}</div>
        </div>
      </div>
    )
  }

  const tagDisplay = memory.tags.filter(t => t !== 'check_pattern').slice(0, 5)
  const sourceBadge =
    memory.kind === 'snapshot'
      ? { label: t('memory.exp.tab.snapshot'), cls: 'kind-badge--distill' }
      : memory.kind === 'pattern'
        ? { label: t('memory.exp.tab.review'), cls: 'kind-badge--user-review' }
        : { label: t('memory.exp.tab.distill'), cls: 'kind-badge--system' }
  const body =
    memory.kind === 'pattern' ? (memory as any).pattern || memory.summary : memory.summary
  const title = (memory.intent || memory.summary || t('memory.noTitle')).substring(0, 80)

  return (
    <div className="ut-detail-page">
      <div className="ut-detail-header">
        <div className="ut-detail-header-row">
          <button className="ut-back-btn" onClick={onBack}>
            <IconChevronDown size={14} style={{ transform: 'rotate(90deg)' }} />
            <span>{t('memory.detail.back')}</span>
          </button>
          <div style={{ flex: 1 }} />
          {memory.session_id && (
            <button
              className="exp-action-btn exp-action-btn--primary"
              onClick={() => onJumpToSession(memory.session_id)}
            >
              {t('memory.exp.jumpToSession')}
            </button>
          )}
        </div>
        <div className="ut-detail-header-row">
          <span className={`kind-badge ${sourceBadge.cls}`}>{sourceBadge.label}</span>
          <div className="ut-detail-title">{title}</div>
        </div>
      </div>
      <div className="ut-detail-body">
        <div
          className="note-card-content note-card-content--mono"
          style={{ whiteSpace: 'pre-wrap' }}
        >
          {body}
        </div>
        {tagDisplay.length > 0 && (
          <div className="ut-memory-tags ut-memory-tags--row">
            {tagDisplay.map(t => (
              <span key={t} className="ann-tag ann-tag--sm">
                {t}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
