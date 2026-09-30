import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { listen } from '@tauri-apps/api/event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  configureLlm,
  listProviderModels,
  switchModel,
  createCustomProvider,
  getSupportedProviders,
  oauthBegin,
  oauthLogout,
  oauthStatus,
  openExternal,
  removeCustomProvider,
  updateCustomProvider,
} from '../lib/api'
import { ModelsPage } from './ModelsPage'

/**
 * 自定义模型（自定义中转站）新建流程回归：
 * - 「+ 新建」只有四个输入项：自定义名称 / 模型提供商 / 模型 API Key / 模型 API URL；
 * - 名称与 URL 为空必须当场拦下（不发 IPC），错误文案就是字段名本身；
 * - 保存走 create_custom_provider 真正落盘，成功后实例立刻出现在左栏（不是只改前端状态）；
 * - Anthropic 兼容实例同样有「连接」/「刷新」入口（后端与协议无关），手动添加是兜底。
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
  setModelSupportsImageGeneration: vi.fn(),
  setCapabilityBinding: vi.fn(),
  createCustomProvider: vi.fn(),
  updateCustomProvider: vi.fn(),
  removeCustomProvider: vi.fn(),
  oauthBegin: vi.fn(),
  oauthStatus: vi.fn(),
  oauthLogout: vi.fn(),
  openExternal: vi.fn(),
  sttStatus: vi.fn(),
}))

vi.mock('./JevSettings', () => ({
  JevSettings: () => <div data-testid="enhanced-judgment-settings">增强判断模型配置内容</div>,
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
  vi.mocked(listProviderModels).mockReset()
  vi.mocked(configureLlm).mockReset()
  vi.mocked(switchModel).mockReset()
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
  // 删除：默认成功并报告「已删掉一段」。configured_providers 同步去掉该项。
  vi.mocked(removeCustomProvider).mockResolvedValue(true)
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

describe('ModelsPage 增强判断模型导航', () => {
  it('支持从增强模式提示直接打开对应子页', async () => {
    render(<ModelsPage onClose={() => {}} initialView="jev" />)

    const entry = await screen.findByRole('button', { name: '增强判断模型' })
    expect(entry).toHaveClass('active')
    expect(screen.getByTestId('enhanced-judgment-settings')).toBeInTheDocument()
  })
})

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

  it('Anthropic 兼容实例：连接/刷新入口齐全，手动添加作为兜底', async () => {
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

    // 曾以「Anthropic 协议没有 /v1/models」为由隐藏连接与刷新，只留手动填模型名；
    // 后端 fetch_provider_models 与协议无关（{base}/models + x-api-key），实测可用，
    // 故入口必须齐全。手动添加保留为兜底（中转自定义别名未必出现在列表里）。
    expect(await screen.findByText(/点「刷新」从中转站拉取模型列表/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '连接测试' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '刷新' })).toBeInTheDocument()
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

/**
 * 删除自定义模型实例回归。
 *
 * 覆盖四件事，按「少了任何一条用户就会踩坑」挑选：
 * 1. 删除是**二次确认**的——点图标不能直接删（误触即丢配置）；
 * 2. 取消即什么都不发生（IPC 一次都不能发）；
 * 3. 确认后才真落盘，且**整段移除**（remove_custom_provider，不是清模型列表）；
 * 4. 左栏条目随之消失——这是「真的删了」的唯一可见证据。
 */
