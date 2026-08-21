import { useState, useEffect } from 'react'
import { invoke } from '../../core/bridge'
import { IconSearch, IconSprout, IconX, IconFolder } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { Section } from '../../ui/PageLayout'
import '../../styles/skills.css'

interface SkillEntry {
  name: string
  path: string
  version: string
  display_name: string
  description: string
  author: string
  keywords: string[]
  active: boolean
  installed_at: string
  builtin: boolean
}

export function SkillsPage() {
  const [skills, setSkills] = useState<SkillEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState('')
  const [installing, setInstalling] = useState(false)
  const [removing, setRemoving] = useState<string | null>(null)
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null)
  const [gitUrl, setGitUrl] = useState('')

  const load = async () => {
    try {
      setLoading(true)
      const list = await invoke<SkillEntry[]>('skill_list')
      setSkills((list || []).filter(s => !s.builtin))
    } catch (e) {
      console.error('Failed to load skills:', e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    load()
  }, [])

  const handleInstall = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const selected = await open({ directory: true, multiple: false, title: '选择技能包目录' })
      if (!selected) return
      setInstalling(true)
      const result = await invoke<string>('skill_install', { path: selected })
      console.log('Skill installed:', result)
      await load()
    } catch (e: any) {
      console.error('Install failed:', e)
      alert('安装失败: ' + (e?.message || String(e)))
    } finally {
      setInstalling(false)
    }
  }

  const handleGitInstall = async () => {
    if (!gitUrl.trim()) return
    setInstalling(true)
    try {
      await invoke('skill_install_git', { url: gitUrl.trim() })
      setGitUrl('')
      await load()
    } catch (e: any) {
      console.error('Git install failed:', e)
      alert('GitHub 安装失败: ' + (e?.message || String(e)))
    } finally {
      setInstalling(false)
    }
  }

  const handleRemove = async (name: string) => {
    setConfirmRemove(null)
    setRemoving(name)
    try {
      await invoke('skill_remove', { name })
      setSkills(prev => prev.filter(s => s.name !== name))
    } catch (e: any) {
      console.error('Remove failed:', e)
      alert('卸载失败: ' + (e?.message || String(e)))
    } finally {
      setRemoving(null)
    }
  }

  const filtered = skills.filter(s => {
    if (!search) return true
    const q = search.toLowerCase()
    return (
      s.display_name.toLowerCase().includes(q) ||
      s.name.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q) ||
      s.keywords.some(k => k.toLowerCase().includes(q))
    )
  })

  const activeCount = skills.filter(s => s.active).length

  return (
    <div className="page">
      <Section>
        <div className="compact-input-row">
          <input
            className="input-mono input-flex"
            placeholder="GitHub URL..."
            value={gitUrl}
            onChange={e => setGitUrl(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleGitInstall()}
          />
          <Button
            variant="default"
            size="sm"
            onClick={handleGitInstall}
            disabled={installing || !gitUrl.trim()}
          >
            {installing ? '安装中...' : '从Git安装'}
          </Button>
          <Button
            variant="default"
            size="sm"
            onClick={handleInstall}
            disabled={installing}
            icon={<IconFolder size={13} />}
          >
            {installing ? '安装中...' : '安装'}
          </Button>
        </div>
      </Section>

      <div className="page-search">
        <IconSearch size={14} />
        <input
          placeholder="搜索技能..."
          value={search}
          onChange={e => setSearch(e.target.value)}
        />
        <span className="page-search-count">
          {activeCount}/{skills.length}
        </span>
      </div>

      {loading ? (
        <div className="page-loading">加载中...</div>
      ) : filtered.length === 0 ? (
        <div className="page-empty">
          <IconSprout size={24} />
          <div>{search ? '没有匹配的技能' : '尚未安装任何技能'}</div>
          <div className="page-empty-hint">
            {search ? '尝试其他关键词' : '技能包扩展 AI 的能力边界'}
          </div>
          {!search && (
            <Button
              variant="default"
              size="sm"
              onClick={handleInstall}
              icon={<IconFolder size={13} />}
            >
              安装首个技能
            </Button>
          )}
        </div>
      ) : (
        <div className="page-list">
          {filtered.map((skill, idx) => (
            <div key={skill.name} className={`skill-card ${skill.active ? 'active' : 'inactive'}`}>
              <div className={`skill-card-dot ${skill.active ? 'active' : ''}`} />

              <div className="skill-card-body">
                <div className="skill-card-header">
                  <span className="skill-card-name">{skill.display_name || skill.name}</span>
                  {skill.version && <span className="skill-card-version">v{skill.version}</span>}
                </div>

                {skill.description && <div className="skill-card-desc">{skill.description}</div>}

                <div className="skill-card-tags">
                  {skill.keywords?.slice(0, 4).map(k => (
                    <span key={k} className="skill-card-tag">
                      {k}
                    </span>
                  ))}
                  {skill.author && <span className="skill-card-author">{skill.author}</span>}
                </div>
              </div>

              <div className="skill-card-actions">
                {skill.builtin ? (
                  <div className="skill-card-builtin">
                    <span className="skill-card-builtin-label">系统内置</span>
                    <span className="skill-card-builtin-sub">不可关闭</span>
                  </div>
                ) : (
                  <>
                    {confirmRemove === skill.name ? (
                      <div className="skill-card-remove-row">
                        <span className="skill-card-confirm-text">确认？</span>
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => handleRemove(skill.name)}
                          disabled={removing === skill.name}
                        >
                          卸载
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setConfirmRemove(null)}
                        >
                          取消
                        </Button>
                      </div>
                    ) : (
                      <button
                        onClick={() => setConfirmRemove(skill.name)}
                        className="skill-remove-btn"
                        title="卸载技能"
                      >
                        <IconX size={10} /> 卸载
                      </button>
                    )}
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}