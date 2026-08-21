import { useEffect, useState } from 'react'
import {
  listCustomAgents,
  saveCustomAgent,
  deleteCustomAgent,
  getActiveCustomAgent,
  setActiveCustomAgent,
  getTools,
  type CustomAgentConfig,
} from '../lib/api'
import type { ToolSchema } from '../../core/types'
import { Button } from '../../ui/Button'
import { Section, FormRow } from '../../ui/PageLayout'
import { IconSparkles, IconPlus, IconTrash2, IconCheck } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import '../../styles/custom-agents.css'

/* 可识别的工具名前缀（name 第一段），其余归入 other */
const KNOWN_TOOL_PREFIXES = [
  'file',
  'browser',
  'desktop',
  'memory',
  'web',
  'system',
  'video',
  'skill',
  'workflow',
  'process',
]

interface Draft {
  /** 空串 = 新建（后端生成 id） */
  id: string
  name: string
  l2Prompt: string
  greeting: string
  /** 知识库路径，每行一条或逗号分隔 */
  knowledgeText: string
  /** 勾选的工具名集合；与全部工具一致时保存为空数组（不过滤） */
  checked: Set<string>
  /** 新建/加载时的原始 created_at（保存时透传） */
  createdAt: string
}

function draftFromAgent(agent: CustomAgentConfig, allTools: ToolSchema[]): Draft {
  const allowlist = agent.tools || []
  const checked =
    allowlist.length === 0
      ? new Set(allTools.map(tool => tool.name))
      : new Set(allTools.filter(tool => allowlist.includes(tool.name)).map(tool => tool.name))
  return {
    id: agent.id,
    name: agent.name,
    l2Prompt: agent.l2_prompt,
    greeting: agent.greeting,
    knowledgeText: (agent.knowledge || []).join('\n'),
    checked,
    createdAt: agent.created_at,
  }
}

function newDraft(allTools: ToolSchema[]): Draft {
  return {
    id: '',
    name: '',
    l2Prompt: '',
    greeting: '',
    knowledgeText: '',
    checked: new Set(allTools.map(tool => tool.name)),
    createdAt: '',
  }
}

function parseKnowledge(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map(line => line.trim())
    .filter(Boolean)
}

function groupTools(allTools: ToolSchema[]): Array<[string, ToolSchema[]]> {
  const buckets = new Map<string, ToolSchema[]>()
  for (const tool of allTools) {
    const prefix = tool.name.includes('_') ? tool.name.split('_')[0] : ''
    const key = KNOWN_TOOL_PREFIXES.includes(prefix) ? prefix : 'other'
    const bucket = buckets.get(key)
    if (bucket) bucket.push(tool)
    else buckets.set(key, [tool])
  }
  const ordered: Array<[string, ToolSchema[]]> = []
  for (const prefix of KNOWN_TOOL_PREFIXES) {
    const bucket = buckets.get(prefix)
    if (bucket) ordered.push([prefix, bucket])
  }
  const other = buckets.get('other')
  if (other) ordered.push(['other', other])
  return ordered
}