describe('ModelsPage 删除自定义模型实例', () => {
  /** 左栏渲染两个自定义实例，其中一个是「已配置密钥」态 */
  function seedInstances() {
    providerList = [
      provider({ id: 'deepseek', name: 'DeepSeek', base_url: 'https://api.deepseek.com' }),
      provider({
        id: 'custom-relay-a',
        name: 'custom-relay-a',
        display_name: '中转站 A',
        provider_type: 'custom',
        base_url: 'https://relay-a.example/v1',
      }),
      provider({
        id: 'custom-relay-b',
        name: 'custom-relay-b',
        display_name: '中转站 B',
        provider_type: 'custom',
        base_url: 'https://relay-b.example/v1',
      }),
    ] as typeof providerList
  }

  beforeEach(() => {
    seedInstances()
  })

  it('点删除图标弹二次确认，取消则不发 IPC、条目保留', async () => {
    render(<ModelsPage onClose={() => {}} />)
    const del = await screen.findByRole('button', { name: '删除「中转站 A」' })
    fireEvent.click(del)

    // 弹窗出现，且讲明了后果（不可撤销 + 移除全部设置）
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('删除自定义模型')
    expect(dialog).toHaveTextContent('中转站 A')
    expect(dialog).toHaveTextContent('无法撤销')

    fireEvent.click(screen.getByRole('button', { name: '取消' }))

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(removeCustomProvider).not.toHaveBeenCalled()
    // 条目仍在：取消必须是真正的「什么都没发生」
    expect(await screen.findByRole('button', { name: '删除「中转站 A」' })).toBeInTheDocument()
  })

  it('确认后走 remove_custom_provider 落盘，条目从左栏消失', async () => {
    // 删除成功后服务商列表少一段：模拟后端真实行为（磁盘是唯一真值）
    vi.mocked(removeCustomProvider).mockImplementation(async () => {
      providerList = providerList.filter(p => p.id !== 'custom-relay-a')
      return true
    })

    render(<ModelsPage onClose={() => {}} />)
    fireEvent.click(await screen.findByRole('button', { name: '删除「中转站 A」' }))
    fireEvent.click(await screen.findByRole('button', { name: '删除' }))

    await waitFor(() => expect(removeCustomProvider).toHaveBeenCalledWith('custom-relay-a'))
    // 删除的必须是段 id（路由依据），不是显示名
    expect(vi.mocked(removeCustomProvider).mock.calls[0][0]).toBe('custom-relay-a')
    // 左栏条目消失、另一个实例不受影响
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: '删除「中转站 A」' })).not.toBeInTheDocument(),
    )
    expect(screen.getByRole('button', { name: '删除「中转站 B」' })).toBeInTheDocument()
    expect(await screen.findByText('已删除「中转站 A」')).toBeInTheDocument()
  })

  it('删除失败时弹窗不关、错误原文展示、条目保留', async () => {
    vi.mocked(removeCustomProvider).mockRejectedValue(
      'write config.toml failed: 拒绝访问 (os error 5)',
    )

    render(<ModelsPage onClose={() => {}} />)
    fireEvent.click(await screen.findByRole('button', { name: '删除「中转站 B」' }))
    fireEvent.click(await screen.findByRole('button', { name: '删除' }))

    // 失败不得静默：弹窗留着让用户看到原因，条目也不得提前消失
    expect(await screen.findByText(/write config\.toml failed/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '取消' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '删除「中转站 B」' })).toBeInTheDocument()
  })

  it('Esc 关闭确认弹窗且不发 IPC', async () => {
    render(<ModelsPage onClose={() => {}} />)
    fireEvent.click(await screen.findByRole('button', { name: '删除「中转站 A」' }))
    expect(await screen.findByRole('dialog')).toBeInTheDocument()

    fireEvent.keyDown(window, { key: 'Escape' })

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(removeCustomProvider).not.toHaveBeenCalled()
  })

  it('删除当前选中的实例后，右栏不残留已删除实例', async () => {
    vi.mocked(removeCustomProvider).mockImplementation(async () => {
      providerList = providerList.filter(p => p.id !== 'custom-relay-a')
      return true
    })

    render(<ModelsPage onClose={() => {}} />)
    // 先选中「中转站 A」：点左栏主区域（不是删除图标）。
    // 用子串匹配：主按钮 title 是 instanceIdTitle 包装过的串，不是裸 id。
    fireEvent.click(await screen.findByTitle(/custom-relay-a/))
    expect(await screen.findByDisplayValue('https://relay-a.example/v1')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '删除「中转站 A」' }))
    fireEvent.click(await screen.findByRole('button', { name: '删除' }))

    await waitFor(() => expect(removeCustomProvider).toHaveBeenCalled())
    // 右栏已删除实例的地址必须消失（否则用户会对着一个不存在的中转站改配置）
    await waitFor(() =>
      expect(screen.queryByDisplayValue('https://relay-a.example/v1')).not.toBeInTheDocument(),
    )
  })
})

