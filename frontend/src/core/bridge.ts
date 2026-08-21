// Tauri Bridge — Type-safe wrapper + Dev Mock layer
// In Vite dev mode, automatically fallback to mock data when @tauri-apps/api is unavailable

import type {
  ToolSchema,
  MemoryStats,
  TimelineIndexStats,
  DesktopStatus,
  HooksConfigStatus,
  SessionSummary,
  SessionInfo,
  ProcessInputResponse,
  ToolExecuteResult,
  SessionDetailEntry,
} from './types'

// ── WebSocket connection (browser mode) ──
let wsPromise: Promise<WebSocket | null> | null = null
let wsSeq = 0
const wsListeners = new Map<string, Set<(payload: unknown) => void>>()

function getWsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${proto}//${location.host}/ws`
}

async function ensureWs(): Promise<WebSocket | null> {
  if (wsPromise) return wsPromise
  wsPromise = new Promise(resolve => {
    try {
      const ws = new WebSocket(getWsUrl())
      ws.onopen = () => resolve(ws)
      ws.onerror = () => {
        resolve(null)
        wsPromise = null
      }
      ws.onmessage = event => {
        try {
          const data = JSON.parse(event.data)
          // Dispatch by event type
          const typeListeners = wsListeners.get(data.type)
          if (typeListeners) typeListeners.forEach(fn => fn(data))
          // Also wrap in nuphus-event format
          const nuphusListeners = wsListeners.get('nuphus-event')
          if (nuphusListeners) {
            nuphusListeners.forEach(fn => fn({ seq: ++wsSeq, event: data }))
          }
        } catch {}
      }
      ws.onclose = () => {
        wsPromise = null
      }
    } catch {
      resolve(null)
    }
  })
  return wsPromise
}

// ── Re-export listen from Tauri API / WebSocket ──

export async function listen<T>(event: string, handler: (payload: T) => void) {
  // Tauri mode
  if (isTauriAvailable()) {
    try {
      const { listen: tauriListen } = await import('@tauri-apps/api/event')
      return tauriListen(event, (e: { payload: unknown }) => handler(e.payload as T))
    } catch {
      console.warn(`[Bridge] Tauri listen failed for ${event}`)
      return () => {}
    }
  }
  // Browser mode: WebSocket
  if (!wsListeners.has(event)) wsListeners.set(event, new Set())
  wsListeners.get(event)!.add(handler as (payload: unknown) => void)
  ensureWs()
  return () => {
    wsListeners.get(event)?.delete(handler as (payload: unknown) => void)
  }
}

// ── Backend health check (disabled: Gateway HTTP removed) ──
// Gateway axum server deleted (P1), no HTTP endpoint to poll.
// Tauri IPC health is checked via invoke return value.
export async function checkBackendAlive(_timeoutMs = 3000): Promise<boolean> {
  return false
}

// ── Mock Registry ──

type MockFn = (args?: Record<string, unknown>) => unknown

const MOCKS: Record<string, MockFn> = {}

export function registerMock(cmd: string, fn: MockFn) {
  MOCKS[cmd] = fn
}

function isTauriAvailable(): boolean {
  try {
    return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
  } catch {
    return false
  }
}

