/**
 * AppShellPage.tsx — 应用插件全屏宿主（App Plugin 体系 P0，设计文档 §4.2/§5）
 *
 * 职责：
 * - 全屏覆盖层 + iframe 沙箱（sandbox="allow-scripts"，无 allow-same-origin/
 *   allow-modals——插件运行在 opaque origin，存储全部由 kv.* Bridge 承接）
 * - Bridge 桥接器：ready→init 握手、call 分派（kv.*、notify.toast、theme.get）、
 *   权限白名单逐次校验、主题变化事件推送
 *
 * 安全模型（三层独立设防）：
 * 1. origin 校验：event.origin === http://127.0.0.1:{port}（端口来自运行中 server 实际值）
 * 2. source 绑定：event.source === iframe.contentWindow（严禁信消息内自报 pluginId）
 * 3. permissions 白名单：每次 call 按该插件 manifest 声明逐次鉴权
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { useLanguage } from '../../locales'
import { useTheme } from '../../hooks/useTheme'
import { IconArrowLeft, IconMinus, IconPuzzle } from '../../ui/Icons'
import {
  pluginAppList,
  pluginAgentChat,
  pluginKvGet,
  pluginKvSet,
  pluginKvDelete,
  pluginKvKeys,
  pluginWorkflowList,
  pluginWorkflowRun,
  type PluginAppSummary,
  type PluginChatHistoryItem,
  type PluginToastFn,
} from '../lib/plugin-apps'
import { mobileServerStatus, mobileServerEnsure } from '../lib/api'
import '../../styles/plugin-apps.css'

interface AppShellPageProps {
  pluginId: string
  onClose: () => void
  /** 最小化：宿主保持挂载（visibility 隐藏保活），由 App.tsx 悬浮按钮恢复 */
  onMinimize: () => void
  /** 最小化态：根元素 visibility:hidden + pointer-events:none（禁 display:none，保 iframe 不重载） */
  minimized: boolean
  showToast: PluginToastFn
}

interface BridgeEnvelope {
  nuphus: number
  id?: string
  type: 'ready' | 'init' | 'call' | 'result' | 'event'
  method?: string
  params?: Record<string, unknown>
  ok?: boolean
  payload?: unknown
  error?: { code: string; message?: string }
  event?: string
}

const BRIDGE_VERSION = 1

/** agent.chat v1 同步等待超时（契约 §5.3：120s） */
const AGENT_CHAT_TIMEOUT_MS = 120_000

/** workflow.run v1 同步等待超时（契约 §5.3：120s；后端 300s 硬超时收尸） */
const WORKFLOW_RUN_TIMEOUT_MS = 120_000

/** agent.chat history 上限（与后端/样例插件一致） */
const MAX_PLUGIN_CHAT_HISTORY = 50
const MAX_PLUGIN_CHAT_HISTORY_CHARS = 64 * 1024

/**
 * agent.chat history 参数校验（契约 §5.3）：
 * 数组元素须为 {role, content} 形状（role ∈ user/assistant/system），
 * 上限 50 条、总字符 ≤64KB，超限/形状非法返回 null（桥接器返回 INVALID_PARAMS）；
 * 未传/空值返回 undefined（不携带该参数）。
 */
function normalizeChatHistory(value: unknown): PluginChatHistoryItem[] | null | undefined {
  if (value === undefined || value === null) return undefined
  if (!Array.isArray(value) || value.length > MAX_PLUGIN_CHAT_HISTORY) return null
  const items: PluginChatHistoryItem[] = []
  let total = 0
  for (const raw of value) {
    if (!raw || typeof raw !== 'object') return null
    const item = raw as Record<string, unknown>
    const role = item.role
    const content = item.content
    if (role !== 'user' && role !== 'assistant' && role !== 'system') return null
    if (typeof content !== 'string' || !content.trim()) return null
    total += role.length + content.length
    if (total > MAX_PLUGIN_CHAT_HISTORY_CHARS) return null
    items.push({ role, content })
  }
  return items
}

function okEnvelope(id: string, payload: unknown): BridgeEnvelope {
  return { nuphus: BRIDGE_VERSION, id, type: 'result', ok: true, payload: payload ?? null }
}

function errEnvelope(id: string, code: string, message?: string): BridgeEnvelope {
  return {
    nuphus: BRIDGE_VERSION,
    id,
    type: 'result',
    ok: false,
    error: { code, message: message || code },
  }
}

