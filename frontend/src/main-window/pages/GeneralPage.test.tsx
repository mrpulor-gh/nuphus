/**
 * GeneralPage（设置中心分区 `general`）契约测试。
 *
 * 2026-10-27 由 LanguagePage 契约测试改造：语言链路的覆盖**原样保留**（切换即写
 * localStorage + 通知后端；后端优先、localStorage 兜底），并补齐通用设置的落库契约：
 * ① 挂载读 get_general_settings（四项）并渲染到控件（含非默认值）；
 * ② 会话字号：改动即调 set_chat_font_size + 注入 CSS 变量 --chat-font-size；
 * ③ 关闭行为 / HUD / 完成置前：改动即调对应 set_* 命令，失败回滚；
 * ④ 快捷键弹窗：同源清单渲染 + 搜索过滤 + 可编辑行（v1.5 录制 / 冲突 / 恢复的细节覆盖在 ShortcutsDialog.test.tsx）。
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { GeneralPage } from './GeneralPage'
import { LangProvider } from '../../locales'
import type { GeneralSettings } from '../lib/api'

let backendLang: Promise<string> = Promise.resolve('zh-CN')
let generalSettings: GeneralSettings = {
  close_action: 'quit',
  chat_font_size: 14,
  hud_enabled: true,
  raise_on_done: true,
}
const setLanguageMock = vi.fn(async (_lang: string) => undefined)
const setCloseActionMock = vi.fn(async (action: string) => action)
const setChatFontSizeMock = vi.fn(async (size: number) => size)
const setHudEnabledMock = vi.fn(async (enabled: boolean) => enabled)
const setRaiseOnDoneMock = vi.fn(async (enabled: boolean) => enabled)

vi.mock('../lib/api', () => ({
  getLanguage: () => backendLang,
  setLanguage: (lang: string) => setLanguageMock(lang),
  getGeneralSettings: () => Promise.resolve(generalSettings),
  setCloseAction: (action: string) => setCloseActionMock(action),
  setChatFontSize: (size: number) => setChatFontSizeMock(size),
  setHudEnabled: (enabled: boolean) => setHudEnabledMock(enabled),
  setRaiseOnDone: (enabled: boolean) => setRaiseOnDoneMock(enabled),
  getShortcutOverrides: () => Promise.resolve({}),
  setShortcutOverrides: (next: Record<string, string>) => Promise.resolve(next),
}))

function renderPage() {
  render(
    <LangProvider>
      <GeneralPage />
    </LangProvider>,
  )
}

const langSelect = () => screen.getByRole('combobox', { name: /语言|Language/ })
const selectByName = (name: string) => screen.getByRole('combobox', { name })
const switchByName = (name: string) => screen.getByRole('switch', { name })

describe('GeneralPage 通用设置', () => {
  beforeEach(() => {
    localStorage.clear()
    document.documentElement.style.removeProperty('--chat-font-size')
    backendLang = Promise.resolve('zh-CN')
    generalSettings = {
      close_action: 'quit',
      chat_font_size: 14,
      hud_enabled: true,
      raise_on_done: true,
    }
    setLanguageMock.mockClear()
    setCloseActionMock.mockClear()
    setChatFontSizeMock.mockClear()
    setHudEnabledMock.mockClear()
    setRaiseOnDoneMock.mockClear()
  })
  afterEach(() => {
    vi.clearAllMocks()
  })

  /* ── 语言链路（迁移自原 LanguagePage 测试，行为一行未改）── */

  it('语言行 + zh/en 两项，默认选中跟随后端设置', async () => {
    renderPage()

    // Section 行 label 与语言导航同源（app.language）
    expect(screen.getAllByText('语言').length).toBeGreaterThan(0)
    await waitFor(() => expect(langSelect()).toHaveValue('zh'))
    const options = Array.from((langSelect() as HTMLSelectElement).options).map(o => o.textContent)
    expect(options).toEqual(['中文', 'English'])
  })

  it('切到 English：写 localStorage + 通知后端 en-US，选中值随动', async () => {
    renderPage()
    await waitFor(() => expect(langSelect()).toHaveValue('zh'))

    fireEvent.change(langSelect(), { target: { value: 'en' } })

    expect(langSelect()).toHaveValue('en')
    expect(localStorage.getItem('nuphus_language')).toBe('en')
    expect(setLanguageMock).toHaveBeenCalledWith('en-US')
  })

  it('后端读取失败：回落 localStorage 里已存的语言', async () => {
    localStorage.setItem('nuphus_language', 'en')
    backendLang = Promise.reject(new Error('backend down'))
    renderPage()

    await waitFor(() => expect(langSelect()).toHaveValue('en'))
  })

  /* ── 通用偏好读取 ── */

  it('挂载读取 get_general_settings：四项偏好渲染到对应控件', async () => {
    generalSettings = {
      close_action: 'tray',
      chat_font_size: 16,
      hud_enabled: false,
      raise_on_done: false,
    }
    renderPage()

    await waitFor(() => expect(selectByName('会话字号')).toHaveValue('16'))
    expect(selectByName('关闭按钮行为')).toHaveValue('tray')
    expect(switchByName('HUD 悬浮条')).toHaveAttribute('aria-checked', 'false')
    expect(switchByName('完成时窗口置前')).toHaveAttribute('aria-checked', 'false')
  })

  /* ── 写入链路（落库 + 行为）── */

  it('会话字号：改动即落库并注入 --chat-font-size（仅消息正文消费）', async () => {
    renderPage()
    await waitFor(() => expect(selectByName('会话字号')).toHaveValue('14'))

    fireEvent.change(selectByName('会话字号'), { target: { value: '15' } })

    expect(setChatFontSizeMock).toHaveBeenCalledWith(15)
    expect(document.documentElement.style.getPropertyValue('--chat-font-size')).toBe('15px')
  })

  it('关闭按钮行为：改动即落库，按后端归一值校准', async () => {
    renderPage()
    await waitFor(() => expect(selectByName('关闭按钮行为')).toHaveValue('quit'))

    fireEvent.change(selectByName('关闭按钮行为'), { target: { value: 'tray' } })

    expect(setCloseActionMock).toHaveBeenCalledWith('tray')
    await waitFor(() => expect(selectByName('关闭按钮行为')).toHaveValue('tray'))
  })

  it('HUD / 完成置前开关：点击即调对应 set_* 命令', async () => {
    renderPage()
    await waitFor(() => expect(switchByName('HUD 悬浮条')).toHaveAttribute('aria-checked', 'true'))

    fireEvent.click(switchByName('HUD 悬浮条'))
    expect(setHudEnabledMock).toHaveBeenCalledWith(false)
    await waitFor(() => expect(switchByName('HUD 悬浮条')).toHaveAttribute('aria-checked', 'false'))

    fireEvent.click(switchByName('完成时窗口置前'))
    expect(setRaiseOnDoneMock).toHaveBeenCalledWith(false)
    await waitFor(() =>
      expect(switchByName('完成时窗口置前')).toHaveAttribute('aria-checked', 'false'),
    )
  })

  it('落库失败回滚：开关不得停留在未落盘状态', async () => {
    setHudEnabledMock.mockRejectedValueOnce(new Error('backend down'))
    renderPage()
    await waitFor(() => expect(switchByName('HUD 悬浮条')).toHaveAttribute('aria-checked', 'true'))

    fireEvent.click(switchByName('HUD 悬浮条'))

    await waitFor(() => expect(switchByName('HUD 悬浮条')).toHaveAttribute('aria-checked', 'true'))
  })

  /* ── 入口行 ── */

  it('快捷键弹窗：同源清单渲染（分段：不可编辑在上、可编辑在下）+「恢复全部默认」默认禁用', async () => {
    renderPage()
    await waitFor(() => expect(switchByName('HUD 悬浮条')).toHaveAttribute('aria-checked', 'true'))

    fireEvent.click(screen.getByRole('button', { name: /快捷键/ }))

    const dialog = await screen.findByRole('dialog', { name: '快捷键' })
    // 清单为 HelpPage 同源数据（buildShortcutItems 共 11 条，Ctrl 平台文案）
    expect(within(dialog).getAllByText(/Ctrl\+K/).length).toBe(1)
    expect(dialog.querySelectorAll('.shortcuts-row').length).toBe(11)
    // 可编辑集 = 命令键 5 + 全局热键 3 = 8；分段标题顺序：不可编辑 → 可编辑
    expect(dialog.querySelectorAll('.shortcuts-row.is-editable').length).toBe(8)
    const groupTitles = Array.from(dialog.querySelectorAll('.shortcuts-group-title')).map(
      el => el.textContent,
    )
    expect(groupTitles).toEqual(['不可编辑', '可编辑'])
    expect(within(dialog).getByRole('button', { name: '恢复全部默认' })).toBeDisabled()
    expect(
      within(dialog).getByText('点击「可编辑」中的行，按下新的组合键即可修改（按 Esc 取消）'),
    ).toBeInTheDocument()
  })

  it('「窗口」组行序：HUD → 完成置前 → 关闭行为（组内末项）', async () => {
    renderPage()
    await waitFor(() => expect(selectByName('会话字号')).toHaveValue('14'))

    const rows = Array.from(document.querySelectorAll('.general-page .form-row')).map(
      row => row.textContent ?? '',
    )
    const iHud = rows.findIndex(text => text.includes('HUD 悬浮条'))
    const iRaise = rows.findIndex(text => text.includes('完成时窗口置前'))
    const iClose = rows.findIndex(text => text.includes('关闭按钮行为'))
    expect(iHud).toBeGreaterThanOrEqual(0)
    expect(iRaise).toBeGreaterThan(iHud)
    expect(iClose).toBeGreaterThan(iRaise)
  })
})