// ── Core invoke ──

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  const tauriAvail = isTauriAvailable()
  let tauriFailed = false
  let errorMsg = ''
  try {
    if (tauriAvail) {
      const { invoke: tauriInvoke } = await import('@tauri-apps/api/core')
      const rawResult = await tauriInvoke(cmd, args)
      // 体积保护：>50KB 的结果跳过全量序列化打印，避免大 session 详情的 stringify 开销
      try {
        const json = JSON.stringify(rawResult)
        if (json.length > 50 * 1024) {
          console.log(`[Bridge] invoke ${cmd} raw result: <${json.length} bytes, too large, skipped>`)
        } else {
          console.log(`[Bridge] invoke ${cmd} raw result:`, json)
        }
      } catch {
        console.log(`[Bridge] invoke ${cmd} raw result: <unserializable>`)
      }
      return rawResult as T
    }
  } catch (e: unknown) {
    errorMsg = e instanceof Error ? e.message : String(e)
    console.warn(`[Bridge] Tauri invoke ${cmd} failed:`, errorMsg)
    tauriFailed = true
  }

  // ── 重试机制：仅 IPC 连接失败时重试，后端 task panic 不重试（tokio::spawn 还在跑）──
  if (tauriFailed && cmd === 'send_message_cmd') {
    const isRetryable =
      errorMsg.includes('Connection refused') || errorMsg.includes('ERR_CONNECTION_REFUSED')
    if (isRetryable) {
      for (let attempt = 1; attempt <= 3; attempt++) {
        await new Promise(r => setTimeout(r, 500 * attempt))
        try {
          const { invoke: tauriInvoke } = await import('@tauri-apps/api/core')
          const result = (await tauriInvoke(cmd, args)) as T
          console.log(`[Bridge] ${cmd} retry #${attempt} succeeded`)
          return result
        } catch (e: unknown) {
          const msg = e instanceof Error ? e.message : String(e)
          console.warn(`[Bridge] ${cmd} retry #${attempt} failed:`, msg)
        }
      }
    }
  }

  // Tauri IPC failed — 抛出 Error 让调用方 .catch/try-catch 兜底。
  // 不可返回错误形状对象：对数组型调用方它是 truthy，存入状态后 .map 会抛
  // "x.map is not a function"，被 App 级 ErrorBoundary 捕获导致全屏错误页
  if (tauriFailed) {
    console.warn(`[Bridge] Tauri invoke ${cmd} failed, no fallback available`)
    throw new Error(`IPC invoke ${cmd} failed: ${errorMsg}`)
  }

  // Non-Tauri environment — use mock data
  if (!tauriAvail) {
    if (cmd === 'get_session_info') {
      return { version: 'mobile', name: 'Nuphus', description: 'Nuphus Mobile' } as T
    }
    if (cmd === 'get_current_config') {
      return { api_key: '', model: '', provider: '', base_url: '' } as T
    }
    if (cmd === 'is_llm_configured') {
      return false as T
    }
  }
  const mockFn = MOCKS[cmd]
  if (mockFn) {
    console.log(`[Bridge] Using mock for ${cmd}`)
    return mockFn(args) as T
  }
  console.warn(`[Bridge] No mock registered for ${cmd}, returning null`)
  return null
}

// ── Raw body invoke —— 用于传输二进制数据（绕过 base64 + JSON 编码开销）──

export async function invokeRaw<T>(cmd: string, body: Uint8Array): Promise<T | null> {
  const tauriAvail = isTauriAvailable()
  if (!tauriAvail) {
    console.warn(`[Bridge] invokeRaw ${cmd} — Tauri not available`)
    return null
  }
  try {
    const { invoke: tauriInvoke } = await import('@tauri-apps/api/core')
    const result = await tauriInvoke(cmd, body, {
      headers: { 'Content-Type': 'application/octet-stream' },
    })
    console.log(`[Bridge] invokeRaw ${cmd} success`)
    return result as T
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e)
    console.warn(`[Bridge] invokeRaw ${cmd} failed:`, msg)
    return { success: false, message: msg } as T
  }
}

// ── Mock Data ──

const MOCK_TOOLS: ToolSchema[] = [
  { name: 'read_file', description: '读取文件内容', input_schema: { type: 'object' } },
  { name: 'write_file', description: '写入文件', input_schema: { type: 'object' } },
  { name: 'list_files', description: '列出目录', input_schema: { type: 'object' } },
  { name: 'grep', description: '搜索文件内容', input_schema: { type: 'object' } },
  { name: 'run_shell', description: '执行命令', input_schema: { type: 'object' } },
  { name: 'analyze_project', description: '分析项目结构', input_schema: { type: 'object' } },
]

const MOCK_MEMORY_STATS: MemoryStats = {
  total_entries: 1423,
  patterns: 67,
  skills: 23,
  principles: 12,
  templates: 31,
}

const MOCK_TIMELINE_STATS: TimelineIndexStats = {
  total_entries: 1423,
  total_sessions: 189,
  successful: 172,
  failed: 17,
  by_intent: { 代码开发: 64, 问题排查: 43, 信息查询: 38, 系统操作: 28, 其他: 16 },
}

