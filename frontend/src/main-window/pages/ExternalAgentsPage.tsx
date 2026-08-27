import { useEffect, useState } from 'react'
import { EXT_AGENT_PINNED_EVENT } from '../chat/ExternalAgentsStatusBar'
import {
  listExternalAgents,
  upsertExternalAgent,
  deleteExternalAgent,
  extractAgentIcon,
  type ExternalAgentConfig,
} from '../lib/api'
import { open } from '@tauri-apps/plugin-dialog'
import { Button } from '../../ui/Button'
import { Section, FormRow } from '../../ui/PageLayout'
import {
  IconPlus,
  IconX,
} from '../../ui/Icons'
import {
  AgentIconAuto,
  agentKind,
  exePathFromOpen,
  iconSourcePath,
  iconUrlCache,
  isIconPath,
} from '../components/AgentIconAuto'
import { useLanguage } from '../../locales'
import '../../styles/external-agents.css'

/** 图标策略：不提供手选 lucide 图标（避免一大堆头像选择）。
 *  - auto（默认）：优先从 open/process 提取应用图标；无则按 agent 名推断类型图标（CLI→终端 / 其他→bot）
 *  - 文件路径：自定义图标文件（pickCustomIcon）
 *  - 旧配置中的 lucide 名（bot/terminal/cpu/...）仍兼容渲染（AgentIcon switch），但 UI 不再提供选择
 */
const MODE_OPTIONS = ['background', 'embedded', 'standalone', 'web'] as const

/** key 白名单：与后端 validate_agent 语义一致（[a-zA-Z0-9_-]，禁 '.'/':'） */
const KEY_RE = /^[a-zA-Z0-9_-]+$/

interface Draft {
  key: string
  display_name: string
  icon: string
  dir: string
  mode: string
  open: string
  args: string
  process: string
  description: string
  // ── v8 交互固化（高级分组，折叠）──
  launch: string
  window_hint: string
  cooldown_secs: number
  dispatch_steps_json: string
  await_timeout_secs: number
  timeout_action: string
  timeout_script: string
  auto_approve: string
  auto_approve_script: string
  confirm_keywords_csv: string
}

/** dispatch_steps JSON 数组 → 缩进文本（编辑域用） */
function stepsToJson(steps?: ExternalAgentConfig['dispatch_steps']): string {
  if (!steps || steps.length === 0) return ''
  return JSON.stringify(steps, null, 2)
}

/** 解析 JSON 文本域为 dispatch_steps 数组；非法 JSON → 抛错（保存时提示） */
function parseStepsJson(text: string): Array<{ tool: string; with?: Record<string, unknown> }> {
  const trimmed = text.trim()
  if (!trimmed) return []
  const parsed = JSON.parse(trimmed) as unknown
  if (!Array.isArray(parsed)) throw new Error('dispatch_steps 必须是 JSON 数组')
  for (const step of parsed) {
    const s = step as Record<string, unknown>
    if (typeof s?.tool !== 'string' || !s.tool.trim()) throw new Error('每步必须含 tool 字段')
  }
  return parsed as Array<{ tool: string; with?: Record<string, unknown> }>
}

/** confirm_keywords 数组 → 逗号分隔文本 */
function keywordsToCsv(keywords?: string[]): string {
  return (keywords || []).join(', ')
}

/** 逗号分隔文本 → 数组（去空白，过滤空项） */
function parseKeywordsCsv(text: string): string[] {
  return text
    .split(',')
    .map(s => s.trim())
    .filter(Boolean)
}

function newDraft(): Draft {
  return {
    key: '',
    display_name: '',
    icon: 'auto',
    dir: '',
    mode: 'embedded',
    open: '',
    args: '',
    process: '',
    description: '',
    launch: '',
    window_hint: '',
    cooldown_secs: 120,
    dispatch_steps_json: '',
    await_timeout_secs: 120,
    timeout_action: 'detect_confirm',
    timeout_script: '',
    auto_approve: '',
    auto_approve_script: '',
    confirm_keywords_csv: '',
  }
}

