/**
 * SoulPage（灵魂：单模块 = 总开关 + 两条身份行）契约测试。
 *
 * 2026-09-28 大王定稿：头像与关系合成一个模块（不拆两个 Section 照搬）——
 * 本测试钉住合并后的形态与两条链路的落库语义：
 * ① 一个模块：显示开关 + 用户 / Nuphus 两行（点头像即上传 + 称呼输入）；
 * ② 头像：点击走 pickAndImportImage 入库写 LS_*（与合并前逐行一致），
 *    清除写 removeItem，空态退回字母头像（U / A），入库失败有 alert；
 * ③ 称呼：保存写 saveRelation（localStorage）+ setRelation（后端 relation.json）。
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SoulPage } from './SoulPage'
import { setRelation } from '../lib/api'
import { pickAndImportImage } from '../lib/localImage'
import { __setSkinImageProbe } from '../../ui/assetUrl'

const LS_SHOW_AVATAR = 'nuphus_show_avatar'
const LS_USER_AVATAR = 'nuphus_user_avatar'
const LS_NUPHUS_AVATAR = 'nuphus_nuphus_avatar'

// 后端 relation 同步（invoke set_relation）：不在本测试范围，桩掉
vi.mock('../lib/api', () => ({ setRelation: vi.fn(async () => undefined) }))
// 本地图片入库链路（Tauri dialog + invoke）：按用例桩返回值
vi.mock('../lib/localImage', () => ({ pickAndImportImage: vi.fn(async () => null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  // 与 @tauri-apps/api/core 的 convertFileSrc 同形：路径 → 可渲染 URL
  convertFileSrc: (p: string) => `asset://localhost/${p}`,
}))

beforeEach(() => {
  // asset:// 通道可用（发布版 WebView2 的真实环境）：头像走 asset 直读。
  __setSkinImageProbe(() => true)
})
afterEach(() => {
  __setSkinImageProbe(null)
})

function renderSoul() {
  return render(<SoulPage onClose={() => {}} />)
}

/** 「在消息中显示头像」所在行的开关（按钮自身无可访问名，按同行 label 就近取） */
const showAvatarSwitch = (view: ReturnType<typeof render>) => {
  const row = Array.from(view.container.querySelectorAll('.form-row')).find(r =>
    r.textContent?.includes('在消息中显示头像'),
  )
  const toggle = row?.querySelector('[role="switch"]')
  if (!toggle) throw new Error('未找到「在消息中显示头像」开关')
  return toggle as HTMLElement
}

/** 两条身份行的上传按钮（DOM 序：用户 → Nuphus） */
const uploadButtons = () => screen.getAllByRole('button', { name: '上传' })