export function CustomAgentsPage({ onActivated }: {
  onClose: () => void
  /** 激活成功后回调（App 层切换到 Custom 模式 + 关闭页面，闭合「配置→使用」链路） */
  onActivated?: () => void
}) {
  const { t } = useLanguage()
  const [agents, setAgents] = useState<CustomAgentConfig[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [allTools, setAllTools] = useState<ToolSchema[]>([])
  const [draft, setDraft] = useState<Draft | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const [list, active, tools] = await Promise.all([
          listCustomAgents().catch(() => [] as CustomAgentConfig[]),
          getActiveCustomAgent().catch(() => null),
          getTools().catch(() => [] as ToolSchema[]),
        ])
        if (cancelled) return
        const agentList = list || []
        const toolList = tools || []
        setAgents(agentList)
        setActiveId(active?.id ?? null)
        setAllTools(toolList)
        // 默认选中激活的 Agent，否则选中第一张卡片
        const initial = agentList.find(agent => agent.id === active?.id) ?? agentList[0]
        if (initial) setDraft(draftFromAgent(initial, toolList))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const update = (patch: Partial<Draft>) => {
    setDraft(prev => (prev ? { ...prev, ...patch } : prev))
  }

  const selectAgent = (agent: CustomAgentConfig) => {
    setDraft(draftFromAgent(agent, allTools))
    setConfirmDelete(false)
  }

  const handleNew = () => {
    setDraft(newDraft(allTools))
    setConfirmDelete(false)
  }

  const toggleTool = (name: string) => {
    setDraft(prev => {
      if (!prev) return prev
      const checked = new Set(prev.checked)
      if (checked.has(name)) checked.delete(name)
      else checked.add(name)
      return { ...prev, checked }
    })
  }

  // 分组反选：该分类内已勾选的取消、未勾选的勾选
  const invertGroup = (tools: ToolSchema[]) => {
    setDraft(prev => {
      if (!prev) return prev
      const checked = new Set(prev.checked)
      tools.forEach(tool => {
        if (checked.has(tool.name)) checked.delete(tool.name)
        else checked.add(tool.name)
      })
      return { ...prev, checked }
    })
  }

  const buildConfig = (): CustomAgentConfig | null => {
    if (!draft) return null
    const allChecked = draft.checked.size === allTools.length
    return {
      id: draft.id,
      name: draft.name.trim(),
      l2_prompt: draft.l2Prompt,
      // 语义：空数组 = 全开不过滤；全选即等价于不过滤
      tools: allChecked
        ? []
        : allTools.filter(tool => draft.checked.has(tool.name)).map(tool => tool.name),
      greeting: draft.greeting,
      knowledge: parseKnowledge(draft.knowledgeText),
      created_at: draft.createdAt,
      updated_at: '',
    }
  }

  const refreshList = async (selectId?: string) => {
    const list = (await listCustomAgents().catch(() => [] as CustomAgentConfig[])) || []
    setAgents(list)
    if (selectId) {
      const target = list.find(agent => agent.id === selectId)
      if (target) setDraft(draftFromAgent(target, allTools))
    }
    return list
  }

  const handleSave = async () => {
    const config = buildConfig()
    if (!config || saving) return
    setSaving(true)
    try {
      const saved = await saveCustomAgent(config)
      if (!saved) throw new Error('save_custom_agent returned null')
      await refreshList(saved.id)
      setConfirmDelete(false)
    } catch (e) {
      console.error('Failed to save custom agent:', e)
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async () => {
    if (!draft?.id || saving) return
    setSaving(true)
    try {
      await deleteCustomAgent(draft.id)
      if (activeId === draft.id) {
        const active = await getActiveCustomAgent().catch(() => null)
        setActiveId(active?.id ?? null)
      }
      const list = await refreshList()
      setDraft(list.length > 0 ? draftFromAgent(list[0], allTools) : null)
      setConfirmDelete(false)
    } catch (e) {
      console.error('Failed to delete custom agent:', e)
    } finally {
      setSaving(false)
    }
  }

  const handleActivate = async () => {
    // 激活前先保存当前草稿，保证激活的是用户看到的配置
    const config = buildConfig()
    if (!config || saving) return
    setSaving(true)
    try {
      const saved = await saveCustomAgent(config)
      if (!saved) throw new Error('save_custom_agent returned null')
      await setActiveCustomAgent(saved.id)
      setActiveId(saved.id)
      await refreshList(saved.id)
      // 激活即进入：切换到 Custom 模式并关闭配置页，用户无需再手动切 mode
      onActivated?.()
    } catch (e) {
      console.error('Failed to activate custom agent:', e)
    } finally {
      setSaving(false)
    }
  }

  const groups = groupTools(allTools)
  const isEditingExisting = Boolean(draft?.id)
  const isActive = Boolean(draft?.id) && draft?.id === activeId

  return (
    <div className="custom-agents-page">
      <div className="custom-agents-layout">
        <aside className="custom-agents-list">
          <Button
            icon={<IconPlus size={13} />}
            onClick={handleNew}
            className="custom-agents-new-btn"
          >
            {t('custom.page.new')}
          </Button>
          {agents.map(agent => (
            <button
              key={agent.id}
              className={[
                'custom-agents-card',
                draft?.id === agent.id && 'selected',
              ]
                .filter(Boolean)
                .join(' ')}
              onClick={() => selectAgent(agent)}
            >
              <IconSparkles size={13} className="custom-agents-card-icon" />
              <span className="custom-agents-card-name">
                {agent.name || t('custom.page.untitled')}
              </span>
              {agent.id === activeId && (
                <span className="custom-agents-card-active">{t('custom.page.active')}</span>
              )}
            </button>
          ))}
          {!loading && agents.length === 0 && (
            <div className="custom-agents-empty">{t('custom.page.empty')}</div>
          )}
        </aside>

        <div className="custom-agents-form">
          {draft ? (
            <Section
              title={
                isEditingExisting
                  ? draft.name || t('custom.page.untitled')
                  : t('custom.page.new')
              }
            >
              <div className="custom-agents-note">{t('custom.page.lockedNote')}</div>

              <FormRow
                stacked
                label={t('custom.page.name')}
                control={
                  <input
                    className="input"
                    value={draft.name}
                    onChange={e => update({ name: e.target.value })}
                    placeholder={t('custom.page.namePlaceholder')}
                  />
                }
              />

              <FormRow
                stacked
                label={t('custom.page.l2')}
                control={
                  <textarea
                    className="textarea custom-agents-l2"
                    rows={12}
                    value={draft.l2Prompt}
                    onChange={e => update({ l2Prompt: e.target.value })}
                    placeholder={t('custom.page.l2Placeholder')}
                  />
                }
              />

              <FormRow
                stacked
                label={t('custom.page.greeting')}
                control={
                  <input
                    className="input"
                    value={draft.greeting}
                    onChange={e => update({ greeting: e.target.value })}
                    placeholder={t('custom.page.greetingPlaceholder')}
                  />
                }
              />

              <FormRow
                stacked
                label={t('custom.page.knowledge')}
                hint={t('custom.page.knowledgeHint')}
                control={
                  <textarea
                    className="textarea"
                    rows={3}
                    value={draft.knowledgeText}
                    onChange={e => update({ knowledgeText: e.target.value })}
                    placeholder={t('custom.page.knowledgePlaceholder')}
                  />
                }
              />

              <FormRow
                stacked
                label={t('custom.page.tools')}
                hint={t('custom.page.toolsHint')}
                control={
                  <div className="custom-agents-tools">
                    <div className="custom-agents-tools-toolbar">
                      <button
                        className="custom-agents-tools-action"
                        onClick={() => update({ checked: new Set(allTools.map(tool => tool.name)) })}
                      >
                        {t('custom.page.selectAll')}
                      </button>
                      <button
                        className="custom-agents-tools-action"
                        onClick={() => update({ checked: new Set() })}
                      >
                        {t('custom.page.clearAll')}
                      </button>
                    </div>
                    {groups.map(([key, tools]) => (
                      <div key={key} className="custom-agents-tool-group">
                        <div className="custom-agents-tool-group-title">
                          <span>{key === 'other' ? t('custom.page.group.other') : key}</span>
                          {/* 反选只在分类 ≥10 个工具时出现——小分类逐个勾选比反选更直观 */}
                          {tools.length >= 10 && (
                            <button
                              type="button"
                              className="custom-agents-tool-invert"
                              onClick={() => invertGroup(tools)}
                              title={t('custom.page.invertHint')}
                            >
                              {t('custom.page.invert')}
                            </button>
                          )}
                        </div>
                        <div className="custom-agents-tool-grid">
                          {tools.map(tool => (
                            <label
                              key={tool.name}
                              className="custom-agents-tool-item"
                              data-desc={tool.description}
                            >
                              <input
                                type="checkbox"
                                checked={draft.checked.has(tool.name)}
                                onChange={() => toggleTool(tool.name)}
                              />
                              <span>{tool.name}</span>
                            </label>
                          ))}
                        </div>
                      </div>
                    ))}
                  </div>
                }
              />

              <div className="form-footer">
                {isEditingExisting &&
                  (confirmDelete ? (
                    <>
                      <span className="custom-agents-delete-confirm">
                        {t('custom.page.deleteConfirm')}
                      </span>
                      <Button
                        variant="danger"
                        size="sm"
                        loading={saving}
                        onClick={handleDelete}
                      >
                        {t('common.delete')}
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setConfirmDelete(false)}>
                        {t('common.cancel')}
                      </Button>
                    </>
                  ) : (
                    <Button
                      variant="danger"
                      icon={<IconTrash2 size={13} />}
                      onClick={() => setConfirmDelete(true)}
                    >
                      {t('common.delete')}
                    </Button>
                  ))}
                <span className="custom-agents-footer-spacer" />
                {isEditingExisting && !isActive && (
                  <Button
                    icon={<IconCheck size={13} />}
                    loading={saving}
                    onClick={handleActivate}
                  >
                    {t('custom.page.setActive')}
                  </Button>
                )}
                <Button
                  variant="primary"
                  loading={saving}
                  disabled={!draft.name.trim()}
                  onClick={handleSave}
                >
                  {t('common.save')}
                </Button>
              </div>
            </Section>
          ) : (
            <div className="custom-agents-placeholder">
              <IconSparkles size={28} />
              <div>{loading ? t('custom.page.loading') : t('custom.page.placeholder')}</div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}