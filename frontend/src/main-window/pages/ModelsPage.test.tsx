import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { listen } from '@tauri-apps/api/event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createCustomProvider,
  getSupportedProviders,
  oauthBegin,
  oauthLogout,
  oauthStatus,
  openExternal,
  updateCustomProvider,
} from '../lib/api'
import { ModelsPage } from './ModelsPage'

/**
 * 自定义模型（自定义中转站）新建流程回归：
 * - 「+ 新建」只有四个输入项：自定义名称 / 模型提供商 / 模型 API Key / 模型 API URL；
 * - 名称与 URL 为空必须当场拦下（不发 IPC），错误文案就是字段名本身；
 * - 保存走 create_custom_provider 真正落盘，成功后实例立刻出现在左栏（不是只改前端状态）；
 * - Anthropic 兼容实例没有 /v1/models：不出现「连接」，改为手动填模型名 + 一行如实说明。
 */
vi.mock('../lib/api', () => ({
  getCurrentConfig: vi.fn(),
  configureLlm: vi.fn(),
  clearProviderApiKey: vi.fn(),
  switchModel: vi.fn(),
  getSupportedProviders: vi.fn(),
  getCapabilities: vi.fn(),
  setCapability: vi.fn(),
  listModels: vi.fn(),
  listProviderModels: vi.fn(),
  refreshProviderModels: vi.fn(),
  getProviderBaseUrl: vi.fn(),
  addProviderModel: vi.fn(),
  clearProviderModels: vi.fn(),
  getAgentModels: vi.fn(),
  getProviderContext: vi.fn(),
  setAgentModel: vi.fn(),
  setModelContextWindow: vi.fn(),
  setModelSupportsVision: vi.fn(),
  setCapabilityBinding: vi.fn(),
  createCustomProvider: vi.fn(),
  updateCustomProvider: vi.fn(),
  oauthBegin: vi.fn(),
  oauthStatus: vi.fn(),
  oauthLogout: vi.fn(),
  openExternal: vi.fn(),
  sttStatus: vi.fn(),
}))

// 授权结果事件由后端推送：桩住 Tauri 事件 API。listen 返回的退订函数要能断言
// （组件卸载必须退订，否则监听泄漏）→ 用 vi.hoisted 让 mock 工厂拿到同一个桩。
const { unlistenMock } = vi.hoisted(() => ({ unlistenMock: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(unlistenMock)),
}))

// 两个下载 hook 会注册 Tauri 事件监听：本测试不进入能力页，给最小桩即可
vi.mock('../lib/useSttModelDownload', () => ({
  useSttModelDownload: () => ({ refresh: vi.fn(), status: null }),
  sttDownloadProgressPct: () => 0,
  sttDownloadProgressText: () => '',
}))
vi.mock('../lib/useVisionModelDownload', () => ({
  useVisionModelDownload: () => ({
    refresh: vi.fn(),
    status: null,
    downloading: false,
    progress: null,
  }),
  modelsDownloadProgressPct: () => 0,
  modelsDownloadProgressText: () => '',
}))

import {
  getAgentModels,
  getCapabilities,
  getCurrentConfig,
  getProviderBaseUrl,
  getProviderContext,
  listModels,
  refreshProviderModels,
  sttStatus,
} from '../lib/api'

type ProviderInfoStub = {
  id: string
  name: string
  display_name?: string
  provider_type: string
  base_url: string
  default_model: string
  auth_header: string
  auth_prefix: string
}

function provider(over: Partial<ProviderInfoStub> & { id: string }): ProviderInfoStub {
  return {
    name: over.id,
    provider_type: over.id,
    base_url: '',
    default_model: '',
    auth_header: 'Authorization',
    auth_prefix: 'Bearer ',
    ...over,
  }
}

let providerList: ProviderInfoStub[] = []