function draftFromAgent(a: ExternalAgentConfig): Draft {
  return {
    key: a.key,
    display_name: a.display_name || '',
    icon: a.icon || 'auto',
    dir: a.dir || '',
    mode: a.mode || 'embedded',
    open: a.open || '',
    args: a.args || '',
    process: a.process || '',
    description: a.description || '',
    launch: a.launch || '',
    window_hint: a.window_hint || '',
    cooldown_secs: a.cooldown_secs ?? 120,
    dispatch_steps_json: stepsToJson(a.dispatch_steps),
    await_timeout_secs: a.await_timeout_secs ?? 120,
    timeout_action: a.timeout_action || 'detect_confirm',
    timeout_script: a.timeout_script || '',
    auto_approve: a.auto_approve || '',
    auto_approve_script: a.auto_approve_script || '',
    confirm_keywords_csv: keywordsToCsv(a.confirm_keywords),
  }
}

/**
 * 外部 Agent 配置中心：plugin/team.toml 的 CRUD 页面。
 * - 顶部横向 chips（切换 / 删除 / + 新增配置）
 * - 表单：ICON / Agent 名称 / 启动路径(+args) / 窗口运行模式(4值) / 进程名 / 工作描述
 * - 保存 = upsertExternalAgent（新 agent 时后端联动 agent_init 生成 handoff 目录）
 */