describe('ModelsPage 连接反馈与快速切换', () => {
  const brief = (id: string) => ({
    id,
    supports_streaming: true,
    supports_vision: false,
    supports_audio: false,
    supports_image_generation: false,
  })
  function deferred<T>() {
    let resolve!: (value: T) => void
    let reject!: (reason: Error) => void
    const promise = new Promise<T>((res, rej) => {
      resolve = res
      reject = rej
    })
    return { promise, resolve, reject }
  }
  async function openModels(onModelChanged = vi.fn()) {
    localStorage.clear()
    providerList.push(
      provider({
        id: 'custom-fast',
        name: '快速中转站',
        display_name: '快速中转站',
        provider_type: 'custom',
        base_url: 'https://relay.example/v1',
      }),
    )
    vi.mocked(getProviderBaseUrl).mockImplementation(async id =>
      id === 'custom-fast' ? 'https://relay.example/v1' : null,
    )
    const result = render(<ModelsPage onClose={() => {}} onModelChanged={onModelChanged} />)
    fireEvent.click(await screen.findByRole('button', { name: '快速中转站' }))
    await waitFor(() =>
      expect(screen.getByLabelText('模型 API URL')).toHaveValue('https://relay.example/v1'),
    )
    return result
  }
  async function detect() {
    vi.mocked(listProviderModels).mockResolvedValue([brief('model-one'), brief('model-two')])
    fireEvent.click(screen.getByRole('button', { name: '连接测试' }))
    await screen.findByText('model-two')
  }
  const row = (name: string) => screen.getByText(name).closest('[role="button"]') as HTMLElement

  it('连接成功明确提示数量，并在修改参数后清除过时提示', async () => {
    await openModels()
    await detect()
    expect(screen.getByRole('status')).toHaveTextContent('已获取到 2 个模型')
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://new.example/v1' },
    })
    expect(screen.queryByText('已获取到 2 个模型')).not.toBeInTheDocument()
  })

  it.each(['empty', 'error'])('连接结果为 %s 时不显示成功', async kind => {
    await openModels()
    if (kind === 'empty') vi.mocked(listProviderModels).mockResolvedValue([])
    else vi.mocked(listProviderModels).mockRejectedValue(new Error('无法连接服务商'))
    fireEvent.click(screen.getByRole('button', { name: '连接测试' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      kind === 'empty' ? 'API 未返回任何可用模型' : '无法连接服务商',
    )
    expect(screen.queryByText(/已获取到/)).not.toBeInTheDocument()
  })

  it('切换配置后忽略迟到的连接结果', async () => {
    await openModels()
    const pending = deferred<ReturnType<typeof brief>[]>()
    vi.mocked(listProviderModels).mockReturnValue(pending.promise)
    fireEvent.click(screen.getByRole('button', { name: '连接测试' }))
    fireEvent.click(screen.getByRole('button', { name: 'DeepSeek' }))
    await act(async () => pending.resolve([brief('stale-model')]))
    expect(screen.queryByText('stale-model')).not.toBeInTheDocument()
    expect(screen.queryByText(/已获取到/)).not.toBeInTheDocument()
  })

  it('切换立即显示状态，保存期间阻止重复提交，成功后更新选择', async () => {
    const onChanged = vi.fn()
    await openModels(onChanged)
    await detect()
    const pending = deferred<string>()
    vi.mocked(switchModel).mockReturnValue(pending.promise)
    fireEvent.click(row('model-two'))
    expect(screen.getByText('正在切换到 model-two…')).toBeInTheDocument()
    expect(row('model-two')).toHaveAttribute('aria-busy', 'true')
    expect(row('model-two')).not.toHaveClass('active')
    fireEvent.click(row('model-one'))
    expect(switchModel).toHaveBeenCalledTimes(1)
    await act(async () => pending.resolve('ok'))
    expect(row('model-two')).toHaveClass('active')
    expect(screen.getByText('已切换到 model-two')).toBeInTheDocument()
    expect(onChanged).toHaveBeenCalledTimes(1)
  })

  it('切换失败保留原选择，支持 Enter 和空格切换', async () => {
    await openModels()
    await detect()
    vi.mocked(switchModel).mockResolvedValueOnce('ok')
    fireEvent.keyDown(row('model-one'), { key: 'Enter' })
    await waitFor(() => expect(row('model-one')).toHaveClass('active'))
    vi.mocked(switchModel).mockRejectedValueOnce(new Error('保存失败'))
    fireEvent.keyDown(row('model-two'), { key: ' ' })
    await screen.findByText('保存失败')
    expect(row('model-one')).toHaveClass('active')
    expect(row('model-two')).not.toHaveClass('active')
  })

  it('输入密钥时复用 configureLlm，重复选择仍可保存修改后的连接参数', async () => {
    await openModels()
    await detect()
    vi.mocked(configureLlm).mockResolvedValue('ok')
    fireEvent.change(screen.getByLabelText('模型 API Key'), { target: { value: 'test-key' } })
    fireEvent.click(row('model-two'))
    await waitFor(() => expect(row('model-two')).toHaveClass('active'))
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://new.example/v1' },
    })
    fireEvent.click(row('model-two'))
    await waitFor(() => expect(configureLlm).toHaveBeenCalledTimes(2))
    expect(configureLlm).toHaveBeenLastCalledWith(
      'test-key',
      'model-two',
      'custom-fast',
      'https://new.example/v1',
      undefined,
    )
    expect(switchModel).not.toHaveBeenCalled()
  })

  it('后台能力更新会刷新模型列表，卸载后释放监听', async () => {
    const { unmount } = await openModels()
    await waitFor(() =>
      expect(vi.mocked(listen).mock.calls.some(c => c[0] === 'model-metadata-updated')).toBe(true),
    )
    const handler = vi.mocked(listen).mock.calls.find(c => c[0] === 'model-metadata-updated')![1]
    const before = vi.mocked(listModels).mock.calls.length
    await act(async () => {
      handler({ payload: { provider: 'custom-fast', model: 'model-two' } } as never)
    })
    expect(listModels).toHaveBeenCalledTimes(before + 1)
    unmount()
    expect(unlistenMock).toHaveBeenCalled()
  })
})