beforeEach(() => {
  vi.clearAllMocks()
  providerList = [
    provider({ id: 'deepseek', name: 'DeepSeek', base_url: 'https://api.deepseek.com' }),
  ]

  vi.mocked(getSupportedProviders).mockImplementation(async () => providerList)
  vi.mocked(getCurrentConfig).mockResolvedValue({
    model: 'deepseek-flash',
    provider: 'deepseek',
    base_url: 'https://api.deepseek.com',
    has_key: true,
    configured_providers: ['deepseek'],
  } as never)
  vi.mocked(getProviderContext).mockResolvedValue({ provider: 'deepseek' } as never)
  vi.mocked(getCapabilities).mockResolvedValue({
    model: 'deepseek-flash',
    vision: '',
    vision_provider: '',
    stt: '',
    tts: '',
    voice: '',
    chat_agent_max_iterations: null,
  } as never)
  vi.mocked(getAgentModels).mockResolvedValue({
    leader: '',
    leader_provider: '',
    workflow: '',
    workflow_provider: '',
    exec: '',
    exec_provider: '',
    custom: '',
    custom_provider: '',
  } as never)
  vi.mocked(listModels).mockResolvedValue([])
  vi.mocked(getProviderBaseUrl).mockResolvedValue(null)
  vi.mocked(refreshProviderModels).mockResolvedValue({ models: [], report: null } as never)
  vi.mocked(sttStatus).mockResolvedValue(null as never)
  // 订阅账号：默认「未配 oauth」（官方服务商与只用静态密钥的实例都是这个状态）
  vi.mocked(oauthStatus).mockResolvedValue({
    configured: false,
    logged_in: false,
    expires_at: null,
    needs_login: false,
  } as never)
})

async function openCreateForm() {
  render(<ModelsPage onClose={() => {}} />)
  // 左栏「自定义模型」组第一项 = 固定 custom 配置入口（点击进入创建态表单）
  const trigger = await screen.findByRole('button', { name: '创建（自定义/中转站）' })
  fireEvent.click(trigger)
  await screen.findByLabelText('自定义名称')
}

