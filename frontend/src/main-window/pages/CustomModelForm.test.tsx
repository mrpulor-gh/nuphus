import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { CustomModelForm } from './ModelsPage'

/**
 * CustomModelForm —— 新建与编辑的**同一套模板**。
 *
 * 本文件锁死三件事（回归价值来自任务背景：两处曾各写一份 JSX，导致同一个名字
 * 一屏出现两次、都不可改，且字段迟早漂移）：
 * 1. 两种模式渲染出的字段标签 / 顺序完全一致（同一份 JSX，不是两份长得像）；
 * 2. 编辑模式的名称是**输入框**、预填当前名称，可改并随提交上报；
 * 3. 两种模式校验同源（名称为空 / 地址为空当场拦下，不发提交）。
 */

/** 表单字段标签集合（FormRow 的 label 文本）——直接反映"字段顺序 + 文案" */
function fieldLabels(container: HTMLElement): string[] {
  return [...container.querySelectorAll('.form-row-label')].map(el => el.textContent?.trim() ?? '')
}

const EDIT_INITIAL = {
  displayName: '我的中转站',
  providerType: 'anthropic',
  baseUrl: 'https://relay.example/v1',
}

function renderForm(props: Partial<Parameters<typeof CustomModelForm>[0]> = {}) {
  const onSubmit = vi.fn()
  const onValuesChange = vi.fn()
  const view = render(
    <CustomModelForm
      mode="create"
      saving={false}
      error={null}
      onSubmit={onSubmit}
      onValuesChange={onValuesChange}
      {...props}
    />,
  )
  return { ...view, onSubmit, onValuesChange }
}

