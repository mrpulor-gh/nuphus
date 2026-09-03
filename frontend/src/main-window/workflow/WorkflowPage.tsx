// WorkflowPage.tsx — 工作流列表管理页面
// 搜索、列表展示、删除、触发创建弹窗

import { useState, useEffect, useCallback } from 'react'
import { listWorkflows, wfDelete, wfSave } from '../lib/api'
import { useWorkflowGate } from '../lib/useWorkflowGate'
import type { WorkflowItem } from '../../core/types'
import { IconSearch, IconTrash2, IconX, IconWorkflow, IconPlay, IconBot } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'
import { ChatAgentConfig } from './ChatAgentConfig'
import { LayoutDashboard } from 'lucide-react'

function formatTime(ts: number, t?: (key: string, ...args: string[]) => string): string {
  try {
    const d = new Date(ts * 1000)
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

const statusLabel: Record<string, { textKey: string; badge: string }> = {
  draft: { textKey: 'workflow.status.draft', badge: 'badge badge-neutral' },
  active: { textKey: 'workflow.status.active', badge: 'badge badge-accent' },
  archived: { textKey: 'workflow.status.archived', badge: 'badge badge-neutral' },
}

interface WorkflowPageProps {
  onClose: () => void
  onRunClick: (workflow: WorkflowItem) => void
  /** 打开节点画布（Pro 门禁在调用方与本组件双重检查） */
  onCanvasClick: (workflow: WorkflowItem) => void
}

export function WorkflowPage({ onClose, onRunClick, onCanvasClick }: WorkflowPageProps) {
  const { t } = useLanguage()
  // ── 全局执行闸门（大王铁律：任意执行态禁止启动工作流 / 进入画布）──
  const gate = useWorkflowGate()
  const gateLocked = gate.locked
  const gateRefresh = gate.refresh
  const gateLockNotice =
    gate.reason === 'workflow' ? '工作流正在执行中，暂不可用！' : '当前有任务执行中，暂不可用！'
  const [items, setItems] = useState<WorkflowItem[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [showChatAgent, setShowChatAgent] = useState(false)
  // ── 画布新建：创建中防重复点击 ──
  const [canvasCreating, setCanvasCreating] = useState(false)

  // 行内「运行 / 画布」入口点击级复核（禁用态之外收窄轮询竞态窗口）
  const requestRun = useCallback(
    async (item: WorkflowItem) => {
      const cur = await gateRefresh()
      if (cur.locked) {
        setError(
          cur.reason === 'workflow'
            ? '工作流正在执行中，暂不可用！'
            : '当前有任务执行中，暂不可用！',
        )
        return
      }
      onRunClick(item)
    },
    [gateRefresh, onRunClick],
  )

  const requestCanvas = useCallback(
    async (item: WorkflowItem) => {
      const cur = await gateRefresh()
      if (cur.locked) {
        setError(
          cur.reason === 'workflow'
            ? '工作流正在执行中，暂不可用！'
            : '当前有任务执行中，暂不可用！',
        )
        return
      }
      onCanvasClick(item)
    },
    [gateRefresh, onCanvasClick],
  )

  const loadData = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const workflows = await listWorkflows().catch(() => [])
      setItems(workflows || [])
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadData()
  }, [loadData])

  const filtered = useCallback(() => {
    if (!searchQuery.trim()) return items
    const q = searchQuery.toLowerCase()
    return items.filter(
      item =>
        item.title.toLowerCase().includes(q) ||
        item.description?.toLowerCase().includes(q) ||
        item.tags.some(t => t.toLowerCase().includes(q)),
    )
  }, [items, searchQuery])

  const [deleting, setDeleting] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null)

  const handleDelete = useCallback(async (id: string) => {
    setDeleting(id)
    try {
      await wfDelete(id)
      setItems(prev => prev.filter(i => i.id !== id))
      setConfirmDelete(null)
    } catch (e) {
      setError(String(e))
    } finally {
      setDeleting(null)
    }
  }, [])

  const results = filtered()

  // ── 画布新建：直建空白工作流并打开画布 ──
  const handleCanvasNew = useCallback(async () => {
    if (canvasCreating) return
    // 闸门点击级复核（轮询窗口内竞态收口；后端 wf_gate/rec 另有兜底）
    const cur = await gateRefresh()
    if (cur.locked) {
      setError(
        cur.reason === 'workflow' ? '工作流正在执行中，暂不可用！' : '当前有任务执行中，暂不可用！',
      )
      return
    }
    setCanvasCreating(true)
    setError(null)
    try {
      // status 必须 PascalCase：后端 WorkflowStatus serde 枚举无 rename（types.rs）
      // wf_save 为 upsert，空 steps 仅 warning 不阻断（compiler.rs）
      const id = crypto.randomUUID()
      const resp = await wfSave({
        id,
        name: '未命名工作流',
        status: 'Draft',
        steps: [],
        doc: null,
        schedule: null,
        run_history: [],
        dry_run: false,
      })
      if (!resp) {
        setError('工作流创建失败：后端无响应')
        return
      }
      if (!resp.saved) {
        setError(resp.report.errors.join('；') || '工作流创建失败')
        return
      }
      const now = Math.floor(Date.now() / 1000)
      const item: WorkflowItem = {
        id,
        title: '未命名工作流',
        description: '',
        steps: [],
        tags: [],
        created_at: now,
        updated_at: now,
        run_count: 0,
        status: 'draft',
        schedule: null,
        run_history: [],
        timeout_secs: null,
        dry_run: false,
        doc: null,
      }
      setItems(prev => [item, ...prev])
      onCanvasClick(item)
    } catch (e) {
      setError(String(e))
    } finally {
      setCanvasCreating(false)
    }
  }, [canvasCreating, gateRefresh, onCanvasClick])

  // ── Chat Agent 配置模式 ──
  if (showChatAgent) {
    return <ChatAgentConfig onClose={() => setShowChatAgent(false)} />
  }

  return (
    <div className="page">
      <div className="page-toolbar">
        <div className="page-search">
          <IconSearch size={14} />
          <input
            placeholder={t('workflow.search')}
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
          />
        </div>
        <Button variant="default" onClick={() => setShowChatAgent(true)} title="Chat Agent">
          <IconBot size={12} /> Chat Agent
        </Button>
        {/* 画布新建入口（闸门锁定态禁用：执行中禁止进入画布） */}
        <Button
          variant="default"
          onClick={() => void handleCanvasNew()}
          loading={canvasCreating}
          disabled={gateLocked}
          title={gateLocked ? gateLockNotice : '新建空白工作流并直接在画布中编排'}
        >
          <LayoutDashboard size={12} /> 画布新建
        </Button>
      </div>

      {/* ── 全局执行闸门锁定提示 ── */}
      {gateLocked && (
        <div className="gate-banner">
          <span>{gateLockNotice}</span>
        </div>
      )}

      {/* ── 错误提示 ── */}
      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <Button variant="ghost" size="sm" onClick={() => setError(null)}>
            <IconX size={12} />
          </Button>
        </div>
      )}

      {/* ── 加载中 ── */}
      {loading && <div className="page-loading">{t('common.loading')}</div>}

      {/* ── 空状态 ── */}
      {!loading && results.length === 0 && (
        <div className="page-empty">
          <IconWorkflow size={32} />
          <div>{searchQuery ? t('workflow.noResults') : t('workflow.empty')}</div>
          <div className="page-empty-hint">
            {searchQuery ? t('workflow.tryOtherKeywords') : t('workflow.clickToCreate')}
          </div>
        </div>
      )}

      {/* ── 列表 ── */}
      {!loading && results.length > 0 && (
        <div className="page-list">
          {results.map(item => {
            const st = statusLabel[item.status] || statusLabel.draft
            // meta 归并单行：步数 · 运行次数 · #tags · 更新时间
            const metaLine = [
              t('workflow.steps', String(item.steps.length)),
              t('workflow.runCount', String(item.run_count)),
              ...item.tags.map(tag => `#${tag}`),
              formatTime(item.updated_at, t),
            ].join(' · ')
            return (
              <div key={item.id} className="page-list-item">
                <div className="item-row">
                  <div className="item-main">
                    <div className="item-head">
                      <span className="item-title">{item.title}</span>
                      <span className={st.badge}>{t(st.textKey)}</span>
                    </div>
                    {item.description && <div className="item-desc">{item.description}</div>}
                    <div className="item-meta-line">{metaLine}</div>
                  </div>
                  <div className="item-actions">
                    <IconButton
                      variant="default"
                      label={t('workflow.run')}
                      onClick={() => void requestRun(item)}
                      disabled={gateLocked}
                      title={gateLocked ? gateLockNotice : t('workflow.run')}
                      className="wf-action-run"
                    >
                      <IconPlay size={14} />
                    </IconButton>
                    {/* 画布入口（闸门锁定态禁用：执行中禁止进入画布） */}
                    <IconButton
                      variant="ghost"
                      label={t('workflow.canvas')}
                      onClick={() => void requestCanvas(item)}
                      disabled={gateLocked}
                      title={gateLocked ? gateLockNotice : t('workflow.canvas')}
                    >
                      <LayoutDashboard size={14} />
                    </IconButton>
                    {confirmDelete === item.id ? (
                      <>
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => handleDelete(item.id)}
                          disabled={deleting === item.id}
                        >
                          {deleting === item.id ? '...' : t('common.confirmDelete')}
                        </Button>
                        <Button variant="default" size="sm" onClick={() => setConfirmDelete(null)}>
                          {t('common.cancel')}
                        </Button>
                      </>
                    ) : (
                      <IconButton
                        variant="ghost"
                        label={t('common.delete')}
                        onClick={() => setConfirmDelete(item.id)}
                        className="delete-btn"
                      >
                        <IconTrash2 size={14} />
                      </IconButton>
                    )}
                  </div>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