const MOCK_SESSIONS: SessionSummary[] = [
  {
    session_id: 's001',
    user_message: '重构前端UI布局，把侧边栏改成悬浮式',
    intent: 'Refactor',
    last_assistant_message: '已重构前端UI布局，将侧边栏改为悬浮式设计，主要改动包括...',
    entry_count: 23,
    tool_call_count: 0,
    timestamp: '2026-05-02T23:00:00Z',
    success: true,
    tags: ['frontend', 'ui'],
  },
  {
    session_id: 's002',
    user_message: '排查对话请求失败的问题，后端返回 500',
    intent: 'DebugDiagnose',
    last_assistant_message: '已排查后端500错误，根因是数据库连接池耗尽...',
    entry_count: 15,
    tool_call_count: 0,
    timestamp: '2026-05-02T22:00:00Z',
    success: true,
    tags: ['backend', 'debug'],
  },
  {
    session_id: 's003',
    user_message: '帮我分析一下项目的依赖结构',
    intent: 'ProjectAnalysis',
    last_assistant_message: '已分析项目依赖结构，主要包括...',
    entry_count: 8,
    tool_call_count: 0,
    timestamp: '2026-05-02T21:00:00Z',
    success: true,
    tags: ['analysis'],
  },
  {
    session_id: 's004',
    user_message: '写一个批量重命名文件的脚本',
    intent: 'FileOperation',
    last_assistant_message: '已创建批量重命名脚本，支持...',
    entry_count: 12,
    tool_call_count: 0,
    timestamp: '2026-05-01T18:00:00Z',
    success: true,
    tags: ['script'],
  },
  {
    session_id: 's005',
    user_message: '最小化所有窗口并截图',
    intent: 'DesktopControl',
    last_assistant_message: '',
    entry_count: 6,
    tool_call_count: 0,
    timestamp: '2026-05-01T15:00:00Z',
    success: false,
    tags: ['desktop'],
  },
]

const MOCK_SESSION_INFO: SessionInfo = {
  version: '4.0.0',
  name: 'Nuphus',
  description: 'AI Desktop Assistant',
}

const MOCK_DESKTOP_STATUS: DesktopStatus = {
  connected: true,
  python_path: 'C:\\Python314\\python.exe',
  tools_count: 15,
}

const MOCK_HOOKS_STATUS: HooksConfigStatus = {
  pre_tool_call: null,
  post_tool_call: null,
  on_session_start: null,
  on_session_end: null,
  config_path: 'hooks/hooks.yaml',
}

// ── Register all mocks ──

