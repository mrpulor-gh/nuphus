/**
 * HelpPage 帮助内容契约测试。
 *
 * 2026-09-28 整页按当前源码重写 —— 这里钉住重写的核心事实：
 * ① 斜杠命令 13 条与 ChatPanel SLASH_ITEMS 一致（含 /snake，无已移除的 /project）；
 * ② 快捷键覆盖 App.tsx useKeyboard 的真实绑定（Ctrl+K / L / N / Shift+W；Ctrl+U 桌面工具栏已彻底移除）；
 * ③ 新版块「界面入口 / 使用建议」在，且不再引用死命令 /project。
 */
import { render, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { HelpPage } from './HelpPage'
import { formatPrimaryShortcut } from '../../ui/platformShortcut'

const SECTION_TITLES = ['斜杠命令', '快捷键', '界面入口', '运行模式', '系统架构', '使用建议']

/** 与 HelpPage 同一规则派生按键文案（Mac = Cmd，其余 = Ctrl） */
const primary = (k: string) => formatPrimaryShortcut(k)
const shiftPrimary = formatPrimaryShortcut('Shift+W')

describe('HelpPage 帮助内容', () => {
  it('六个区块齐全（按源码重写后的结构）', () => {
    render(<HelpPage />)
    for (const title of SECTION_TITLES) {
      expect(screen.getByRole('heading', { name: title })).toBeInTheDocument()
    }
  })

  it('斜杠命令与 ChatPanel SLASH_ITEMS 逐条一致（含 /snake，无 /project）', () => {
    render(<HelpPage />)
    const slashSection = screen
      .getByRole('heading', { name: '斜杠命令' })
      .closest('section') as HTMLElement
    const cmds = Array.from(slashSection.querySelectorAll('.help-cmd')).map(el => el.textContent)
    expect(cmds).toEqual([
      '/new',
      '/models',
      '/themes',
      '/security',
      '/browser',
      '/memories',
      '/workflow',
      '/skills',
      '/knowledge',
      '/soul',
      '/reset',
      '/help',
      '/snake',
    ])
    expect(slashSection.textContent).not.toContain('/project')
  })

  it('快捷键覆盖真实绑定（K / L / N / Shift+W / Esc），无 TODO 占位', () => {
    render(<HelpPage />)
    const shortcutSection = screen
      .getByRole('heading', { name: '快捷键' })
      .closest('section') as HTMLElement
    for (const keys of ['Enter', primary('K'), primary('L'), primary('N'), shiftPrimary, 'Esc']) {
      expect(within(shortcutSection).getByText(keys)).toBeInTheDocument()
    }
    // 未实现的 TODO 占位（Ctrl/Cmd+O）不写进帮助
    expect(shortcutSection.textContent).not.toContain(`${primary('O')}`)
  })
})
