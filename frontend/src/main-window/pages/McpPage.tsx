import { useState, useEffect, useCallback } from 'react'
import { IconChevronDown, IconChevronRight, IconPlug } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { Section } from '../../ui/PageLayout'
import { useLanguage } from '../../locales'
import { listMcpServers, listMcpTools } from '../lib/api'
import type { McpServerInfo, McpToolInfo } from '../lib/api'
import '../../styles/mcp.css'

type ToolsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ok'; tools: McpToolInfo[] }
  | { status: 'error'; error: string }

/** MCP 管理界面（只读）：server 列表 + 按需查询工具列表，无增删改入口。 */
export function McpPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [servers, setServers] = useState<McpServerInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState('')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [toolsMap, setToolsMap] = useState<Record<string, ToolsState>>({})

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError('')
    try {
      const res = await listMcpServers()
      setServers(res?.servers || [])
    } catch (e: any) {
      setLoadError(e?.message || String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const loadTools = useCallback(async (key: string) => {
    setToolsMap(prev => ({ ...prev, [key]: { status: 'loading' } }))
    try {
      const res = await listMcpTools(key)
      setToolsMap(prev => ({ ...prev, [key]: { status: 'ok', tools: res?.tools || [] } }))
    } catch (e: any) {
      setToolsMap(prev => ({
        ...prev,
        [key]: { status: 'error', error: e?.message || String(e) },
      }))
    }
  }, [])

  const toggle = useCallback(
    (key: string) => {
      const isOpen = expanded === key
      setExpanded(isOpen ? null : key)
      // 首次展开（或重试前置的空状态）才查询工具列表
      if (!isOpen) {
        const state = toolsMap[key]
        if (!state || state.status === 'idle') void loadTools(key)
      }
    },
    [expanded, toolsMap, loadTools],
  )

  return (
    <div className="page">
      <Section title={t('mcp.serversTitle')} description={t('mcp.serversDesc')}>
        {loading ? (
          <div className="mcp-state">{t('mcp.loading')}</div>
        ) : loadError ? (
          <div className="mcp-state mcp-state--error">
            {t('mcp.loadError')}: {loadError}
            <Button variant="default" size="sm" onClick={() => void load()}>
              {t('mcp.retry')}
            </Button>
          </div>
        ) : servers.length === 0 ? (
          <div className="mcp-empty">
            <IconPlug size={20} />
            <div className="mcp-empty-title">{t('mcp.empty.title')}</div>
            <div className="mcp-empty-desc">{t('mcp.empty.desc')}</div>
            <div className="mcp-path">plugin/mcp/servers.yaml</div>
          </div>
        ) : (
          <div className="mcp-server-list">
            {servers.map(server => {
              const isOpen = expanded === server.key
              const state = toolsMap[server.key]
              return (
                <div key={server.key} className="mcp-server-card">
                  <button className="mcp-server-header" onClick={() => toggle(server.key)}>
                    <div className="mcp-server-info">
                      <div className="mcp-server-top">
                        <span className="mcp-server-key">{server.key}</span>
                        {server.auto_start ? (
                          <span className="mcp-badge mcp-badge--auto">
                            {t('mcp.autoStart')}
                          </span>
                        ) : (
                          <span className="mcp-badge">{t('mcp.onDemand')}</span>
                        )}
                      </div>
                      <div className="mcp-server-command">
                        {server.command} {server.args.join(' ')}
                      </div>
                      <div className="mcp-server-meta">
                        {t('mcp.timeout')}: {server.timeout_ms}ms
                      </div>
                    </div>
                    <span className="mcp-chevron">
                      {isOpen ? (
                        <IconChevronDown size={14} />
                      ) : (
                        <IconChevronRight size={14} />
                      )}
                    </span>
                  </button>
                  {isOpen && (
                    <div className="mcp-tools">
                      {(!state || state.status === 'idle' || state.status === 'loading') && (
                        <div className="mcp-state">{t('mcp.loadingTools')}</div>
                      )}
                      {state?.status === 'error' && (
                        <div className="mcp-state mcp-state--error">
                          {t('mcp.loadToolsError')}: {state.error}
                          <Button variant="default" size="sm" onClick={() => void loadTools(server.key)}>
                            {t('mcp.retry')}
                          </Button>
                        </div>
                      )}
                      {state?.status === 'ok' &&
                        (state.tools.length === 0 ? (
                          <div className="mcp-state">{t('mcp.noTools')}</div>
                        ) : (
                          <div className="mcp-tool-list">
                            {state.tools.map(tool => (
                              <div key={tool.name} className="mcp-tool-item">
                                <span className="mcp-tool-name">{tool.name}</span>
                                {tool.description && (
                                  <span className="mcp-tool-desc">{tool.description}</span>
                                )}
                              </div>
                            ))}
                          </div>
                        ))}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
      </Section>
    </div>
  )
}