registerMock('get_tools', () => MOCK_TOOLS)
registerMock('get_memory_stats', () => MOCK_MEMORY_STATS)
registerMock('get_timeline_index_stats', () => MOCK_TIMELINE_STATS)
registerMock('get_knowledge_items', args => {
  const category = (args?.category as string) || 'skills'
  const items: Record<string, unknown[]> = {
    skills: [
      {
        id: 'sk1',
        name: 'Web Search',
        description: '网络搜索能力',
        active: true,
        confidence: 0.95,
      },
      {
        id: 'sk2',
        name: 'File Operations',
        description: '文件读写能力',
        active: true,
        confidence: 0.88,
      },
    ],
    seeds: [
      { id: 'sd1', seed_type: '提升', summary: '建议增加批量文件处理能力', status: 'Pending' },
    ],
  }
  return items[category] || []
})
registerMock('search_knowledge', (args?: Record<string, unknown>) => {
  const query = ((args?.query as string) || '').toLowerCase()
  const tags = (args?.tags as string[]) || []
  const all: Array<{
    rel_path: string
    title: string
    tags: string[]
    snippet: string
    file_mtime: number
  }> = [
    {
      rel_path: 'powershell/基础操作.md',
      title: 'PowerShell 基础操作',
      tags: ['powershell', '入门'],
      snippet: 'PowerShell 是 Windows 的脚本语言和执行引擎',
      file_mtime: 1700000000,
    },
    {
      rel_path: '自动化操作经验.md',
      title: '桌面/网页自动化操作经验',
      tags: ['desktop', 'automation', 'browser'],
      snippet: '本文档记录实际操作桌面和网页自动化流程中的经验',
      file_mtime: 1700000001,
    },
  ]
  // Pre-filter by tags first
  let filtered = tags.length > 0 ? all.filter(h => tags.some(t => h.tags.includes(t))) : all
  // Then filter by keyword
  if (query) {
    filtered = filtered.filter(
      h =>
        h.title.toLowerCase().includes(query) ||
        h.tags.some((t: string) => t.includes(query)) ||
        h.snippet.toLowerCase().includes(query),
    )
  }
  return filtered.slice(0, (args?.maxResults as number) || 10)
})
registerMock('list_knowledge', () => [
  {
    rel_path: 'powershell/基础操作.md',
    title: 'PowerShell 基础操作',
    tags: ['powershell', '入门'],
    snippet: 'PowerShell 是 Windows 的脚本语言和执行引擎',
    file_mtime: 1700000000,
  },
  {
    rel_path: '自动化操作经验.md',
    title: '桌面/网页自动化操作经验',
    tags: ['desktop', 'automation', 'browser'],
    snippet: '本文档记录实际操作桌面和网页自动化流程中的经验',
    file_mtime: 1700000001,
  },
])
registerMock('list_knowledge_tags', () => [
  'powershell',
  '入门',
  'desktop',
  'automation',
  'browser',
])
registerMock('delete_knowledge', () => true)
registerMock('get_current_config', () => ({
  api_key: '',
  model: 'MiniMax-M2.7',
  provider: 'minimax',
  base_url: 'https://api.minimaxi.com/v1',
}))
registerMock('get_session_history', () => MOCK_SESSIONS)
registerMock('get_session_detail', args => {
  // 兼容 camelCase（前端 invoke 实际发送）与 snake_case 两种键名
  const sessionId = ((args?.sessionId ?? args?.session_id) as string) || ''
  const session = MOCK_SESSIONS.find(s => s.session_id === sessionId)
  if (!session) return []

  const turns: Record<string, Array<{ user: string; assistant: string }>> = {
    s001: [
      {
        user: '重构前端UI布局，把侧边栏改成悬浮式',
        assistant:
          '侧边栏改为悬浮式：1) absolute/fixed 定位 2) 展开收起动画 3) margin 调整。推荐 CSS transition。',
      },
      {
        user: '动画用哪种方式实现？',
        assistant: 'CSS transition 配合 transform。宽度 0→240px，transition 0.25s ease。',
      },
      {
        user: '内容区怎么适配？',
        assistant: 'flex: 1 + min-width: 0 或 grid-template-columns: auto 1fr。',
      },
    ],
    s002: [
      {
        user: '排查对话请求失败，后端返回 500',
        assistant: '数据库连接池耗尽。建议：增加上限、查询超时、排查泄漏。',
      },
      { user: '上限调到多少？', assistant: '当前 10，建议 20-30。加空闲超时 60s。' },
    ],
    s003: [
      {
        user: '分析项目依赖结构',
        assistant:
          'React 19 + Vite + Tauri v2 + Rust。核心链：App→ModalPage→HistoryPage→API→Bridge。',
      },
      {
        user: 'Bridge mock 机制？',
        assistant: 'isTauriAvailable() 检测。Vite 下用 mock，生产调真实 invoke。',
      },
    ],
    s004: [
      {
        user: '写批量重命名脚本',
        assistant:
          'import os\nfor f in os.listdir("."):\n    if f.endswith(".tmp"): os.rename(f, f.replace(".tmp", ".bak"))',
      },
      {
        user: '加递归子目录',
        assistant:
          'import os\nfor root, dirs, files in os.walk("."):\n    for f in files:\n        if f.endswith(".tmp"): os.rename(...)',
      },
    ],
    s005: [
      { user: '最小化所有窗口并截图', assistant: '枚举窗口→最小化→500ms→截图。系统窗口已跳过。' },
      { user: '跳过了哪些窗口？', assistant: '12 个成功 10 个。跳过 Progman 和开始菜单。' },
    ],
  }

  const sessionTurns = turns[sessionId] || turns['s001']
  return sessionTurns.map((t, i) => ({
    id: `${sessionId}-turn-${i}`,
    user_message: t.user,
    assistant_message: t.assistant,
    steps_summary: [`第 ${i + 1} 轮`, `意图: ${session.intent}`],
    timestamp: session.timestamp,
    success: session.success,
  }))
})
registerMock('configure_llm', _args => 'LLM configured successfully (mock)')
registerMock('get_desktop_status', () => MOCK_DESKTOP_STATUS)
registerMock('get_hooks_status', () => MOCK_HOOKS_STATUS)
registerMock('get_session_info', () => MOCK_SESSION_INFO)
registerMock('execute_tool', _args => ({
  success: true,
  output: 'Mock tool execution result',
  error: '',
}))
registerMock('send_message_cmd', _args => ({ success: true, message: 'Mock: message sent' }))
registerMock('reject_security', () => 'Mock: rejected')
registerMock('interrupt', () => 'Mock: interrupted')
registerMock('graceful_stop', () => 'mock-graceful-stop-action-id')
registerMock(
  'set_permission_mode',
  (args?: Record<string, unknown>) => `Mock: permission mode set to ${args?.mode || 'unknown'}`,
)
registerMock('get_context_limit', async () => {
  // Context window limits by model family (mock fallback)
  return 128000
})

