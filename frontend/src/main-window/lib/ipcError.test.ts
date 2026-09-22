import { describe, it, expect } from 'vitest'
import { friendlyIpcError } from './ipcError'

/**
 * 用户可见错误文案的守门测试。
 *
 * 背景：这些文案过去直接透传 Tauri/reqwest 的英文原文（「IPC invoke xxx failed:
 * Command xxx not found」「error sending request for url …」），大王明确反馈
 * 「又是这种用户看不懂的提示信息」。本测试锁死：技术错误必须转成可操作的中文，
 * 而后端返回的中文业务错误必须原样保留（不能被吞成兜底文案）。
 */
describe('friendlyIpcError', () => {
  const wrap = (reason: string) => new Error(`IPC invoke some_cmd failed: ${reason}`)

  it('命令未注册 → 指向「重启/重新构建」，不泄露命令名', () => {
    const msg = friendlyIpcError(wrap('Command create_custom_provider not found'))
    expect(msg).toContain('后端版本过旧')
    expect(msg).toContain('重新构建')
    expect(msg).not.toContain('create_custom_provider')
    expect(msg).not.toMatch(/IPC invoke/i)
  })

  it('后端业务错误里的 "not found" 不得被误判成命令未注册', () => {
    // 历史事故：`model 'x' not found for provider 'y'` 被吞成「后端版本过旧」，
    // 排查方向被带偏。归因必须限定在命令上下文。
    const biz = "model 'deepseek-v4.1-flash' not found for provider 'custom-41flash'"
    const msg = friendlyIpcError(wrap(biz))
    expect(msg).not.toContain('后端版本过旧')
    expect(msg).toBe(biz)
  })

  it('网络不可达 → 提示检查地址与网络', () => {
    const msg = friendlyIpcError(
      wrap('error sending request for url (https://api.example.com/v1/models)'),
    )
    expect(msg).toContain('无法连接该接口地址')
    expect(msg).not.toContain('api.example.com')
  })

  it('超时同样归类为连接失败', () => {
    expect(friendlyIpcError(wrap('operation timed out'))).toContain('无法连接该接口地址')
  })

  it('401 / 密钥错误 → 指向检查 API Key', () => {
    expect(friendlyIpcError(wrap('HTTP 401 Unauthorized'))).toContain('API Key 无效')
    expect(friendlyIpcError(wrap('invalid api key'))).toContain('API Key 无效')
  })

  it('403 → 权限问题', () => {
    expect(friendlyIpcError(wrap('403 Forbidden'))).toContain('无权访问')
  })

  it('404 → 提示 URL 可能漏写/多写 /v1', () => {
    expect(friendlyIpcError(wrap('404 Not Found'))).toContain('/v1')
  })

  it('返回非 JSON（中转站登录页等）→ 提示不是模型列表接口', () => {
    expect(friendlyIpcError(wrap('expected value at line 1 column 1'))).toContain('不是模型列表')
  })

  it('后端中文业务错误原样保留（不得被兜底吞掉）', () => {
    const biz = '该自定义模型尚未创建，请先保存基本信息'
    expect(friendlyIpcError(wrap(biz))).toBe(biz)
  })

  it('原因不可读时使用调用方兜底文案', () => {
    expect(friendlyIpcError(new Error('IPC invoke x failed: '), '创建失败')).toBe('创建失败')
    expect(friendlyIpcError(new Error(''), '创建失败')).toBe('创建失败')
  })
})