export function ExternalAgentsPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [agents, setAgents] = useState<ExternalAgentConfig[]>([])
  const [selectedKey, setSelectedKey] = useState<string>('new')
  const [draft, setDraft] = useState<Draft>(newDraft)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [toast, setToast] = useState('')
  // ── 高级分组「交互固化」折叠状态 ──
  const [advancedOpen, setAdvancedOpen] = useState(false)
  // ── 应用图标提取状态（icon=auto 或显式路径时的加载/失败提示；预览由 AgentIconAuto 内部渲染）──
  const [iconLoading, setIconLoading] = useState(false)
  const [iconError, setIconError] = useState('')

  const update = (patch: Partial<Draft>) => setDraft(prev => ({ ...prev, ...patch }))

  const showToast = (msg: string) => {
    setToast(msg)
    window.setTimeout(() => setToast(''), 2500)
  }

  // auto/路径 → 自动提取应用图标（预览由 AgentIconAuto 内部渲染，这里仅跟踪加载/失败提示）
  useEffect(() => {
    let cancelled = false
    const srcRaw = iconSourcePath(draft)
    const src: string = srcRaw ?? ''
    if (!src) {
      setIconLoading(false)
      setIconError('')
      return
    }
    if (iconUrlCache.has(src)) {
      setIconLoading(false)
      return
    }
    setIconLoading(true)
    setIconError('')
    extractAgentIcon(src)
      .then(() => {
        /* 预览由 AgentIconAuto 渲染（带缓存），这里只清除错误态 */
        if (!cancelled) setIconError('')
      })
      .catch(() => {
        if (!cancelled) setIconError(t('extAgents.cfg.iconExtractFail'))
      })
      .finally(() => {
        if (!cancelled) setIconLoading(false)
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft.icon, draft.open, draft.process])

  const pickCustomIcon = async () => {
    try {
      const sel = await open({
        title: t('extAgents.cfg.iconCustomTitle'),
        multiple: false,
        directory: false,
        filters: [
          {
            name: 'Icon / Image',
            extensions: ['png', 'jpg', 'jpeg', 'ico', 'webp', 'gif', 'bmp', 'exe'],
          },
        ],
      })
      if (typeof sel === 'string') {
        update({ icon: sel })
      }
    } catch {
      /* dialog 取消或不可用 */
    }
  }

  const iconAutoActive = draft.icon === 'auto'
  const iconCustomActive = isIconPath(draft.icon)
  const iconAutoHint = exePathFromOpen(draft.open) || exePathFromOpen(draft.process) || ''

  const refreshList = async (selectKey: string | null) => {
    const list = (await listExternalAgents().catch(() => [] as ExternalAgentConfig[])) || []
    setAgents(list)
    if (selectKey) {
      const found = list.find(a => a.key === selectKey)
      if (found) {
        setSelectedKey(found.key)
        setDraft(draftFromAgent(found))
      } else {
        setSelectedKey('new')
        setDraft(newDraft())
      }
    }
  }

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const list = (await listExternalAgents().catch(() => [] as ExternalAgentConfig[])) || []
        if (cancelled) return
        setAgents(list)
        const initial = list[0]
        if (initial) {
          setSelectedKey(initial.key)
          setDraft(draftFromAgent(initial))
        }
      } catch {
        if (!cancelled) setError(t('extAgents.cfg.loadFail'))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const isNew = !agents.some(a => a.key === selectedKey)

  const handleSelect = (agent: ExternalAgentConfig) => {
    setSelectedKey(agent.key)
    setDraft(draftFromAgent(agent))
    setError('')
  }

  const handleNew = () => {
    setSelectedKey('new')
    setDraft(newDraft())
    setError('')
  }

  const handleSave = async () => {
    if (!draft.key.trim()) {
      setError(t('extAgents.cfg.keyRequired'))
      return
    }
    if (!KEY_RE.test(draft.key)) {
      setError(t('extAgents.cfg.keyInvalid'))
      return
    }
    if (!draft.display_name.trim()) {
      setError(t('extAgents.cfg.nameRequired'))
      return
    }
    setSaving(true)
    setError('')
    try {
      // 高级字段：dispatch_steps JSON 解析（非法则中止保存并提示）
      let dispatchSteps: Array<{ tool: string; with?: Record<string, unknown> }> = []
      try {
        dispatchSteps = parseStepsJson(draft.dispatch_steps_json)
      } catch (e) {
        setSaving(false)
        setError(
          `${t('extAgents.cfg.dispatchSteps')}: ${
            e instanceof Error ? e.message : String(e)
          }`,
        )
        return
      }
      await upsertExternalAgent({
        key: draft.key,
        display_name: draft.display_name.trim(),
        icon: draft.icon,
        dir: draft.dir.trim(),
        mode: draft.mode,
        open: draft.open,
        args: draft.args,
        process: draft.process,
        description: draft.description,
        launch: draft.launch.trim(),
        window_hint: draft.window_hint.trim(),
        cooldown_secs: draft.cooldown_secs,
        dispatch_steps: dispatchSteps,
        await_timeout_secs: draft.await_timeout_secs,
        timeout_action: draft.timeout_action,
        timeout_script: draft.timeout_script.trim(),
        auto_approve: draft.auto_approve.trim(),
        auto_approve_script: draft.auto_approve_script.trim(),
        confirm_keywords: parseKeywordsCsv(draft.confirm_keywords_csv),
      })
      // pin：用户显式添加的 agent 在应用生命周期内常驻状态栏
      // （内存态，随启动清零；CustomEvent 通知状态栏立即生效）
      window.dispatchEvent(new CustomEvent(EXT_AGENT_PINNED_EVENT, { detail: draft.key }))
      await refreshList(draft.key)
      showToast(t('extAgents.cfg.saved'))
    } catch (e) {
      console.error('Failed to save external agent:', e)
      setError(t('extAgents.cfg.saveFail'))
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async (key: string) => {
    if (!window.confirm(t('extAgents.cfg.deleteConfirm'))) return
    setSaving(true)
    setError('')
    try {
      await deleteExternalAgent(key)
      await refreshList(null)
      showToast(t('extAgents.cfg.deleted'))
    } catch (e) {
      console.error('Failed to delete external agent:', e)
      setError(t('extAgents.cfg.deleteFail'))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="ext-agents-page">
      {/* ── 顶部横向 chips：+ 新增配置 + 各 agent（切换 / 删除）── */}
      <div className="ext-agents-chips">
        <button type="button" className="ext-agents-add-chip" onClick={handleNew}>
          <IconPlus size={13} />
          <span>{t('extAgents.cfg.new')}</span>
        </button>
        {agents.map(agent => (
          <span
            key={agent.key}
            className={['ext-agents-chip', selectedKey === agent.key && 'active']
              .filter(Boolean)
              .join(' ')}
          >
            <button
              type="button"
              className="ext-agents-chip-main"
              onClick={() => handleSelect(agent)}
            >
              {/* 头像与编辑区/状态栏同源：AgentIconAuto 按配置渲染真实头像（auto→提取应用图标 / 路径→自定义图） */}
              <span className="ext-agents-chip-avatar" aria-hidden>
                <span className={`agent-avatar-icon is-${agentKind(agent.key)}`}>
                  <AgentIconAuto
                    icon={agent.icon || 'auto'}
                    size={15}
                    avatarSize={18}
                    name={agent.key}
                    open={agent.open || ''}
                    process={agent.process || ''}
                  />
                </span>
              </span>
              <span className="ext-agents-chip-name" title={agent.display_name || agent.key}>
                {agent.key}
              </span>
            </button>
            <button
              type="button"
              className="ext-agents-chip-del"
              title={t('common.delete')}
              aria-label={`${t('common.delete')} ${agent.display_name || agent.key}`}
              onClick={() => handleDelete(agent.key)}
            >
              <IconX size={11} />
            </button>
          </span>
        ))}
        {!loading && agents.length === 0 && (
          <div className="ext-agents-chips-empty">{t('extAgents.cfg.empty')}</div>
        )}
      </div>

      {/* ── 表单 ── */}
      <div className="ext-agents-form">
        <Section
          title={isNew ? t('extAgents.cfg.newTitle') : draft.display_name || draft.key || '—'}
        >
          {/* ── 身份区：ICON 预览与 名称/标识 一体化（替代原先分散的三行）──
              左侧大头像随 icon/启动路径 实时更新；右侧名称在上、标识在下，
              底部一行是图标来源操作（自动提取 / 自定义文件） */}
          <div className="ext-agents-identity">
            <div className="ext-agents-identity-side">
              <span className="ext-agents-identity-avatar" aria-hidden>
                <AgentIconAuto
                  icon={draft.icon}
                  size={28}
                  avatarSize={54}
                  name={draft.key || draft.display_name}
                  open={draft.open}
                  process={draft.process}
                />
              </span>
              <div className="ext-agents-identity-iconrow">
                <button
                  type="button"
                  className={['ext-agents-icon-act', iconAutoActive && 'active']
                    .filter(Boolean)
                    .join(' ')}
                  title={
                    iconAutoHint
                      ? `${t('extAgents.cfg.iconAuto')}: ${iconAutoHint}`
                      : t('extAgents.cfg.iconAutoEmpty')
                  }
                  onClick={() => update({ icon: 'auto' })}
                >
                  {t('extAgents.cfg.iconAuto')}
                </button>
                <button
                  type="button"
                  className={['ext-agents-icon-act', iconCustomActive && 'active']
                    .filter(Boolean)
                    .join(' ')}
                  title={t('extAgents.cfg.iconCustom')}
                  onClick={pickCustomIcon}
                >
                  {t('extAgents.cfg.iconCustom')}
                </button>
              </div>
            </div>
            <div className="ext-agents-identity-main">
              <div className="ext-agents-identity-field">
                <span className="ext-agents-identity-label">
                  {t('extAgents.cfg.name')} · {t('extAgents.cfg.icon')}
                </span>
                <input
                  className="input"
                  value={draft.display_name}
                  onChange={e => update({ display_name: e.target.value })}
                  placeholder={t('extAgents.cfg.namePlaceholder')}
                />
              </div>
              <div className="ext-agents-identity-field">
                <span
                  className="ext-agents-identity-label"
                  title={t('extAgents.cfg.dirHint')}
                >
                  {t('extAgents.cfg.dir')}
                </span>
                <input
                  className="input"
                  value={draft.dir}
                  onChange={e => update({ dir: e.target.value })}
                  placeholder={t('extAgents.cfg.dirPlaceholder')}
                />
              </div>
              {iconAutoActive && !iconAutoHint && !iconLoading && (
                <div className="ext-agents-icon-hint">{t('extAgents.cfg.iconAutoEmpty')}</div>
              )}
              {iconAutoActive && iconError && (
                <div className="ext-agents-icon-hint ext-agents-icon-hint--error">{iconError}</div>
              )}
            </div>
          </div>
        </Section>

        {/* ── Agent 配置（技术参数，默认收起；其余细节交由用户按需展开配置）── */}
        <Section
          className="ext-agents-advanced"
          title={
            <button
              type="button"
              className="ext-agents-advanced-toggle"
              onClick={() => setAdvancedOpen(v => !v)}
              aria-expanded={advancedOpen}
            >
              <span className="ext-agents-advanced-caret">{advancedOpen ? '▾' : '▸'}</span>
              {t('extAgents.cfg.agentConfig')}
            </button>
          }
          description={t('extAgents.cfg.agentConfigHint')}
        >
          {advancedOpen && (
            <>
          <FormRow
            stacked
            label={t('extAgents.cfg.key')}
            hint={t('extAgents.cfg.keyHint')}
            control={
              <input
                className="input"
                value={draft.key}
                disabled={!isNew}
                onChange={e => update({ key: e.target.value.trim() })}
                placeholder="e.g. claude-code"
              />
            }
          />
          <FormRow
            stacked
            label={t('extAgents.cfg.open')}
            hint={t('extAgents.cfg.openHint')}
            control={
              <input
                className="input"
                value={draft.open}
                onChange={e => update({ open: e.target.value })}
                placeholder="e.g. C:\\...\\claude.exe 或 终端执行 xxx"
              />
            }
          />
          <FormRow
            stacked
            label={t('extAgents.cfg.args')}
            hint={t('extAgents.cfg.argsHint')}
            control={
              <input
                className="input"
                value={draft.args}
                onChange={e => update({ args: e.target.value })}
                placeholder="--flag1 --flag2"
              />
            }
          />
          <FormRow
            stacked
            label={t('extAgents.cfg.mode')}
            control={
              <select
                className="select"
                value={draft.mode}
                onChange={e => update({ mode: e.target.value })}
              >
                {MODE_OPTIONS.map(m => (
                  <option key={m} value={m}>
                    {t(`extAgents.cfg.mode.${m}`)}
                  </option>
                ))}
              </select>
            }
          />
          <FormRow
            stacked
            label={t('extAgents.cfg.process')}
            hint={t('extAgents.cfg.processHint')}
            control={
              <input
                className="input"
                value={draft.process}
                onChange={e => update({ process: e.target.value })}
                placeholder="e.g. claude.exe"
              />
            }
          />
          <FormRow
            stacked
            label={t('extAgents.cfg.description')}
            hint={t('extAgents.cfg.descriptionHint')}
            control={
              <textarea
                className="textarea"
                rows={3}
                value={draft.description}
                onChange={e => update({ description: e.target.value })}
                placeholder={t('extAgents.cfg.descriptionPlaceholder')}
              />
            }
          />
          {/* ── v8 交互固化字段 ── */}
          <FormRow
            stacked
            label={t('extAgents.cfg.launch')}
                hint={t('extAgents.cfg.launchHint')}
                control={
                  <input
                    className="input"
                    value={draft.launch}
                    onChange={e => update({ launch: e.target.value })}
                    placeholder="wt.exe -p PowerShell opencode"
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.windowHint')}
                hint={t('extAgents.cfg.windowHintField')}
                control={
                  <input
                    className="input"
                    value={draft.window_hint}
                    onChange={e => update({ window_hint: e.target.value })}
                    placeholder="opencode"
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.cooldown')}
                control={
                  <input
                    className="input"
                    type="number"
                    min={0}
                    value={draft.cooldown_secs}
                    onChange={e => update({ cooldown_secs: Number(e.target.value) || 0 })}
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.dispatchSteps')}
                hint={
                  <span className="ext-agents-steps-hint">
                    {t('extAgents.cfg.dispatchStepsTools')}
                    <br />
                    {t('extAgents.cfg.dispatchStepsPlaceholders')}
                  </span>
                }
                control={
                  <textarea
                    className="textarea ext-agents-steps-json"
                    rows={10}
                    spellCheck={false}
                    value={draft.dispatch_steps_json}
                    onChange={e => update({ dispatch_steps_json: e.target.value })}
                    placeholder={t('extAgents.cfg.dispatchStepsPlaceholder')}
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.awaitTimeout')}
                control={
                  <input
                    className="input"
                    type="number"
                    min={0}
                    value={draft.await_timeout_secs}
                    onChange={e => update({ await_timeout_secs: Number(e.target.value) || 0 })}
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.timeoutAction')}
                hint={t('extAgents.cfg.timeoutActionHint')}
                control={
                  <select
                    className="select"
                    value={draft.timeout_action}
                    onChange={e => update({ timeout_action: e.target.value })}
                  >
                    {['detect_confirm', 'screenshot_alive', 'notify_user', 'redeliver'].map(a => (
                      <option key={a} value={a}>
                        {a}
                      </option>
                    ))}
                    <option value="timeout_script">timeout_script（自定义）</option>
                  </select>
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.timeoutScript')}
                hint={t('extAgents.cfg.timeoutScriptHint')}
                control={
                  <input
                    className="input"
                    value={draft.timeout_script}
                    onChange={e => update({ timeout_script: e.target.value })}
                    placeholder="D:/policies/timeout.ps1"
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.autoApprove')}
                hint={t('extAgents.cfg.autoApproveHint')}
                control={
                  <input
                    className="input"
                    value={draft.auto_approve}
                    onChange={e => update({ auto_approve: e.target.value })}
                    placeholder="yes"
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.autoApproveScript')}
                control={
                  <input
                    className="input"
                    value={draft.auto_approve_script}
                    onChange={e => update({ auto_approve_script: e.target.value })}
                    placeholder="D:/policies/approve.ps1"
                  />
                }
              />
              <FormRow
                stacked
                label={t('extAgents.cfg.confirmKeywords')}
                hint={t('extAgents.cfg.confirmKeywordsHint')}
                control={
                  <input
                    className="input"
                    value={draft.confirm_keywords_csv}
                    onChange={e => update({ confirm_keywords_csv: e.target.value })}
                    placeholder="allow, confirm, proceed, yes/no"
                  />
                }
              />
            </>
          )}
        </Section>
      </div>

      {/* ── 状态反馈 ── */}
      {error && <div className="ext-agents-error">{error}</div>}
      {toast && <div className="ext-agents-toast">{toast}</div>}

      {/* ── 底部操作：取消 / 保存并生效 ── */}
      <div className="form-footer">
        <Button variant="default" onClick={onClose}>
          {t('extAgents.cfg.cancel')}
        </Button>
        <Button
          variant="primary"
          loading={saving}
          disabled={!draft || !draft.key.trim() || !draft.display_name.trim()}
          onClick={handleSave}
        >
          {t('extAgents.cfg.save')}
        </Button>
      </div>
    </div>
  )
}