describe('CustomModelForm（新建 / 编辑共用一套模板）', () => {
  it('新建与编辑渲染出的字段标签集合与顺序完全一致', () => {
    const create = renderForm({ mode: 'create' })
    const createLabels = fieldLabels(create.container)
    create.unmount()

    const edit = renderForm({ mode: 'edit', initial: EDIT_INITIAL, hasKey: true })
    const editLabels = fieldLabels(edit.container)

    expect(createLabels).toEqual(editLabels)
    expect(createLabels).toEqual([
      '自定义名称',
      '模型提供商',
      '模型 API Key',
      '模型 API URL',
      '自定义标头',
    ])
  })

  it('编辑模式：名称是输入框、预填当前名称，且可改后随提交上报', () => {
    const { onSubmit } = renderForm({ mode: 'edit', initial: EDIT_INITIAL, hasKey: true })

    const name = screen.getByLabelText('自定义名称')
    expect(name.tagName).toBe('INPUT')
    expect(name).toHaveValue('我的中转站')
    expect(name).toHaveAttribute('placeholder', '例如：公司网关')

    // 协议下拉选中当前值；地址预填当前值
    expect((screen.getByLabelText('模型提供商') as HTMLSelectElement).value).toBe('anthropic')
    expect(screen.getByLabelText('模型 API URL')).toHaveValue('https://relay.example/v1')

    // 名称可改 → 提交时上报新名字（段 id 由调用方按原 id 传后端，表单不碰）
    fireEvent.change(name, { target: { value: '新名字' } })
    fireEvent.change(screen.getByLabelText('模型提供商'), { target: { value: 'custom' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    expect(onSubmit).toHaveBeenCalledWith({
      displayName: '新名字',
      providerType: 'custom',
      baseUrl: 'https://relay.example/v1',
      apiKey: '',
      headers: [],
      // 订阅账号未配置（三项必填全空）→ 上报 null，落盘时该实例走静态密钥
      oauth: null,
    })
  })

  it('编辑模式：Key 留空 = 保持原密钥（占位说明），非空才随提交上报', () => {
    const { onSubmit } = renderForm({ mode: 'edit', initial: EDIT_INITIAL, hasKey: true })

    const key = screen.getByLabelText('模型 API Key')
    expect(key).toHaveAttribute('type', 'password')
    expect(key).toHaveAttribute('placeholder', '留空则保持原密钥不变')

    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(onSubmit).toHaveBeenLastCalledWith(expect.objectContaining({ apiKey: '' }))

    fireEvent.change(key, { target: { value: 'sk-new' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(onSubmit).toHaveBeenLastCalledWith(expect.objectContaining({ apiKey: 'sk-new' }))
  })

  it('新建模式：默认 OpenAI 兼容、Key 占位「无鉴权端点可留空」、主按钮是「创建」', () => {
    renderForm({ mode: 'create' })

    expect((screen.getByLabelText('模型提供商') as HTMLSelectElement).value).toBe('custom')
    expect(screen.getByLabelText('模型 API Key')).toHaveAttribute('placeholder', '无鉴权端点可留空')
    expect(screen.getByLabelText('模型 API URL')).toHaveAttribute(
      'placeholder',
      'https://your-relay.com/v1',
    )
    expect(screen.getByRole('button', { name: '创建' })).toBeInTheDocument()
  })

  it('校验同源：名称为空 / 地址为空当场拦下，提示就是字段名且不发提交', () => {
    const { onSubmit } = renderForm({ mode: 'create' })

    fireEvent.click(screen.getByRole('button', { name: '创建' }))
    expect(screen.getByText('请填写自定义名称')).toBeInTheDocument()
    expect(onSubmit).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '我的中转站' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))
    expect(screen.getByText('请填写模型 API URL')).toBeInTheDocument()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('字段改动会单向上报（供页面「连接 / 刷新 / 地址变更提示」使用）', () => {
    const { onValuesChange } = renderForm({ mode: 'edit', initial: EDIT_INITIAL, hasKey: true })

    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay2.example/v1' },
    })

    expect(onValuesChange).toHaveBeenLastCalledWith(
      expect.objectContaining({ baseUrl: 'https://relay2.example/v1' }),
    )
  })

  /**
   * 密钥行 DOM 结构守卫（回归来源：清除按钮曾被塞进 .models-key-field 内部，
   * 导致 input 被挤到换行、绝对定位的眼睛图标被顶出输入框）。
   *
   * CSS 约定：.models-key-field 是 position:relative 的输入框容器（内含 100% 宽 input +
   * 绝对定位的眼睛）；.models-key-clear 是流式方块，必须与 field **平级**待在 .models-key-row 里。
   * 结构错一层，布局就崩——这条测试锁死层级关系。
   */
  it('清除密钥按钮与输入框容器平级（不得塞进 .models-key-field）', () => {
    const { container } = renderForm({
      mode: 'edit',
      initial: EDIT_INITIAL,
      hasKey: true,
      onClearKey: vi.fn(),
    })

    const field = container.querySelector('.models-key-field')
    const eye = container.querySelector('.models-key-eye')
    const clear = container.querySelector('.models-key-clear')

    expect(field).toBeTruthy()
    expect(clear).toBeTruthy()
    // 眼睛图标必须留在 field 内：它是相对 field 绝对定位的（right:6px 嵌在输入框右端）
    expect(field?.contains(eye)).toBe(true)
    // 清除按钮必须在 field 之外、与 field 同为 .models-key-row 的子元素
    expect(field?.contains(clear)).toBe(false)
    expect(clear?.parentElement?.classList.contains('models-key-row')).toBe(true)
    // 且 field 的直接子元素只有 input 与眼睛，没有第三个流式元素撑破宽度
    expect(field?.children.length).toBe(2)
  })

  // ── 自定义标头编辑区（key/value 成对行，可增删） ──

  it('标头区默认无行（不塞空行噪音），只有「添加标头」入口', () => {
    renderForm({ mode: 'create' })

    expect(screen.getByRole('button', { name: '+ 添加标头' })).toBeInTheDocument()
    expect(screen.queryByLabelText('自定义标头 1 名称')).not.toBeInTheDocument()
  })

  it('已存标头回显为可编辑行，删除行后不再出现', () => {
    renderForm({
      mode: 'edit',
      initial: {
        ...EDIT_INITIAL,
        headers: [
          { name: 'X-Gateway', value: 'nuphus' },
          { name: 'X-Trace', value: 'abc' },
        ],
      },
      hasKey: true,
    })

    const name1 = screen.getByLabelText('自定义标头 1 名称') as HTMLInputElement
    const value1 = screen.getByLabelText('自定义标头 1 值') as HTMLInputElement
    expect(name1).toHaveValue('X-Gateway')
    expect(value1).toHaveValue('nuphus')
    expect(screen.getByLabelText('自定义标头 2 名称')).toHaveValue('X-Trace')

    // 行内容可编辑
    fireEvent.change(name1, { target: { value: 'X-Gateway-2' } })
    expect(name1).toHaveValue('X-Gateway-2')

    // 删除第 1 行：第 2 行前移成第 1 行
    fireEvent.click(screen.getByRole('button', { name: '删除此标头 1' }))
    expect(screen.queryByLabelText('自定义标头 2 名称')).not.toBeInTheDocument()
    expect(screen.getByLabelText('自定义标头 1 名称')).toHaveValue('X-Trace')
  })

  it('提交时过滤 name 与 value 均为空的行，其余原样上报', () => {
    const { onSubmit } = renderForm({
      mode: 'edit',
      initial: {
        ...EDIT_INITIAL,
        headers: [
          { name: 'X-Keep', value: 'v' },
          { name: '', value: '' },
        ],
      },
      hasKey: true,
    })

    // 编辑残渣：先给第 2 行补一个 value 再清空——确保它真的是「全空行」而非初始未填
    fireEvent.change(screen.getByLabelText('自定义标头 2 值'), { target: { value: 'x' } })
    fireEvent.change(screen.getByLabelText('自定义标头 2 值'), { target: { value: '' } })

    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        headers: [{ name: 'X-Keep', value: 'v' }],
      }),
    )
  })

  it('标头变更经 onValuesChange 同步上报（连接测试等消费方拿得到当前值）', () => {
    const { onValuesChange } = renderForm({ mode: 'edit', initial: EDIT_INITIAL, hasKey: true })

    fireEvent.click(screen.getByRole('button', { name: '+ 添加标头' }))
    fireEvent.change(screen.getByLabelText('自定义标头 1 名称'), {
      target: { value: 'X-New' },
    })

    expect(onValuesChange).toHaveBeenLastCalledWith(
      expect.objectContaining({
        headers: [{ name: 'X-New', value: '' }],
      }),
    )
  })

  // ════════════════════════════════════════════════════════════════
  // 订阅账号（OAuth）：可折叠区块 + 登录区
  //
  // OAuth 是可选路径：只配静态密钥的用户不该被六项 OAuth 字段挡住视线 → 默认折叠、
  // 折叠态只有一行摘要；三项必填全空 = 该实例不启用 OAuth（提交 null）。
  // ════════════════════════════════════════════════════════════════

  it('订阅账号区块默认折叠：只有开关与一行摘要，OAuth 字段不在 DOM 里', () => {
    renderForm({ mode: 'create' })

    const toggle = screen.getByRole('button', { name: /订阅账号/ })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByText('未配置')).toBeInTheDocument()
    expect(screen.queryByLabelText('授权端点')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('令牌端点')).not.toBeInTheDocument()

    fireEvent.click(toggle)
    expect(screen.getByRole('button', { name: /订阅账号/ })).toHaveAttribute(
      'aria-expanded',
      'true',
    )
    expect(screen.getByLabelText('授权端点')).toBeInTheDocument()
  })

  it('三项必填全空且可选项未动 = 不启用 OAuth：提交 null（静态密钥路径不受影响）', () => {
    const { onSubmit } = renderForm({ mode: 'create' })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '网关' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay.example/v1' },
    })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ oauth: null }))
  })

  it('编辑态回显已存 OAuth 配置：五项字段预填，可直接续改', () => {
    renderForm({
      mode: 'edit',
      initial: {
        ...EDIT_INITIAL,
        oauth: {
          authorizeUrl: 'https://sso.example.com/authorize',
          tokenUrl: 'https://sso.example.com/token',
          clientId: 'cid-1',
          scopes: 'openid profile',
          usePkce: true,
          redirectPort: '8765',
        },
      },
      hasKey: true,
    })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    expect(screen.getByLabelText('授权端点')).toHaveValue('https://sso.example.com/authorize')
    expect(screen.getByLabelText('令牌端点')).toHaveValue('https://sso.example.com/token')
    expect(screen.getByLabelText('Client ID')).toHaveValue('cid-1')
    expect(screen.getByLabelText('Scopes')).toHaveValue('openid profile')
    expect(screen.getByLabelText('回调端口')).toHaveValue('8765')
  })

  it('编辑态清空三项必填 = 明确要切回静态密钥：提交全空值（非 null），供后端移除 OAuth 配置', () => {
    const { onSubmit } = renderForm({
      mode: 'edit',
      initial: {
        ...EDIT_INITIAL,
        oauth: {
          authorizeUrl: 'https://sso.example.com/authorize',
          tokenUrl: 'https://sso.example.com/token',
          clientId: 'cid-1',
          scopes: '',
          usePkce: true,
          redirectPort: '',
        },
      },
      hasKey: true,
    })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    fireEvent.change(screen.getByLabelText('授权端点'), { target: { value: '' } })
    fireEvent.change(screen.getByLabelText('令牌端点'), { target: { value: '' } })
    fireEvent.change(screen.getByLabelText('Client ID'), { target: { value: '' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    const submitted = onSubmit.mock.calls[0][0]
    expect(submitted.oauth).not.toBeNull()
    expect(submitted.oauth).toMatchObject({ authorizeUrl: '', tokenUrl: '', clientId: '' })
  })

  it('三项必填填全 → 随提交上报 OAuth 五项（端口以文本形态上报，落盘 DTO 由调用方转换）', () => {
    const { onSubmit } = renderForm({ mode: 'create' })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '订阅网关' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay.example/v1' },
    })
    fireEvent.change(screen.getByLabelText('授权端点'), {
      target: { value: 'https://sso.example.com/authorize' },
    })
    fireEvent.change(screen.getByLabelText('令牌端点'), {
      target: { value: 'https://sso.example.com/token' },
    })
    fireEvent.change(screen.getByLabelText('Client ID'), { target: { value: 'cid-1' } })
    fireEvent.change(screen.getByLabelText('Scopes'), { target: { value: 'openid profile' } })
    fireEvent.change(screen.getByLabelText('回调端口'), { target: { value: '8765' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        oauth: {
          authorizeUrl: 'https://sso.example.com/authorize',
          tokenUrl: 'https://sso.example.com/token',
          clientId: 'cid-1',
          scopes: 'openid profile',
          usePkce: true,
          redirectPort: '8765',
        },
      }),
    )
  })

  it('PKCE 默认开启，可显式关闭；关闭状态随提交上报', () => {
    const { onSubmit } = renderForm({ mode: 'create' })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    const pkce = screen.getByLabelText('PKCE') as HTMLInputElement
    expect(pkce.checked).toBe(true)

    fireEvent.change(screen.getByLabelText('授权端点'), {
      target: { value: 'https://sso.example.com/authorize' },
    })
    fireEvent.change(screen.getByLabelText('令牌端点'), {
      target: { value: 'https://sso.example.com/token' },
    })
    fireEvent.change(screen.getByLabelText('Client ID'), { target: { value: 'cid-1' } })
    fireEvent.click(pkce)
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '网关' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay.example/v1' },
    })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ oauth: expect.objectContaining({ usePkce: false }) }),
    )
  })

  it('三项必填留空却填了可选项（端口）→ 当场拦下并说明，不发提交', () => {
    const { onSubmit } = renderForm({ mode: 'create' })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '网关' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay.example/v1' },
    })
    fireEvent.change(screen.getByLabelText('回调端口'), { target: { value: '8765' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    expect(
      screen.getByText('请先填写授权端点 / 令牌端点 / Client ID（三项必填）'),
    ).toBeInTheDocument()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('回调端口非 1-65535 整数 → 当场拦下（后端按 u16 解析，不该把解析错抛给用户）', () => {
    const { onSubmit } = renderForm({ mode: 'create' })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))
    fireEvent.change(screen.getByLabelText('自定义名称'), { target: { value: '网关' } })
    fireEvent.change(screen.getByLabelText('模型 API URL'), {
      target: { value: 'https://relay.example/v1' },
    })
    fireEvent.change(screen.getByLabelText('授权端点'), {
      target: { value: 'https://sso.example.com/authorize' },
    })
    fireEvent.change(screen.getByLabelText('令牌端点'), {
      target: { value: 'https://sso.example.com/token' },
    })
    fireEvent.change(screen.getByLabelText('Client ID'), { target: { value: 'cid-1' } })
    fireEvent.change(screen.getByLabelText('回调端口'), { target: { value: '99999' } })
    fireEvent.click(screen.getByRole('button', { name: '创建' }))

    expect(screen.getByText('回调端口需为 1-65535 的整数')).toBeInTheDocument()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('创建态没有实例可授权：展开也只有一行「创建后可授权」提示，不出现登录按钮', () => {
    renderForm({ mode: 'create' })

    fireEvent.click(screen.getByRole('button', { name: /订阅账号/ }))

    expect(screen.queryByRole('button', { name: '授权登录' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '退出登录' })).not.toBeInTheDocument()
    expect(
      screen.getByText('实例创建完成后，回到本页即可用「授权登录」绑定订阅账号。'),
    ).toBeInTheDocument()
  })

  it('编辑态未登录：显示「授权登录」；已配置时区块自动展开', () => {
    const onLogin = vi.fn()
    renderForm({
      mode: 'edit',
      initial: EDIT_INITIAL,
      hasKey: true,
      oauthLogin: {
        status: { configured: true, logged_in: false, expires_at: null, needs_login: false },
        busy: false,
        feedback: null,
        onLogin,
        onLogout: vi.fn(),
      },
    })

    // 已配置 → 自动展开（摘要同步显示「已配置 · 未登录」）
    expect(screen.getByRole('button', { name: /订阅账号/ })).toHaveAttribute(
      'aria-expanded',
      'true',
    )
    expect(screen.getByText('已配置 · 未登录')).toBeInTheDocument()

    const loginBtn = screen.getByRole('button', { name: '授权登录' })
    fireEvent.click(loginBtn)
    expect(onLogin).toHaveBeenCalledTimes(1)
    // 未登录时没有可清的令牌 → 不出现「退出登录」
    expect(screen.queryByRole('button', { name: '退出登录' })).not.toBeInTheDocument()
  })

  it('编辑态已登录：状态行显示有效期至本地时间 +「重新授权」「退出登录」', () => {
    const onLogout = vi.fn()
    renderForm({
      mode: 'edit',
      initial: EDIT_INITIAL,
      hasKey: true,
      oauthLogin: {
        status: {
          configured: true,
          logged_in: true,
          expires_at: 4102444800, // 2100-01-01T00:00:00Z
          needs_login: false,
        },
        busy: false,
        feedback: null,
        onLogin: vi.fn(),
        onLogout,
      },
    })

    expect(screen.getByText(/^已登录 · 有效期至 /)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重新授权' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '退出登录' }))
    expect(onLogout).toHaveBeenCalledTimes(1)
  })

  it('编辑态 needs_login：提示需重新授权，按钮仍是「授权登录」', () => {
    renderForm({
      mode: 'edit',
      initial: EDIT_INITIAL,
      hasKey: true,
      oauthLogin: {
        status: { configured: true, logged_in: false, expires_at: null, needs_login: true },
        busy: false,
        feedback: null,
        onLogin: vi.fn(),
        onLogout: vi.fn(),
      },
    })

    expect(screen.getByText('需重新授权')).toBeInTheDocument()
    expect(screen.getByText('登录已失效：需重新授权')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '授权登录' })).toBeInTheDocument()
  })

  it('授权请求进行中：按钮 loading 且不可重复点击；失败反馈原样展示后端文案', () => {
    const onLogin = vi.fn()
    const { unmount } = renderForm({
      mode: 'edit',
      initial: EDIT_INITIAL,
      hasKey: true,
      oauthLogin: {
        status: { configured: true, logged_in: false, expires_at: null, needs_login: false },
        busy: true,
        feedback: null,
        onLogin,
        onLogout: vi.fn(),
      },
    })

    const btn = screen.getByRole('button', { name: '发起中…' })
    expect(btn).toBeDisabled()
    fireEvent.click(btn)
    expect(onLogin).not.toHaveBeenCalled()
    unmount()

    renderForm({
      mode: 'edit',
      initial: EDIT_INITIAL,
      hasKey: true,
      oauthLogin: {
        status: { configured: true, logged_in: false, expires_at: null, needs_login: false },
        busy: false,
        feedback: { ok: false, msg: '授权被拒绝: access_denied' },
        onLogin: vi.fn(),
        onLogout: vi.fn(),
      },
    })
    expect(screen.getByText('授权被拒绝: access_denied')).toBeInTheDocument()
  })
})