// ── Workflow Mocks ──

interface MockBackendWorkflow {
  id: string
  name: string
  created_at: string
  updated_at: string
  status: string
  steps: MockBackendStep[]
  doc?: string
  schedule?: any
  run_history: any[]
  timeout_secs?: number | null
  dry_run?: boolean
  tags?: string[]
}

interface MockBackendStep {
  id: string
  name: string
  description?: string
  on_error?: any
  capture?: string
  timeout_secs?: number
  do: Record<string, unknown>
}

let mockWorkflows: MockBackendWorkflow[] = [
  {
    id: 'nuphus-tour-v3',
    name: 'nuphus-tour-v3',
    doc: 'Nuphus 功能面板巡览',
    status: 'Ready',
    created_at: new Date(Date.now() - 86400000 * 7).toISOString(),
    updated_at: new Date(Date.now() - 3600000).toISOString(),
    steps: [
      {
        id: 'init', name: '初始化', on_error: 'abort',
        do: { seq: [
          { id: 'init_list', name: '列窗', on_error: 'abort', capture: '@wins', do: { tool: 'desktop_windows_list', with: {} } },
          { id: 'init_compute', name: '预计算坐标', on_error: 'abort', capture: '@panels:json', do: { script: { runtime: 'python', code: 'print("computing...")' } } },
        ]},
      },
      {
        id: 'tour', name: '面板巡览', on_error: 'abort',
        do: { loop: { for_each: { items: { var: 'panels' }, as: 'p' }, max: 100, do: [
          { id: 'tour_activate', name: '激活窗口', on_error: 'abort', do: { tool: 'desktop_window_activate', with: { hwnd: '{{p.h}}' } } },
          { id: 'tour_click', name: '点开面板', on_error: 'abort', do: { tool: 'desktop_mouse_click', with: { x: 100, y: 200 } } },
        ]}},
      },
    ],
    tags: ['tour'],
    run_history: [
      { id: 'r1', status: 'completed', started_at: new Date(Date.now() - 3600000).toISOString(), finished_at: new Date(Date.now() - 3500000).toISOString() },
      { id: 'r2', status: 'completed', started_at: new Date(Date.now() - 7200000).toISOString(), finished_at: new Date(Date.now() - 7100000).toISOString() },
      { id: 'r3', status: 'failed', started_at: new Date(Date.now() - 10800000).toISOString(), finished_at: null, error: 'timeout' },
    ],
  },
  {
    id: 'wf-001',
    name: '代码审查流程',
    doc: '自动审查 Pull Request 的代码质量、风格和安全问题',
    status: 'Ready',
    created_at: new Date(Date.now() - 86400000 * 7).toISOString(),
    updated_at: new Date(Date.now() - 3600000).toISOString(),
    steps: [
      { id: 'step-1', name: '拉取代码', description: '从远程仓库拉取最新代码', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'git pull' } } },
      { id: 'step-2', name: '静态分析', description: '运行 linter 和类型检查', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'cargo clippy' } } },
      { id: 'step-3', name: '安全扫描', description: '检查依赖和已知漏洞', on_error: 'skip', do: { tool: 'system_shell', with: { command: 'cargo audit' } } },
    ],
    tags: ['code-review', 'ci'],
    run_history: Array.from({ length: 23 }, (_, i) => ({ id: `r-${i}`, status: i < 21 ? ('completed' as const) : ('failed' as const), started_at: new Date(Date.now() - (i + 1) * 86400000).toISOString(), finished_at: new Date(Date.now() - (i + 1) * 86400000 + 600000).toISOString() })),
  },
  {
    id: 'wf-002',
    name: '部署流水线',
    doc: '从构建到生产环境的完整部署流程',
    status: 'Ready',
    created_at: new Date(Date.now() - 86400000 * 30).toISOString(),
    updated_at: new Date(Date.now() - 86400000).toISOString(),
    steps: [
      { id: 'step-1', name: '构建', description: '编译前端和后端代码', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'npm run build' } } },
      { id: 'step-2', name: '测试', description: '运行单元测试和集成测试', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'npm test' } } },
      { id: 'step-3', name: '部署到 staging', description: '推送到预发布环境验证', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'deploy staging' } } },
      { id: 'step-4', name: '生产部署', description: '灰度发布到生产环境', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'deploy prod' } } },
    ],
    tags: ['deploy', 'devops'],
    run_history: Array.from({ length: 56 }, (_, i) => ({ id: `r-${i}`, status: 'completed' as const, started_at: new Date(Date.now() - (i + 1) * 43200000).toISOString(), finished_at: new Date(Date.now() - (i + 1) * 43200000 + 300000).toISOString() })),
  },
  {
    id: 'wf-003',
    name: '文档自动生成',
    doc: '从代码注释自动生成 API 文档和 changelog',
    status: 'Draft',
    created_at: new Date(Date.now() - 86400000 * 3).toISOString(),
    updated_at: new Date(Date.now() - 86400000 * 2).toISOString(),
    steps: [
      { id: 'step-1', name: '提取注释', description: '扫描源码中的 JSDoc/RustDoc 注释', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'extract-docs' } } },
      { id: 'step-2', name: '生成 Markdown', description: '将注释转换为结构化文档', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'generate-md' } } },
      { id: 'step-3', name: '发布', description: '推送到文档站点', on_error: 'abort', do: { tool: 'system_shell', with: { command: 'publish-docs' } } },
    ],
    tags: ['docs', 'automation'],
    run_history: [],
  },
]