describe('ModelsPage 自定义模型（Custom 配置入口 → 创建具名实例）', () => {
  it('Custom 入口展开创建态表单，且预填旧 custom 段的地址与协议', async () => {
    providerList = [
      ...providerList,
      provider({
        id: 'custom',
        name: 'custom',
        provider_type: 'custom',
        base_url: 'https://legacy.example/v1',
      }),
    ]

    await openCreateForm()

    const typeSelect = screen.getByLabelText('模型提供商') as HTMLSelectElement
    expect(typeSelect.value).toBe('custom')
    expect(screen.getByLabelText('模型 API URL')).toHaveValue('https://legacy.example/v1')
    // display_name 留空由用户填（旧段无显示名，也不代填）
    expect(screen.getByLabelText('自定义名称')).toHaveValue('')
  })

  it('创建态表单的字段与占位提示（默认选中 OpenAI 兼容）', async () => {
    await openCreateForm()

    expect(screen.getByLabelText('自定义名称')).toHaveAttribute('placeholder', '例如：公司网关')
    const typeSelect = screen.getByLabelText('模型提供商') as HTMLSelectElement
    expect(typeSelect.value).toBe('custom')
    expect(screen.getByRole('option', { name: 'OpenAI 兼容' })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: 'Anthropic 兼容' })).toBeInTheDocument()
    expect(screen.getByLabelText('模型 API Key')).toHaveAttribute('placeholder', '无鉴权端点可留空')
    expect(screen.getByLabelText('模型 API URL')).toHaveAttribute(
      'placeholder',
      'https://your-relay.com/v1',
    )
  })

  it('名称为空 / URL 为空时不发 IPC，提示就是字段名', async () => {
    await openCreateForm()

    fireEvent.click(screen.getByRole('button', { name: '创建' }))
    expect(await screen.findByText('请填写自定义名称')).toBeInTheDocument()
    expect(createCustomProvider).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '我的中转站' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))
    expect(await screen.findByText('请填写模型 API URL')).toBeInTheDocument()
    expect(createCustomProvider).not.toHaveBeenCalled()
  })

  it('保存落盘（create_custom_provider）后实例立刻出现在左栏，无关键可留空', async () => {
    vi.mocked(createCustomProvider).mockImplementation(
      async (name, displayName, providerType, baseUrl) => {
        const created = provider({
          id: name,
          name: displayName,
          display_name: displayName,
          provider_type: providerType,
          base_url: baseUrl,
        })
        providerList = [...providerList, created]
        return created as never
      },
    )

    await openCreateForm()
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '我的中转站' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay.example/v1' },
    })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    await waitFor(() =>
      expect(createCustomProvider).toHaveBeenCalledWith(
        expect.stringMatching(/^custom-\d+$/), // 纯中文名 → 时间戳段 id（合法 ASCII）
        '我的中转站',
        'custom',
        'https://relay.example/v1',
        '', // API Key 留空 = 无鉴权端点
        [], // 未添加标头 → 空数组（后端不写 extra_headers 键）
        null, // 未配 OAuth → null（该实例走静态密钥）
      ),
    )
    // 落盘后重新拉取列表 → 左栏出现该实例（S1），且显示的是用户填的名称而非段 id
    expect(await screen.findByRole('button', { name: /^我的中转站$/ })).toBeInTheDocument()
    // 创建成功即进入该实例的编辑视图（按钮「保存」），不再是创建态
    expect(await screen.findByRole('button', { name: '保存' })).toBeInTheDocument()
  })

  it('标头随创建上报：表单里添加的自定义标头以二元组数组透传后端', async () => {
    vi.mocked(createCustomProvider).mockImplementation(async (name, displayName) => {
      const created = provider({
        id: name,
        name: displayName,
        display_name: displayName,
        provider_type: 'custom',
        base_url: 'https://relay.example/v1',
      })
      providerList = [...providerList, created]
      return created as never
    })

    await openCreateForm()
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '网关中转' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay.example/v1' },
    })
    // 添加两行标头：一行填全，一行只填名称（value 可为空串）
    fireEvent.click(screen.getByRole('button', { name: '+ 添加标头' }))
    fireEvent.click(screen.getByRole('button', { name: '+ 添加标头' }))
    fireEvent.change(screen.getByLabelText('自定义标头 1 名称'), {
      target: { value: 'X-Gateway' },
    })
    fireEvent.change(screen.getByLabelText('自定义标头 1 值'), { target: { value: 'nuphus' } })
    fireEvent.change(screen.getByLabelText('自定义标头 2 名称'), {
      target: { value: 'X-Trace' },
    })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    await waitFor(() =>
      expect(createCustomProvider).toHaveBeenCalledWith(
        expect.any(String),
        '网关中转',
        'custom',
        'https://relay.example/v1',
        '',
        [
          ['X-Gateway', 'nuphus'],
          ['X-Trace', ''],
        ],
        null, // 未配 OAuth → null
      ),
    )
  })

  it('创建：订阅账号配置随创建上报为 camelCase DTO（回调端口转数字、PKCE 缺省 true）', async () => {
    await openCreateForm()
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '订阅网关' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://sso-relay.example/v1' },
    })
    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    fireEvent.change(screen.getByLabelText('授权端点'), {
      target: { value: 'https://sso.example.com/authorize' },
    })
    fireEvent.change(screen.getByLabelText('令牌端点'), {
      target: { value: 'https://sso.example.com/token' },
    })
    fireEvent.change(screen.getByLabelText('Client ID'), { target: { value: 'cid-1' } })
    fireEvent.change(screen.getByLabelText('回调端口'), { target: { value: '8787' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    await waitFor(() =>
      expect(createCustomProvider).toHaveBeenCalledWith(
        expect.any(String),
        '订阅网关',
        'custom',
        'https://sso-relay.example/v1',
        '',
        [],
        {
          // 与后端 OauthConfigDto 字段一一对应（camelCase；redirectPort = number|null）
          authorizeUrl: 'https://sso.example.com/authorize',
          tokenUrl: 'https://sso.example.com/token',
          clientId: 'cid-1',
          scopes: '',
          usePkce: true,
          redirectPort: 8787,
        },
      ),
    )
  })

  it('旧 custom 段不再作为左栏条目出现，具名实例紧随 Custom 入口之后', async () => {
    providerList = [
      provider({ id: 'deepseek', name: 'DeepSeek', base_url: 'https://api.deepseek.com' }),
      provider({
        id: 'custom',
        name: 'custom',
        provider_type: 'custom',
        base_url: 'https://legacy.example/v1',
      }),
      provider({
        id: 'custom-my-relay',
        name: '我的中转站',
        display_name: '我的中转站',
        provider_type: 'custom',
        base_url: 'https://relay.example/v1',
      }),
    ]

    const { container } = render(<ModelsPage onClose={() => {}} />)
    await screen.findByRole('button', { name: /^我的中转站$/ })

    const groups = [...container.querySelectorAll('.models-rail-group')].map(g => ({
      title: g.querySelector('.models-rail-group-title')?.textContent?.trim() ?? '',
      items: [...g.querySelectorAll('.models-rail-item')].map(
        b => b.querySelector('.models-rail-name')?.textContent?.trim() ?? '',
      ),
    }))
    const customGroup = groups.find(g => g.title === '自定义模型')

    // 「自定义模型」组最终形态：custom 配置入口恒在 + 具名实例；旧段名绝不出现
    expect(customGroup?.items[0]).toBe('创建（自定义/中转站）')
    expect(customGroup?.items).toContain('我的中转站')
    expect(customGroup?.items.filter(i => i === 'custom')).toHaveLength(0)
    // 「+ 新建」按钮已移除
    expect(customGroup?.items.some(i => i.includes('+ 新建'))).toBe(false)
  })

  it('Anthropic 兼容实例：不出现「连接」，改为手动填模型名 + 一行如实说明', async () => {
    providerList = [
      ...providerList,
      provider({
        id: 'custom-claude-relay',
        name: 'Claude 中转',
        display_name: 'Claude 中转',
        provider_type: 'anthropic',
        base_url: 'https://claude-relay.example/v1',
      }),
    ]

    render(<ModelsPage onClose={() => {}} />)
    fireEvent.click(await screen.findByRole('button', { name: /^Claude 中转$/ }))

    expect(
      await screen.findByText('Anthropic 协议不支持自动获取模型列表，请手动填写模型名'),
    ).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '连接测试' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '+ 手动添加模型名' })).toBeInTheDocument()
    // 名称不再是只读展示：同一套表单里是可编辑输入框，且预填当前实例名称
    const nameInput = screen.getByLabelText('自定义名称')
    expect(nameInput.tagName).toBe('INPUT')
    expect(nameInput).toHaveValue('Claude 中转')
    // 「模型提供商」是下拉，选中项就是该实例已存的协议
    expect((screen.getByLabelText('模型提供商') as HTMLSelectElement).value).toBe('anthropic')
  })

  /**
   * 编辑自定义模型（统一表单回归）：
   * 名称一屏只出现一次（左栏除外）、是可编辑输入框、且保存走 update_custom_provider；
   * 名称变化只改 display_name —— 段 id 由后端保持稳定。
   */
  it('编辑：名称是输入框、可改，保存走 update_custom_provider 且不改段 id', async () => {
    providerList = [
      ...providerList,
      provider({
        id: 'custom-my-relay',
        name: '我的中转站',
        display_name: '我的中转站',
        provider_type: 'custom',
        base_url: 'https://relay.example/v1',
      }),
    ]
    vi.mocked(updateCustomProvider).mockImplementation(
      async (name, displayName, providerType, baseUrl) => {
        const updated = provider({
          id: name,
          name: displayName,
          display_name: displayName,
          provider_type: providerType,
          base_url: baseUrl,
        })
        // 落盘是唯一真值：重拉列表后左栏显示新名称（重启后从 providers.toml 读回同一值）
        providerList = providerList.map(p => (p.id === name ? updated : p))
        return updated as never
      },
    )

    const { container } = render(<ModelsPage onClose={() => {}} />)
    fireEvent.click(await screen.findByRole('button', { name: /^我的中转站$/ }))

    // ① 整屏只有一个「自定义名称」输入框，值是当前名称
    const nameInput = await screen.findByLabelText('自定义名称')
    expect(screen.getAllByLabelText('自定义名称')).toHaveLength(1)
    expect(nameInput.tagName).toBe('INPUT')
    expect(nameInput).toHaveValue('我的中转站')
    // ② 主内容区不再有第二个名字（配置块只留状态）
    const main = container.querySelector('.models-main') as HTMLElement
    expect(main.querySelectorAll('.models-provider-current-name')).toHaveLength(0)
    expect(main.textContent).not.toContain('我的中转站')

    // ③ 改名 + 换协议 + 换地址：Key 留空（保持原密钥）
    fireEvent.change(nameInput, { target: { value: '新名字' } })
    fireEvent.change(screen.getByLabelText('模型提供商'), { target: { value: 'anthropic' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay2.example/v1' },
    })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    await waitFor(() =>
      expect(updateCustomProvider).toHaveBeenCalledWith(
        'custom-my-relay', // 段 id 原样传入：重命名不动路由身份
        '新名字',
        'anthropic',
        'https://relay2.example/v1',
        '', // 留空 = 保持原密钥，不是清空
        [], // 未动标头 → 空数组（全量提交契约：空 = 清除）
        null, // 未动 OAuth 区块 → null = 不动已存 OAuth 配置（后端 update 语义）
      ),
    )
    // ④ 左栏立即显示新名称
    expect(await screen.findByRole('button', { name: /^新名字$/ })).toBeInTheDocument()
  })

  it('编辑：密钥已配置时占位提示「留空则保持原密钥不变」，官方服务商页保持原样', async () => {
    providerList = [
      ...providerList,
      provider({
        id: 'custom-keyed',
        name: '带密钥的中转站',
        display_name: '带密钥的中转站',
        provider_type: 'custom',
        base_url: 'https://keyed.example/v1',
      }),
    ]
    vi.mocked(getCurrentConfig).mockResolvedValue({
      model: 'deepseek-flash',
      provider: 'deepseek',
      base_url: 'https://api.deepseek.com',
      has_key: true,
      configured_providers: ['deepseek', 'custom-keyed'],
    } as never)

    const { container } = render(<ModelsPage onClose={() => {}} />)
    fireEvent.click(await screen.findByRole('button', { name: /^带密钥的中转站$/ }))

    expect(await screen.findByLabelText('模型 API Key')).toHaveAttribute(
      'placeholder',
      '留空则保持原密钥不变',
    )

    // 官方 provider 页不回归：只读名称 + 原有密钥栏，无自定义四字段表单
    fireEvent.click(screen.getByRole('button', { name: /DeepSeek/ }))
    await waitFor(() => expect(screen.queryByLabelText('自定义名称')).not.toBeInTheDocument())
    expect(container.querySelector('.models-provider-current-name')?.textContent).toBe('DeepSeek')
    expect(screen.getByLabelText('API 密钥')).toBeInTheDocument()
    expect(screen.queryByLabelText('模型 API URL')).not.toBeInTheDocument()
  })

  /**
   * 分组归属回归（用户视角，不是实现视角）：
   * 本地模型跑在用户自己机器上，不该混进「模型提供商」（云端）列表里让人去云厂商中找；
   * Opencode GO 是远程付费网关，属「别人提供的服务」，留在模型提供商组。
   */
  it('本地模型独立成组，不出现在云端「模型提供商」列表里', async () => {
    providerList = [
      provider({ id: 'deepseek', name: 'DeepSeek', base_url: 'https://api.deepseek.com' }),
      provider({ id: 'opencode-go', name: 'Opencode GO 套餐', base_url: 'https://go.example/v1' }),
      provider({ id: 'local', name: 'Local', base_url: 'http://localhost:11434/v1' }),
    ]

    const { container } = render(<ModelsPage onClose={() => {}} />)
    await screen.findByRole('button', { name: /DeepSeek/ })

    const groups = [...container.querySelectorAll('.models-rail-group')].map(g => ({
      title: g.querySelector('.models-rail-group-title')?.textContent?.trim() ?? '',
      items: [...g.querySelectorAll('.models-rail-item')].map(
        b => b.querySelector('.models-rail-name')?.textContent?.trim() ?? '',
      ),
    }))

    const cloud = groups.find(g => g.title === '模型提供商')
    const local = groups.find(g => g.title === '本地模型')

    // 本地模型必须在自己的组里
    expect(local).toBeDefined()
    expect(local?.items).toEqual(['本地模型'])
    // 且绝不出现在云端提供商列表
    expect(cloud).toBeDefined()
    expect(cloud?.items.some(i => /local/i.test(i))).toBe(false)
    // 远程网关（Opencode GO）仍属模型提供商
    expect(cloud?.items).toContain('Opencode GO 套餐')
  })

  // ════════════════════════════════════════════════════════════════
  // 订阅账号（OAuth）：编辑态登录交互
  //
  // 登录态真值在磁盘（前端只能拿状态，永远拿不到令牌）；授权结果不由 oauth_begin
  // 返回 —— 后端在浏览器回调到达后推 `oauth-login-result` 事件，前端按 provider
  // 匹配当前实例再刷新状态。
  // ════════════════════════════════════════════════════════════════

  /** 登录结果事件载荷（与后端 emit 的 payload 同形） */
  type OauthLoginResultPayload = { provider: string; ok: boolean; error: string | null }

  /** 取最近一次注册的登录结果回调（订阅发生在组件内部，测试手动触发） */
  function lastLoginResultHandler() {
    const calls = vi.mocked(listen).mock.calls
    const handler = calls[calls.length - 1]?.[1] as unknown as
      ((e: { payload: OauthLoginResultPayload }) => void) | undefined
    if (!handler) throw new Error('oauth-login-result 监听未注册')
    return handler
  }

  /** 铺一个已配 OAuth 的自定义实例，并停在它的编辑页 */
  async function openOauthInstance() {
    providerList = [
      ...providerList,
      provider({
        id: 'custom-sso',
        name: '公司订阅',
        display_name: '公司订阅',
        provider_type: 'custom',
        base_url: 'https://sso-relay.example/v1',
      }),
    ]
    const view = render(<ModelsPage onClose={() => {}} />)
    fireEvent.click(await screen.findByRole('button', { name: /^公司订阅$/ }))
    await screen.findByLabelText('自定义名称')
    return view
  }

  it('编辑：授权登录 → oauth_begin + openExternal，事件回填后刷新登录态', async () => {
    vi.mocked(oauthStatus).mockResolvedValue({
      configured: true,
      logged_in: false,
      expires_at: null,
      needs_login: false,
    } as never)
    vi.mocked(oauthBegin).mockResolvedValue({
      authorize_url: 'https://sso.example.com/authorize?state=abc',
      port: 8765,
    } as never)

    await openOauthInstance()

    // 已配 oauth → 订阅账号区块自动展开，未登录时给「授权登录」
    const loginBtn = await screen.findByRole('button', { name: '授权登录' })
    fireEvent.click(loginBtn)

    await waitFor(() => expect(oauthBegin).toHaveBeenCalledWith('custom-sso'))
    await waitFor(() =>
      expect(openExternal).toHaveBeenCalledWith('https://sso.example.com/authorize?state=abc'),
    )
    expect(
      await screen.findByText('已打开浏览器授权页，完成后本页自动更新登录状态'),
    ).toBeInTheDocument()

    // 别的实例的授权结果不落到本页（同一事件通道上有多个实例在登录）
    const handler = lastLoginResultHandler()
    act(() => {
      handler({ payload: { provider: 'custom-other', ok: true, error: null } })
    })
    expect(screen.queryByText('授权成功')).not.toBeInTheDocument()

    // 本实例授权成功 → 重读状态（已登录 + 有效期）并给出成功反馈
    vi.mocked(oauthStatus).mockResolvedValue({
      configured: true,
      logged_in: true,
      expires_at: 4102444800, // 2100-01-01T00:00:00Z
      needs_login: false,
    } as never)
    await act(async () => {
      handler({ payload: { provider: 'custom-sso', ok: true, error: null } })
    })

    expect(await screen.findByText(/^已登录 · 有效期至 /)).toBeInTheDocument()
    expect(screen.getByText('授权成功')).toBeInTheDocument()
    await waitFor(() => expect(oauthStatus).toHaveBeenCalledWith('custom-sso'))
  })

  it('编辑：授权失败展示后端 error 原文，卸载时退订事件监听', async () => {
    vi.mocked(oauthStatus).mockResolvedValue({
      configured: true,
      logged_in: false,
      expires_at: null,
      needs_login: true,
    } as never)
    vi.mocked(oauthBegin).mockResolvedValue({
      authorize_url: 'https://sso.example.com/authorize?state=abc',
      port: 8765,
    } as never)

    const view = await openOauthInstance()

    // needs_login → 摘要如实说「需重新授权」
    expect(screen.getByText('需重新授权')).toBeInTheDocument()
    fireEvent.click(await screen.findByRole('button', { name: '授权登录' }))
    await waitFor(() => expect(oauthBegin).toHaveBeenCalledWith('custom-sso'))

    act(() => {
      lastLoginResultHandler()({
        payload: { provider: 'custom-sso', ok: false, error: '授权被拒绝: access_denied' },
      })
    })
    expect(await screen.findByText('授权被拒绝: access_denied')).toBeInTheDocument()

    // 卸载 → 必须退订（否则旧实例的监听会一直挂着）
    await waitFor(() => expect(listen).toHaveBeenCalled())
    view.unmount()
    await waitFor(() => expect(unlistenMock).toHaveBeenCalled())
  })

  it('编辑：已登录时「退出登录」走 oauth_logout 并刷新状态', async () => {
    vi.mocked(oauthStatus).mockResolvedValue({
      configured: true,
      logged_in: true,
      expires_at: 4102444800,
      needs_login: false,
    } as never)

    await openOauthInstance()
    await screen.findByText(/^已登录 · 有效期至 /)

    // 退出登录后回到「已配置但未登录」
    vi.mocked(oauthStatus).mockResolvedValue({
      configured: true,
      logged_in: false,
      expires_at: null,
      needs_login: false,
    } as never)
    fireEvent.click(screen.getByRole('button', { name: '退出登录' }))

    await waitFor(() => expect(oauthLogout).toHaveBeenCalledWith('custom-sso'))
    expect(await screen.findByText('已退出登录')).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: '授权登录' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '退出登录' })).not.toBeInTheDocument()
  })
})
