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
  IconBot,
  IconTerminal,
  IconCpu,
  IconGlobe,
  IconLayers,
  IconBox,
  IconMonitor,
  IconWrench,
  IconBrain,
  IconPlug,
  IconRocket,
  IconHardDrive,
  IconAppWindow,
  IconRadio,
} from '../../ui/Icons'
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
  mode: string
  open: string
  args: string
  process: string
  description: string
}

function newDraft(): Draft {
  return {
    key: '',
    display_name: '',
    icon: 'auto',
    mode: 'embedded',
    open: '',
    args: '',
    process: '',
    description: '',
  }
}

function draftFromAgent(a: ExternalAgentConfig): Draft {
  return {
    key: a.key,
    display_name: a.display_name || '',
    icon: a.icon || 'auto',
    mode: a.mode || 'embedded',
    open: a.open || '',
    args: a.args || '',
    process: a.process || '',
    description: a.description || '',
  }
}

/** 渲染 agent 图标：icon 字符串 → lucide 组件（未知名 fallback bot）；iconUrl 优先（应用图标 data URL） */
function AgentIcon({
  icon,
  size = 14,
  iconUrl,
}: {
  icon: string
  size?: number
  iconUrl?: string | null
}) {
  if (iconUrl) {
    return (
      <img
        className="agent-icon-img"
        src={iconUrl}
        width={size}
        height={size}
        alt=""
        draggable={false}
      />
    )
  }
  switch (icon) {
    case 'terminal':
      return <IconTerminal size={size} />
    case 'cpu':
      return <IconCpu size={size} />
    case 'globe':
      return <IconGlobe size={size} />
    case 'layers':
      return <IconLayers size={size} />
    case 'box':
      return <IconBox size={size} />
    case 'monitor':
      return <IconMonitor size={size} />
    case 'wrench':
      return <IconWrench size={size} />
    case 'brain':
      return <IconBrain size={size} />
    case 'plug':
      return <IconPlug size={size} />
    case 'rocket':
      return <IconRocket size={size} />
    case 'hard-drive':
      return <IconHardDrive size={size} />
    case 'app-window':
      return <IconAppWindow size={size} />
    case 'radio':
      return <IconRadio size={size} />
    default:
      return <IconBot size={size} />
  }
}

/** agent 名 → 图标类型（与状态栏 ExternalAgentsStatusBar 的 agentKind 完全一致：CLI 类终端图标 / 其余 bot） */
function agentKind(name: string): 'cli' | 'agent' {
  const n = name.toLowerCase()
  if (n.includes('claude') || n.includes('code') || n.includes('cli')) return 'cli'
  return 'agent'
}

/** icon 值是否为文件路径（盘符 / UNC / 含扩展名） */
function isIconPath(v: string): boolean {
  if (!v) return false
  if (/^[a-zA-Z]:[\\/]/.test(v) || v.startsWith('\\\\')) return true
  return /\.[a-zA-Z0-9]{2,4}$/.test(v)
}