describe('SoulPage 灵魂（单模块：开关 + 两侧身份行）', () => {
  beforeEach(() => {
    localStorage.clear()
    vi.mocked(pickAndImportImage).mockResolvedValue(null)
  })
  afterEach(() => {
    vi.clearAllMocks()
  })

  it('单模块：标题「灵魂」+ 显示开关 + 两条身份行（头像可点上传 + 称呼输入，无解释性文字）', () => {
    const view = renderSoul()

    // 一个模块（单一 Section 标题），不再有独立的「头像设置」分区标题
    expect(screen.getByRole('heading', { name: '灵魂' })).toBeInTheDocument()
    expect(screen.queryByText('头像设置')).toBeNull()

    // 三行：总开关 + 两条身份行
    expect(view.container.querySelectorAll('.form-row')).toHaveLength(3)
    expect(showAvatarSwitch(view)).toBeInTheDocument()

    // 两条身份行：可点头像（按钮形态，键盘可达）+ 称呼输入；侧别无可见文字
    // （大王定稿：不要「用户 / 称呼」这类解释，侧别由头像 U / A 与固定次序区分）
    expect(uploadButtons()).toHaveLength(2)

    const nameInputs = screen.getAllByLabelText('称呼')
    expect(nameInputs).toHaveLength(2)
    // 默认值来自 relation 默认配置（USER / Nuphus）
    expect(nameInputs[0]).toHaveValue('USER')
    expect(nameInputs[1]).toHaveValue('Nuphus')
  })

  it('点头像即上传：走 pickAndImportImage 入库、写 LS_USER_AVATAR、渲染 asset:// 并给出「清除」', async () => {
    vi.mocked(pickAndImportImage).mockResolvedValue('C:/img/me.png')
    const view = renderSoul()

    fireEvent.click(uploadButtons()[0])

    await waitFor(() => expect(localStorage.getItem(LS_USER_AVATAR)).toBe('C:/img/me.png'))
    expect(vi.mocked(pickAndImportImage)).toHaveBeenCalledTimes(1)
    const previews = view.container.querySelectorAll('.avatar-preview')
    // 异步解出（asset:// 优先）：等 resolveAvatarImageUrl 的 promise 落地后再断言
    await waitFor(() =>
      expect(previews[0].querySelector('img')).toHaveAttribute(
        'src',
        'asset://localhost/C:/img/me.png',
      ),
    )
    expect(screen.getByRole('button', { name: '清除' })).toBeInTheDocument()
  })

  // issue #94 的核心：asset 协议不可用时不能再出现「破图 + 零提示」。
  // 注入「asset:// 不可用」+ blob 兜底也读不到文件 → 组件必须落回字母头像。
  it('asset 通道不可用且读不到文件 → 落回字母头像，不出现破图', async () => {
    __setSkinImageProbe(() => false)
    vi.mocked(pickAndImportImage).mockResolvedValue('C:/img/broken.png')
    const view = renderSoul()

    fireEvent.click(uploadButtons()[0])
    await waitFor(() => expect(localStorage.getItem(LS_USER_AVATAR)).toBe('C:/img/broken.png'))

    const previews = () => view.container.querySelectorAll('.avatar-preview')
    await waitFor(() => {
      // 终点态：<img> 彻底消失（退回字母头像 U）。
      // 关键是「不能停在那个解不出来的 URL 上」——那正是破图 + 零提示。
      expect(previews()[0].querySelector('img')).toBeNull()
      expect(previews()[0].querySelector('svg text')?.textContent).toBe('U')
    })
  })

  it('空态：无自定义头像 → 字母头像兜底（用户侧 U / 智能体侧 A），不空白', () => {
    const view = renderSoul()

    const previews = view.container.querySelectorAll('.avatar-preview')
    expect(previews).toHaveLength(2)
    expect(previews[0].querySelector('svg text')?.textContent).toBe('U')
    expect(previews[1].querySelector('svg text')?.textContent).toBe('A')
  })

  it('开关切换写 LS_SHOW_AVATAR（合并前同一批键、同一语义）', () => {
    const view = renderSoul()
    const toggle = showAvatarSwitch(view)
    expect(toggle).toHaveAttribute('aria-checked', 'false')

    fireEvent.click(toggle)

    expect(toggle).toHaveAttribute('aria-checked', 'true')
    expect(localStorage.getItem(LS_SHOW_AVATAR)).toBe('true')
  })

  it('清除头像 = removeItem（不是写空串），并退回字母头像', () => {
    localStorage.setItem(LS_NUPHUS_AVATAR, 'C:/img/bot.png')
    const view = renderSoul()

    fireEvent.click(screen.getByRole('button', { name: '清除' }))

    expect(localStorage.getItem(LS_NUPHUS_AVATAR)).toBeNull()
    const previews = view.container.querySelectorAll('.avatar-preview')
    expect(previews[1].querySelector('img')).toBeNull()
    expect(previews[1].querySelector('svg text')?.textContent).toBe('A')
  })

  it('称呼保存：写 localStorage 两个 rel 键 + setRelation 同步后端，并给出「已保存」', () => {
    renderSoul()

    const nameInputs = screen.getAllByLabelText('称呼')
    fireEvent.change(nameInputs[0], { target: { value: '大王' } })
    fireEvent.change(nameInputs[1], { target: { value: '小助手' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    expect(localStorage.getItem('nuphus_rel_user_label')).toBe('大王')
    expect(localStorage.getItem('nuphus_rel_assistant')).toBe('小助手')
    expect(vi.mocked(setRelation)).toHaveBeenCalledWith({
      assistantName: '小助手',
      userLabel: '大王',
    })
    expect(screen.getByText('已保存')).toBeInTheDocument()
  })

  it('入库失败：alert 提示（不静默失败）', async () => {
    vi.mocked(pickAndImportImage).mockRejectedValueOnce(new Error('disk full'))
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {})
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    renderSoul()

    fireEvent.click(uploadButtons()[1])

    await waitFor(() =>
      expect(alertSpy).toHaveBeenCalledWith('头像入库失败：无法把所选图片复制到应用数据目录。'),
    )
    // 失败不落盘：键保持未设置
    expect(localStorage.getItem(LS_NUPHUS_AVATAR)).toBeNull()
  })
})