registerMock('wf_list', async () => ({ workflows: mockWorkflows }))
registerMock('wf_delete', async (args?: Record<string, unknown>) => {
  const { id } = args || {}
  const idx = mockWorkflows.findIndex(w => w.id === id)
  if (idx === -1) return false
  mockWorkflows.splice(idx, 1)
  return true
})
registerMock('wf_parse_template', async args => {
  const input = args as unknown as { templateText?: string }
  const text = input.templateText || ''
  // Parse hidden workflow-steps JSON from template
  const stepsMatch = text.match(/<!-- workflow-steps\n([\s\S]*?)\n-->/)
  let steps: { name?: string; description?: string; tool?: string; params?: Record<string, unknown> }[] = []
  let name = '未命名工作流'
  let description = ''

  // Extract name from [workflow: ...] header
  const nameMatch = text.match(/^\[workflow:\s*(.+?)\]/)
  if (nameMatch) {
    name = nameMatch[1].trim()
  }

  // Extract description from text after header and before hidden block
  const descMatch = text.match(/^\[workflow:.*?\]\n([\s\S]*?)(?:\n<!--|$)/)
  if (descMatch) {
    description = descMatch[1].trim()
  }

  if (stepsMatch) {
    try {
      const parsed = JSON.parse(stepsMatch[1])
      steps = (parsed.steps || []).map((s: any) => ({
        name: s.name || '',
        description: s.description || '',
        tool: s.tool || 'system_shell',
        params: s.params || {},
      }))
    } catch {
      /* ignore parse errors */
    }
  }

  return {
    name,
    steps,
    states: null,
    elements: null,
    guide: description,
  }
})
// ── 移动端局域网 server（浏览器模式 mock：状态自洽，供 vite dev 下UI开发/验证）──
const MOCK_MOBILE_STATE = {
  running: false,
  port: 18772,
  token: 'mock-mobile-token-abcdef0123456789',
}
registerMock('mobile_server_status', () => ({
  running: MOCK_MOBILE_STATE.running,
  port: MOCK_MOBILE_STATE.port,
  token: MOCK_MOBILE_STATE.token,
  lan_url: MOCK_MOBILE_STATE.running
    ? `http://192.168.1.100:${MOCK_MOBILE_STATE.port}`
    : null,
}))
registerMock('mobile_server_start', args => {
  const port = typeof args?.port === 'number' ? args.port : MOCK_MOBILE_STATE.port
  MOCK_MOBILE_STATE.running = true
  MOCK_MOBILE_STATE.port = port
  return {
    running: true,
    port,
    token: MOCK_MOBILE_STATE.token,
    lan_url: `http://192.168.1.100:${port}`,
  }
})
registerMock('mobile_server_stop', () => {
  MOCK_MOBILE_STATE.running = false
})
registerMock('mobile_token_regenerate', () => {
  MOCK_MOBILE_STATE.token =
    'mock-regenerated-' + Math.random().toString(16).slice(2, 10) + '-0123456789'
  return MOCK_MOBILE_STATE.token
})
