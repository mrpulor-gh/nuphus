// ChatAgentConfig.tsx — Chat Agent 配置页面
import { useState, useEffect, useCallback } from 'react'
import {
  listChatAgents,
  saveChatAgent,
  deleteChatAgent,
  setActiveChatAgent,
  getActiveChatAgent,
  listChatAgentsInline,
  updateChatAgentInline,
  listWorkflows,
  getCapabilities,
  setCapability,
} from '../lib/api'
import type { ChatAgentConfig as AgentT, InlineChatAgentEntry } from '../../core/types'
import {
  IconBot,
  IconEdit3,
  IconCheck,
  IconX,
  IconPlus,
  IconTrash2,
  IconStar,
} from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { Section, FormRow } from '../../ui/PageLayout'
import '../../styles/chat-agent.css'

interface Props {
  onClose: () => void
}

// ═══════════════ Agent 模板卡片 ═══════════════
function AgentCard({
  agent,
  active,
  onSave,
  onDelete,
  onSetActive,
}: {
  agent: AgentT
  active: boolean
  onSave: (cfg: AgentT) => Promise<void>
  onDelete: (name: string) => void
  onSetActive: (name: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const [del, setDel] = useState(false)
  const [saving, setSaving] = useState(false)
  const [id] = useState(agent.id)
  const [name, setName] = useState(agent.name)
  const [persona, setPersona] = useState(agent.persona || '')
  const [goal, setGoal] = useState(agent.goal || '')
  const [constraints, setConstr] = useState((agent.constraints || []).join('\n'))
  const [reqs, setReqs] = useState((agent.requirements || []).join('\n'))
  const [know, setKnow] = useState((agent.knowledge || []).join('\n'))
  const [maxIter, setMaxIter] = useState(agent.max_iterations || 15)

  const save = async () => {
    if (!name.trim()) return
    setSaving(true)
    await onSave({
      id,
      name: name.trim(),
      persona,
      goal: goal.trim() || undefined,
      constraints: constraints.split('\n').filter(l => l.trim()),
      requirements: reqs.split('\n').filter(l => l.trim()),
      knowledge: know.split('\n').filter(l => l.trim()),
      max_iterations: maxIter,
    })
    setSaving(false)
    setEditing(false)
  }

  return (
    <div className={`cac-card${active ? ' cac-active' : ''}`}>
      <div
        className="cac-card-header"
        onClick={editing ? () => setEditing(false) : undefined}
        style={editing ? { cursor: 'pointer' } : undefined}
      >
        <div className="cac-card-info">
          <div className="cac-card-name">
            {agent.name}
            {active && <span className="badge badge-accent cac-tag">Active</span>}
          </div>
          <div className="cac-card-id">id: {agent.id}</div>
          {!editing && agent.persona && (
            <div className="cac-card-desc">
              {agent.persona.slice(0, 80)}
              {agent.persona.length > 80 ? '…' : ''}
            </div>
          )}
          {!editing && (
            <div className="cac-card-meta">
              迭代 {agent.max_iterations}
              {agent.requirements.length > 0 ? ` · ${agent.requirements.length} 条要求` : ''}
              {agent.knowledge.length > 0 ? ` · ${agent.knowledge.length} 知识` : ''}
            </div>
          )}
        </div>
        <div className="cac-card-actions" onClick={e => e.stopPropagation()}>
          {del ? (
            <>
              <span className="cac-del-msg">确认删除？</span>
              <Button variant="primary" size="sm" onClick={() => onDelete(agent.name)}>
                确认
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setDel(false)}>
                取消
              </Button>
            </>
          ) : (
            <>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => onSetActive(agent.name)}
                disabled={active}
                title="设为 Active"
              >
                <IconStar size={12} />
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setEditing(true)} title="编辑">
                <IconEdit3 size={12} />
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setDel(true)} title="删除">
                <IconTrash2 size={12} />
              </Button>
            </>
          )}
        </div>
      </div>
      {editing && (
        <div className="cac-card-body">
          <FormRow
            stacked
            label="名称"
            control={
              <input
                className="compact-input"
                style={{ width: '100%' }}
                value={name}
                onChange={e => setName(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Persona（身份定义）"
            control={
              <textarea
                className="textarea"
                value={persona}
                rows={2}
                onChange={e => setPersona(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Goal（任务目标）"
            control={
              <textarea
                className="textarea"
                value={goal}
                rows={1}
                onChange={e => setGoal(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Constraints（约束条件，每行一条）"
            control={
              <textarea
                className="textarea"
                value={constraints}
                rows={2}
                onChange={e => setConstr(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Requirements（操作规范，每行一条）"
            control={
              <textarea
                className="textarea"
                value={reqs}
                rows={2}
                onChange={e => setReqs(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Knowledge"
            control={
              <textarea
                className="textarea"
                value={know}
                rows={1}
                onChange={e => setKnow(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="迭代轮数"
            control={
              <input
                className="compact-input input-num"
                style={{ width: '80px' }}
                type="number"
                min={1}
                value={maxIter}
                onChange={e => setMaxIter(parseInt(e.target.value) || 15)}
              />
            }
          />
          <div className="btn-row">
            <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
              取消
            </Button>
            <Button variant="primary" size="sm" disabled={!name.trim() || saving} onClick={save}>
              <IconCheck size={12} /> {saving ? '保存…' : '保存'}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

// ═══════════════ 新建 Agent 表单 ═══════════════
function CreateAgentForm({
  onSave,
  onCancel,
  saving,
}: {
  onSave: (cfg: AgentT) => void
  onCancel: () => void
  saving: boolean
}) {
  const [id, setId] = useState('')
  const [name, setName] = useState('')
  const [persona, setPersona] = useState('')
  const [goal, setGoal] = useState('')
  const [constr, setConstr] = useState('')
  const [reqs, setReqs] = useState('')
  const [know, setKnow] = useState('')
  const [maxIter, setMaxIter] = useState(15)

  return (
    <div className="cac-card cac-card-new">
      <div className="cac-card-header">
        <div className="cac-card-name">新建 Agent 模板</div>
      </div>
      <div className="cac-card-body">
        <FormRow
          stacked
          label="ID（唯一标识）"
          control={
            <input
              className="compact-input"
              style={{ width: '100%' }}
              value={id}
              placeholder="my-agent"
              onChange={e => setId(e.target.value)}
            />
          }
        />
        <FormRow
          stacked
          label="名称"
          control={
            <input
              className="compact-input"
              style={{ width: '100%' }}
              value={name}
              placeholder="My Agent"
              onChange={e => setName(e.target.value)}
            />
          }
        />
        <FormRow
          stacked
          label="Persona（身份定义）"
          control={
            <textarea
              className="textarea"
              value={persona}
              rows={2}
              placeholder="描述角色和行为方式"
              onChange={e => setPersona(e.target.value)}
            />
          }
        />
        <FormRow
          stacked
          label="Goal（任务目标）"
          control={
            <textarea
              className="textarea"
              value={goal}
              rows={1}
              placeholder="要达成什么"
              onChange={e => setGoal(e.target.value)}
            />
          }
        />
        <FormRow
          stacked
          label="Constraints（约束条件，每行一条）"
          control={
            <textarea
              className="textarea"
              value={constr}
              rows={2}
              onChange={e => setConstr(e.target.value)}
            />
          }
        />
        <FormRow
          stacked
          label="Requirements（操作规范，每行一条）"
          control={
            <textarea
              className="textarea"
              value={reqs}
              rows={2}
              onChange={e => setReqs(e.target.value)}
            />
          }
        />
        <FormRow
          stacked
          label="Knowledge（文件路径，每行一条）"
          control={
            <textarea
              className="textarea"
              value={know}
              rows={1}
              onChange={e => setKnow(e.target.value)}
            />
          }
        />
        <FormRow
          stacked
          label="迭代轮数"
          control={
            <input
              className="compact-input input-num"
              style={{ width: '80px' }}
              type="number"
              min={1}
              value={maxIter}
              onChange={e => setMaxIter(parseInt(e.target.value) || 15)}
            />
          }
        />
        <div className="btn-row">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            取消
          </Button>
          <Button
            variant="primary"
            size="sm"
            disabled={!id.trim() || !name.trim() || saving}
            onClick={() =>
              onSave({
                id: id.trim(),
                name: name.trim(),
                persona,
                goal: goal.trim() || undefined,
                constraints: constr.split('\n').filter(l => l.trim()),
                requirements: reqs.split('\n').filter(l => l.trim()),
                knowledge: know.split('\n').filter(l => l.trim()),
                max_iterations: maxIter,
              })
            }
          >
            <IconCheck size={12} /> {saving ? '创建…' : '创建'}
          </Button>
        </div>
      </div>
    </div>
  )
}

// ═══════════════ 内联步骤卡片 ═══════════════
function InlineStepCard({
  entry,
  agents,
  onSave,
}: {
  entry: InlineChatAgentEntry
  agents: AgentT[]
  onSave: (
    persona: string,
    goal: string,
    constraints: string,
    reqs: string,
    know: string,
    aid: string,
  ) => Promise<void>
}) {
  const { config } = entry
  const [editing, setEditing] = useState(false)
  const [saving, setSaving] = useState(false)
  const [persona, setPersona] = useState(config.persona || '')
  const [goal, setGoal] = useState(config.goal || '')
  const [constr, setConstr] = useState((config.constraints || []).join('\n'))
  const [reqs, setReqs] = useState((config.requirements || []).join('\n'))
  const [know, setKnow] = useState((config.knowledge || []).join('\n'))
  const [aid, setAid] = useState(config.agent_id || '')

  const save = async () => {
    setSaving(true)
    await onSave(persona, goal, constr, reqs, know, aid)
    setSaving(false)
    setEditing(false)
  }

  return (
    <div className="cac-inline-card">
      <div
        className="cac-inline-header"
        onClick={editing ? () => setEditing(false) : undefined}
        style={editing ? { cursor: 'pointer' } : undefined}
      >
        <div className="cac-inline-info">
          <div className="cac-inline-name">{entry.step_name}</div>
          <div className="cac-inline-id">step: {entry.step_id.slice(0, 16)}…</div>
          <div className="cac-inline-meta">工作流: {entry.workflow_name}</div>
        </div>
        <div className="cac-inline-actions" onClick={e => e.stopPropagation()}>
          {editing ? (
            <>
              <Button variant="ghost" size="sm" onClick={() => setEditing(false)} disabled={saving}>
                取消
              </Button>
              <Button variant="primary" size="sm" disabled={saving} onClick={save}>
                <IconCheck size={12} /> {saving ? '保存…' : '保存'}
              </Button>
            </>
          ) : (
            <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
              <IconEdit3 size={12} /> 编辑
            </Button>
          )}
        </div>
      </div>
      {editing ? (
        <div className="cac-card-body">
          <FormRow
            stacked
            label="引用模板（留空 = Active）"
            control={
              <select
                className="select"
                style={{ width: '100%' }}
                value={aid}
                onChange={e => setAid(e.target.value)}
              >
                <option value="">默认（Active）</option>
                {agents.map(a => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
            }
          />
          <FormRow
            stacked
            label="Persona 覆盖"
            control={
              <textarea
                className="textarea"
                value={persona}
                rows={2}
                onChange={e => setPersona(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Goal 覆盖"
            control={
              <textarea
                className="textarea"
                value={goal}
                rows={1}
                onChange={e => setGoal(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Constraints 覆盖"
            control={
              <textarea
                className="textarea"
                value={constr}
                rows={2}
                onChange={e => setConstr(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Requirements 覆盖"
            control={
              <textarea
                className="textarea"
                value={reqs}
                rows={2}
                onChange={e => setReqs(e.target.value)}
              />
            }
          />
          <FormRow
            stacked
            label="Knowledge 覆盖"
            control={
              <textarea
                className="textarea"
                value={know}
                rows={1}
                onChange={e => setKnow(e.target.value)}
              />
            }
          />
        </div>
      ) : (
        <div style={{ paddingTop: 'var(--space-2)' }}>
          <div className="cac-inline-meta">
            模板: {config.agent_id || 'Active'} · 迭代: {config.max_iterations ?? '默认'}
          </div>
          {config.persona && (
            <div className="cac-inline-desc">
              Persona: {config.persona.slice(0, 100)}
              {config.persona.length > 100 ? '…' : ''}
            </div>
          )}
          {(config.requirements || []).length > 0 && (
            <div className="cac-inline-meta">Requirements: {config.requirements!.length} 条</div>
          )}
          {(config.knowledge || []).length > 0 && (
            <div className="cac-inline-meta">Knowledge: {config.knowledge!.length} 文件</div>
          )}
        </div>
      )}
    </div>
  )
}

// ═══════════════ 页面主体 ═══════════════
export function ChatAgentConfig({ onClose }: Props) {
  const [agents, setAgents] = useState<AgentT[]>([])
  const [loading, setLoading] = useState(true)
  const [activeName, setActiveName] = useState<string | null>(null)
  const [showCreate, setShowCreate] = useState(false)
  const [creating, setCreating] = useState(false)

  const [entries, setEntries] = useState<InlineChatAgentEntry[]>([])
  const [entriesLoading, setEntriesLoading] = useState(true)

  const [defaultMaxIter, setDefaultMaxIter] = useState<number | null>(null)
  const [dmiSaving, setDmiSaving] = useState(false)
  const [dmiSaved, setDmiSaved] = useState(false)

  const loadAll = useCallback(async () => {
    setLoading(true)
    setEntriesLoading(true)
    try {
      const [list, wfs] = await Promise.all([listChatAgents(), listWorkflows()])
      setAgents(list ?? [])
      try {
        const a = await getActiveChatAgent()
        if (a) setActiveName(a.name)
      } catch {}
      const all: InlineChatAgentEntry[] = []
      for (const wf of wfs) {
        try {
          const r = await listChatAgentsInline(wf.id)
          if (Array.isArray(r)) all.push(...r)
        } catch {}
      }
      setEntries(all)
      try {
        const caps: any = await getCapabilities()
        if (caps) setDefaultMaxIter(caps.chat_agent_max_iterations ?? null)
      } catch {}
    } catch {
      setAgents([])
      setEntries([])
    } finally {
      setLoading(false)
      setEntriesLoading(false)
    }
  }, [])

  useEffect(() => {
    loadAll()
  }, [loadAll])

  const refresh = async () => {
    await loadAll()
    setShowCreate(false)
  }
  const handleSave = async (cfg: AgentT) => {
    try {
      await saveChatAgent(cfg)
      await refresh()
    } catch (e) {
      console.warn(e)
    }
  }
  const handleDelete = async (name: string) => {
    try {
      await deleteChatAgent(name)
      await refresh()
    } catch (e) {
      console.warn(e)
    }
  }
  const handleActive = async (name: string) => {
    try {
      const r = await setActiveChatAgent(name)
      if (r) setActiveName(r.name)
    } catch (e) {
      console.warn(e)
    }
  }
  const handleCreate = async (cfg: AgentT) => {
    setCreating(true)
    await handleSave(cfg)
    setCreating(false)
  }

  const saveInline = async (
    entry: InlineChatAgentEntry,
    persona: string,
    goal: string,
    constraints: string,
    reqs: string,
    know: string,
    aid: string,
  ) => {
    const u = {
      ...entry.config,
      persona,
      goal: goal.trim() || undefined,
      constraints: constraints.split('\n').filter(l => l.trim()),
      requirements: reqs.split('\n').filter(l => l.trim()),
      knowledge: know.split('\n').filter(l => l.trim()),
      agent_id: aid || undefined,
    }
    await updateChatAgentInline(entry.workflow_id, entry.step_id, u)
    setEntries(prev =>
      prev.map(e =>
        e.workflow_id === entry.workflow_id && e.step_id === entry.step_id
          ? { ...e, config: u }
          : e,
      ),
    )
  }

  const saveDMI = async () => {
    if (defaultMaxIter == null) return
    setDmiSaving(true)
    try {
      await setCapability('chat_agent_max_iterations', String(defaultMaxIter))
      setDmiSaved(true)
      setTimeout(() => setDmiSaved(false), 2000)
    } catch (e) {
      console.warn(e)
    } finally {
      setDmiSaving(false)
    }
  }

  return (
    <div className="page">
      <div className="page-toolbar">
        <div className="cac-title">
          <IconBot size={16} /> Chat Agent 配置
        </div>
        <div className="btn-row">
          <Button variant="ghost" size="sm" onClick={refresh}>
            刷新
          </Button>
          <Button variant="ghost" size="sm" onClick={onClose}>
            <IconX size={12} /> 返回
          </Button>
        </div>
      </div>

      {/* Agent 模板 */}
      <Section
        title="Agent 模板"
        description="跨工作流复用的配置。步骤通过 agent_id 引用，或设 Active 作为默认。"
        actions={
          <Button variant="primary" size="sm" onClick={() => setShowCreate(true)}>
            <IconPlus size={12} /> 新建
          </Button>
        }
      >
        {loading ? (
          <div className="page-loading">加载中…</div>
        ) : agents.length === 0 && !showCreate ? (
          <div className="page-empty">
            <IconBot size={28} />
            <div>暂无模板</div>
            <div className="page-empty-hint">点击「新建」创建</div>
          </div>
        ) : (
          <div className="cac-list">
            {showCreate && (
              <CreateAgentForm
                onSave={handleCreate}
                onCancel={() => setShowCreate(false)}
                saving={creating}
              />
            )}
            {agents.map(a => (
              <AgentCard
                key={a.id}
                agent={a}
                active={activeName === a.name}
                onSave={handleSave}
                onDelete={handleDelete}
                onSetActive={handleActive}
              />
            ))}
          </div>
        )}
      </Section>

      {/* 工作流步骤 */}
      <Section
        title="工作流步骤"
        description="来自各工作流 Talk 步骤的内联配置，修改直接写入 workflow.json。"
      >
        {entriesLoading ? (
          <div className="page-loading">加载中…</div>
        ) : entries.length === 0 ? (
          <div className="page-empty">
            <IconBot size={28} />
            <div>暂无步骤配置</div>
            <div className="page-empty-hint">工作流含 Talk 步骤后自动显示</div>
          </div>
        ) : (
          entries.map(e => (
            <InlineStepCard
              key={`${e.workflow_id}|${e.step_id}`}
              entry={e}
              agents={agents}
              onSave={(p, g, c, r, k, aid) => saveInline(e, p, g, c, r, k, aid)}
            />
          ))
        )}
      </Section>

      {/* 默认值 */}
      <Section
        title="默认最大推理轮数"
        description="所有步骤 max_iterations 的默认值（模板或步骤可覆盖）。"
      >
        <FormRow
          label="默认值"
          control={
            <>
              <input
                className="compact-input input-num input-w-sm"
                type="number"
                min={1}
                value={defaultMaxIter ?? ''}
                placeholder="15"
                onChange={e => {
                  const v = e.target.value
                  setDefaultMaxIter(v === '' ? null : parseInt(v) || null)
                }}
              />
              {dmiSaved && <span className="badge badge-success">已保存</span>}
              <Button
                variant="primary"
                size="sm"
                disabled={defaultMaxIter === null || dmiSaving}
                onClick={saveDMI}
              >
                {dmiSaving ? '保存…' : '保存'}
              </Button>
            </>
          }
        />
      </Section>
    </div>
  )
}