/** 从 open/process 启动串中提取可执行/图标文件路径（引号优先，其次含 .exe/.cmd/.bat/.lnk/.ico token） */
function exePathFromOpen(s: string): string | null {
  if (!s) return null
  const quoted = s.match(/"([^"]+\.(?:exe|cmd|bat|lnk|ico|dll))"/i)
  if (quoted) return quoted[1]
  const token = s.match(/([A-Za-z]:[\\/][^"'\s]+\.(?:exe|cmd|bat|lnk|ico|dll))/i)
  if (token) return token[1]
  return null
}

/** 解析 icon 提取源路径：显式路径直接返回；auto → open/process 中的可执行路径 */
function iconSourcePath(d: {
  icon: string
  open: string
  process: string
}): string | null {
  if (isIconPath(d.icon)) return d.icon
  if (d.icon === 'auto') return exePathFromOpen(d.open) || exePathFromOpen(d.process) || null
  return null
}

/** 跨组件图标提取缓存（同一会话内同一路径只提取一次，避免重复 PowerShell 调用） */
const iconUrlCache = new Map<string, string>()

/** 带自动提取的 AgentIcon：预设 SVG 直接渲染；auto/路径按需提取应用图标（带缓存） */
function AgentIconAuto({
  icon,
  size = 14,
  name = '',
  open = '',
  process = '',
}: {
  icon: string
  size?: number
  name?: string
  open?: string
  process?: string
}) {
  const src = iconSourcePath({ icon, open, process })
  const [url, setUrl] = useState<string | null>(() => (src ? iconUrlCache.get(src) ?? null : null))

  useEffect(() => {
    const sourceRaw = iconSourcePath({ icon, open, process })
    const source: string = sourceRaw ?? ''
    if (!source) {
      setUrl(null)
      return
    }
    if (iconUrlCache.has(source)) {
      setUrl(iconUrlCache.get(source)!)
      return
    }
    let cancelled = false
    extractAgentIcon(source)
      .then(u => {
        if (!u) return
        iconUrlCache.set(source, u)
        if (!cancelled) setUrl(u)
      })
      .catch(() => {
        /* 提取失败：保持默认 SVG 兜底 */
      })
    return () => {
      cancelled = true
    }
  }, [icon, open, process])

  // auto：优先应用图标（url）；提取不到时按 agent 名推断类型（与状态条 agentKind 一致）
  if (icon === 'auto' && !url) {
    const n = name.toLowerCase()
    if (n.includes('claude') || n.includes('code') || n.includes('cli')) {
      return <IconTerminal size={size} />
    }
    return <IconBot size={size} />
  }
  return <AgentIcon icon={icon} size={size} iconUrl={url} />
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
      await upsertExternalAgent({
        key: draft.key,
        display_name: draft.display_name.trim(),
        icon: draft.icon,
        mode: draft.mode,
        open: draft.open,
        args: draft.args,
        process: draft.process,
        description: draft.description,
      })
      // pin：用户显式添加的 agent 在应用生命周期内常驻状态栏
      // （localStorage 持久 + CustomEvent 通知状态栏立即生效）
      try {
        const raw = localStorage.getItem('nuphus.extAgents.pinned')
        const pins: string[] = raw ? JSON.parse(raw) : []
        if (Array.isArray(pins) && !pins.includes(draft.key)) {
          localStorage.setItem(
            'nuphus.extAgents.pinned',
            JSON.stringify([...pins, draft.key]),
          )
        }
      } catch {}
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
              {/* 头像统一为状态栏风格：圆形 + 按 agent 名推断类型图标（cli→终端蓝 / 其他→bot 紫） */}
              <span className="ext-agents-chip-avatar" aria-hidden>
                <span className={`agent-avatar-icon is-${agentKind(agent.key)}`}>
                  {agentKind(agent.key) === 'cli' ? (
                    <IconTerminal size={13} />
                  ) : (
                    <IconBot size={13} />
                  )}
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
          title={
            isNew ? t('extAgents.cfg.newTitle') : draft.display_name || draft.key || '—'
          }
        >
          <FormRow
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
            label={t('extAgents.cfg.icon')}
            control={
              <div className="ext-agents-icon-row">
                <span className="ext-agents-icon-preview" aria-hidden>
                  <AgentIconAuto
                    icon={draft.icon}
                    size={18}
                    name={draft.key || draft.display_name}
                    open={draft.open}
                    process={draft.process}
                  />
                </span>
                <button
                  type="button"
                  className={['ext-agents-icon-act', iconAutoActive && 'active'].filter(Boolean).join(' ')}
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
                  className={['ext-agents-icon-act', iconCustomActive && 'active'].filter(Boolean).join(' ')}
                  title={t('extAgents.cfg.iconCustom')}
                  onClick={pickCustomIcon}
                >
                  {t('extAgents.cfg.iconCustom')}
                </button>
              </div>
            }
          />
          {iconAutoActive && !iconAutoHint && !iconLoading && (
            <div className="ext-agents-icon-hint">{t('extAgents.cfg.iconAutoEmpty')}</div>
          )}
          {iconAutoActive && iconError && (
            <div className="ext-agents-icon-hint ext-agents-icon-hint--error">{iconError}</div>
          )}
          <FormRow
            stacked
            label={t('extAgents.cfg.name')}
            control={
              <input
                className="input"
                value={draft.display_name}
                onChange={e => update({ display_name: e.target.value })}
                placeholder={t('extAgents.cfg.namePlaceholder')}
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