export function AppShellPage({
  pluginId,
  onClose,
  onMinimize,
  minimized,
  showToast,
}: AppShellPageProps) {
  const { t, lang } = useLanguage()
  const { theme, customTheme, previewOverrides } = useTheme()

  const iframeRef = useRef<HTMLIFrameElement | null>(null)
  const [plugin, setPlugin] = useState<PluginAppSummary | null>(null)
  const [port, setPort] = useState<number | null>(null)
  const [loadError, setLoadError] = useState('')

  // agent.chat 在途串行标记（每插件同时只允许一个在途 chat；ref 避免重渲染）
  const chatInFlightRef = useRef(false)
  // workflow.run 在途串行标记（每插件同时只允许一个在途工作流执行；ref 避免重渲染）
  const workflowInFlightRef = useRef(false)

  // message handler 内读取的最新值（ref 避免 effect 重复绑定）
  const pluginRef = useRef<PluginAppSummary | null>(null)
  const portRef = useRef<number | null>(null)
  const readyRef = useRef(false)
  const themeRef = useRef({ base: '', overrides: {} as Record<string, string> })

  // 生效主题：预览优先 → 已保存自定义 → 纯内置基底（与 useTheme 应用逻辑一致）
  const currentTheme = useCallback(() => {
    const base = customTheme?.base ?? theme
    const overrides =
      previewOverrides !== null ? previewOverrides : customTheme ? customTheme.overrides : {}
    return { base, overrides }
  }, [theme, customTheme, previewOverrides])

  useEffect(() => {
    themeRef.current = currentTheme()
  }, [currentTheme])

  const postToPlugin = useCallback((msg: BridgeEnvelope) => {
    const iframe = iframeRef.current
    const p = portRef.current
    if (iframe?.contentWindow && p !== null) {
      // targetOrigin 必须 '*'：iframe sandbox 无 allow-same-origin → 插件为 opaque origin，
      // 任何具体 origin 都不匹配，postMessage 会静默丢弃（§4.2/§5：回程安全由插件侧
      // 信封版本字段 + 请求 id 配对保证，opaque origin 本就不携带身份信息）。
      iframe.contentWindow.postMessage(msg, '*')
    }
  }, [])

  // ── 加载插件信息 + 确保插件伺服运行（复用移动端 server，port 取实际值）──
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        let status = await mobileServerStatus()
        if (!status?.running) {
          // ensure 非持久化启动：插件需要 server 但不改变用户移动端开关设置
          status = await mobileServerEnsure()
        }
        if (cancelled) return
        if (!status) {
          setLoadError(t('plugins.serverUnavailable'))
          return
        }
        setPort(status.port)
        portRef.current = status.port
        const list = (await pluginAppList()) || []
        const found = list.find(p => p.id === pluginId)
        if (cancelled) return
        if (!found) {
          setLoadError(t('plugins.notFound'))
          return
        }
        setPlugin(found)
        pluginRef.current = found
      } catch (e: any) {
        if (!cancelled) setLoadError(e?.message || String(e))
      }
    })()
    return () => {
      cancelled = true
    }
  }, [pluginId, t])

  // ── Bridge call 分派（权限白名单逐次校验）──
  const handleCall = useCallback(
    async (
      id: string,
      method: string,
      params: Record<string, unknown>,
    ): Promise<BridgeEnvelope> => {
      // ready 前拒绝任何 call（SDK 自动 ready → init 后才能发 call，此处纵深防御）
      if (!readyRef.current) return errEnvelope(id, 'NOT_READY', t('plugins.errNotReady'))
      const perms = pluginRef.current?.permissions ?? []
      const has = (perm: string) => perms.includes(perm)
      const key = params?.key

      try {
        if (method === 'kv.get') {
          if (!has('kv')) return errEnvelope(id, 'PERMISSION_DENIED')
          if (typeof key !== 'string' || !key) return errEnvelope(id, 'INVALID_PARAMS')
          return okEnvelope(id, await pluginKvGet(pluginId, key))
        }
        if (method === 'kv.set') {
          if (!has('kv')) return errEnvelope(id, 'PERMISSION_DENIED')
          if (typeof key !== 'string' || !key) return errEnvelope(id, 'INVALID_PARAMS')
          await pluginKvSet(pluginId, key, params.value)
          return okEnvelope(id, null)
        }
        if (method === 'kv.delete') {
          if (!has('kv')) return errEnvelope(id, 'PERMISSION_DENIED')
          if (typeof key !== 'string' || !key) return errEnvelope(id, 'INVALID_PARAMS')
          await pluginKvDelete(pluginId, key)
          return okEnvelope(id, null)
        }
        if (method === 'kv.keys') {
          if (!has('kv')) return errEnvelope(id, 'PERMISSION_DENIED')
          return okEnvelope(id, await pluginKvKeys(pluginId))
        }
        if (method === 'notify.toast') {
          if (!has('notify')) return errEnvelope(id, 'PERMISSION_DENIED')
          const text = params?.text
          if (typeof text !== 'string' || !text) return errEnvelope(id, 'INVALID_PARAMS')
          showToast(text, 'info')
          return okEnvelope(id, null)
        }
        if (method === 'theme.get') {
          // 只读主题查询，免权限声明（hello 样板即不依赖 permissions 声明即可用）
          return okEnvelope(id, themeRef.current)
        }
        if (method === 'agent.chat') {
          if (!has('agent.chat')) return errEnvelope(id, 'PERMISSION_DENIED')
          const text = params?.message
          if (typeof text !== 'string' || !text.trim()) return errEnvelope(id, 'INVALID_PARAMS')
          // history 可选：{role, content}[]（≤50 条、总字符 ≤64KB），超限/形状非法 → INVALID_PARAMS
          const history = normalizeChatHistory(params?.history)
          if (history === null) return errEnvelope(id, 'INVALID_PARAMS')
          // 在途串行：同插件上一个 chat 未完成时拒绝重入（后端 guard 为纵深防御）
          if (chatInFlightRef.current) return errEnvelope(id, 'BUSY', t('plugins.errChatBusy'))
          chatInFlightRef.current = true
          let timer: ReturnType<typeof setTimeout> | undefined
          try {
            const reply = await Promise.race([
              pluginAgentChat(pluginId, text, history),
              new Promise<never>((_, reject) => {
                timer = setTimeout(() => reject(new Error('CHAT_TIMEOUT')), AGENT_CHAT_TIMEOUT_MS)
              }),
            ])
            return okEnvelope(id, reply)
          } catch (e: any) {
            // 120s 超时（契约 §5.3 同步等待语义；v1 无流式增量）
            if (e?.message === 'CHAT_TIMEOUT') {
              return errEnvelope(id, 'TIMEOUT', t('plugins.errChatTimeout'))
            }
            // 后端校验/执行错误（未声明权限、消息非法、后端在途等）透传 message
            return errEnvelope(id, 'AGENT_CHAT_FAILED', e?.message || String(e))
          } finally {
            if (timer) clearTimeout(timer)
            chatInFlightRef.current = false
          }
        }
        if (method === 'workflow.list') {
          // 与 workflow.run 共用单一权限（契约 §5.3：禁止新增权限枚举）
          if (!has('workflow.run')) return errEnvelope(id, 'PERMISSION_DENIED')
          // 只读列表，不触发执行
          return okEnvelope(id, await pluginWorkflowList())
        }
        if (method === 'workflow.run') {
          if (!has('workflow.run')) return errEnvelope(id, 'PERMISSION_DENIED')
          const wid = params?.id
          if (typeof wid !== 'string' || !wid.trim()) return errEnvelope(id, 'INVALID_PARAMS')
          // 在途串行：同插件上一个 run 未完成时拒绝重入（后端 guard 为纵深防御）
          if (workflowInFlightRef.current)
            return errEnvelope(id, 'BUSY', t('plugins.errWorkflowBusy'))
          workflowInFlightRef.current = true
          let timer: ReturnType<typeof setTimeout> | undefined
          try {
            const result = await Promise.race([
              pluginWorkflowRun(pluginId, wid),
              new Promise<never>((_, reject) => {
                timer = setTimeout(
                  () => reject(new Error('WORKFLOW_TIMEOUT')),
                  WORKFLOW_RUN_TIMEOUT_MS,
                )
              }),
            ])
            return okEnvelope(id, result)
          } catch (e: any) {
            // 120s 超时（契约 §5.3 同步等待语义；后端 300s 硬超时收尸）
            if (e?.message === 'WORKFLOW_TIMEOUT') {
              return errEnvelope(id, 'TIMEOUT', t('plugins.errWorkflowTimeout'))
            }
            // 后端校验/执行错误（未声明权限、工作流不存在、后端在途等）透传 message
            return errEnvelope(id, 'WORKFLOW_RUN_FAILED', e?.message || String(e))
          } finally {
            if (timer) clearTimeout(timer)
            workflowInFlightRef.current = false
          }
        }
        return errEnvelope(id, 'UNKNOWN_METHOD')
      } catch (e: any) {
        return errEnvelope(id, 'INTERNAL_ERROR', e?.message || String(e))
      }
    },
    [pluginId, showToast, t],
  )

  // ── 桥接器消息监听（origin + source 双校验）──
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      const p = portRef.current
      if (p === null) return
      // 第一道防线（origin 形态白名单）：iframe sandbox 不含 allow-same-origin →
      // 插件为 opaque origin，postMessage 的 origin 恒为 "null"（§4.2 有意设计）；
      // 兼容未来放开 same-origin 后的伺服 origin 形态。
      // ⚠️ 消息认证主防线是下方 source 绑定（§5：pluginId 绑定 iframe 实例），
      // opaque origin 下 origin 不携带身份信息，不得作为唯一校验。
      if (event.origin !== 'null' && event.origin !== `http://127.0.0.1:${p}`) return
      // 第二道防线：source 必须等于本宿主持有的 iframe.contentWindow
      const iframe = iframeRef.current
      if (!iframe || event.source !== iframe.contentWindow) return

      const msg = event.data as BridgeEnvelope | null
      if (!msg || msg.nuphus !== BRIDGE_VERSION) return
      if (msg.type === 'ready') {
        readyRef.current = true
        postToPlugin({
          nuphus: BRIDGE_VERSION,
          type: 'init',
          payload: {
            pluginId,
            permissions: pluginRef.current?.permissions ?? [],
            theme: themeRef.current,
            locale: lang,
          },
        })
        return
      }
      if (msg.type === 'call' && typeof msg.id === 'string' && typeof msg.method === 'string') {
        void handleCall(msg.id, msg.method, msg.params ?? {}).then(resp => postToPlugin(resp))
      }
    }
    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [handleCall, lang, pluginId, postToPlugin])

  // ── 主题变化 → 向插件推送 theme.changed 事件（iframe 重取 theme.css 获得新值）──
  useEffect(() => {
    if (!readyRef.current) return
    postToPlugin({
      nuphus: BRIDGE_VERSION,
      type: 'event',
      event: 'theme.changed',
      payload: { base: themeRef.current.base },
    })
  }, [currentTheme, postToPlugin])

  const img =
    port !== null && plugin?.icon
      ? `http://127.0.0.1:${port}/plugins/${plugin.id}/${plugin.icon}`
      : null

  return (
    <div className={`plugin-shell${minimized ? ' plugin-shell--minimized' : ''}`}>
      <div className="plugin-shell-topbar">
        <button className="plugin-shell-back" onClick={onClose} aria-label={t('plugins.back')}>
          <IconArrowLeft size={16} />
        </button>
        <div className="plugin-shell-title">
          <div className="plugin-app-icon">
            {img ? (
              <img
                src={img}
                alt=""
                onError={e => {
                  ;(e.target as HTMLImageElement).style.display = 'none'
                }}
              />
            ) : (
              <IconPuzzle size={16} />
            )}
          </div>
          <span className="plugin-shell-name">{plugin?.name ?? pluginId}</span>
          {plugin && <span className="plugin-app-version">v{plugin.version}</span>}
        </div>
        <button
          className="plugin-shell-min"
          onClick={onMinimize}
          aria-label={t('plugins.minimize')}
          title={t('plugins.minimize')}
        >
          <IconMinus size={16} />
        </button>
      </div>
      <div className="plugin-shell-body">
        {loadError ? (
          <div className="plugin-shell-error">{loadError}</div>
        ) : port === null ? (
          <div className="plugin-shell-loading">{t('common.loading')}</div>
        ) : (
          <iframe
            ref={iframeRef}
            className="plugin-shell-iframe"
            sandbox="allow-scripts"
            src={`http://127.0.0.1:${port}/plugins/${pluginId}/`}
            referrerPolicy="no-referrer"
            title={plugin?.name ?? pluginId}
          />
        )}
      </div>
    </div>
  )
}
