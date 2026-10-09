/**
 * ChatInputBar 的「增强模式状态行落位」契约（源码断言）。
 *
 * 为什么按源码断言而不是渲染整根输入栏：ChatInputBar 有 40+ 个必填 prop 与十余条
 * IPC 依赖，把它跑起来需要把整套 bridge / locale / gate 都替身一遍，而本组用例
 * 真正要钉的是**结构契约**——这两点在 jsdom 里都会渲染成同一个空 div，断言会退化成
 * 「渲染成功」这种恒真。源码断言能把这几条钉死：
 *
 * ① 大王定稿：增强状态行住在 `.input-bar-effort-menu`（推理强度弹窗）**内**、选项列表**下方**。
 * ② 边界取 (a) 方案：菜单入口放宽为 `effortAvailable || mode === 'workflow'`，
 *    否则模型不声明 reasoning_efforts 时 workflow 模式下增强状态行完全不可见。
 * ③ 「有没有等级可选」与「弹窗该不该出现」是两个条件，不得合并回一个。
 * ④ 已删除的扳手菜单入口不得复活（state / ref / effect / prop / i18n 词条 / CSS）。
 */

import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

const src = read('./ChatInputBar.tsx')
const panelSrc = read('./ChatPanel.tsx')
const zh = read('../../locales/zh.ts')
const en = read('../../locales/en.ts')
const chatInputCss = read('../../styles/chat-input.css')

/** 取 `<div className="input-bar-effort-menu">` 到其收尾之间的原文片段 */
function effortMenuBody(): string {
  const start = src.indexOf('<div className="input-bar-effort-menu">')
  if (start < 0) throw new Error('effort menu not found')
  const end = src.indexOf('</div>', src.indexOf('<EnhancedModeStatus />', start))
  if (end < 0) throw new Error('effort menu end not found')
  return src.slice(start, end)
}

describe('源码确实读进来了（防"读空文件导致断言静默通过"）', () => {
  it('四个文件非空且含预期锚点', () => {
    expect(src.length).toBeGreaterThan(5000)
    expect(src).toContain('className="input-bar-effort-menu"')
    expect(panelSrc.length).toBeGreaterThan(5000)
    expect(zh).toContain("'help.shortcut.workflowPanel'")
    expect(en).toContain("'help.shortcut.workflowPanel'")
    expect(chatInputCss.length).toBeGreaterThan(1000)
  })
})

describe('① 增强状态行落在推理强度弹窗内、选项列表下方', () => {
  it('<EnhancedModeStatus /> 位于 .input-bar-effort-menu 内部', () => {
    const body = effortMenuBody()
    expect(body).toContain('<EnhancedModeStatus />')
  })

  it('状态行排在推理强度选项（supportedEfforts.map）之后', () => {
    const body = effortMenuBody()
    expect(body.indexOf('supportedEfforts.map')).toBeGreaterThanOrEqual(0)
    expect(body.indexOf('<EnhancedModeStatus />')).toBeGreaterThan(
      body.indexOf('supportedEfforts.map'),
    )
  })

  it('状态行仅 workflow 模式渲染', () => {
    const body = effortMenuBody()
    expect(body).toContain("{mode === 'workflow' && <EnhancedModeStatus />}")
  })

  it('输入框 chip 行不再挂增强开关（EnhancedModeToggle 已移出输入栏）', () => {
    expect(src).not.toContain('<EnhancedModeToggle')
    expect(src).not.toContain('wfc-enhanced-toggle')
  })
})

describe('② 边界取 (a)：模型不支持 reasoning 时增强状态行仍可见', () => {
  it('effortAvailable 与 effortMenuAvailable 是两个独立条件', () => {
    expect(src).toContain('const effortAvailable = supportedEfforts.length > 0')
    expect(src).toContain("const effortMenuAvailable = effortAvailable || mode === 'workflow'")
  })

  it('菜单与 caret 的渲染条件都已改用 effortMenuAvailable（不是 effortAvailable）', () => {
    expect(src).toContain('{effortMenuAvailable && modelEffortOpen && (')
    expect(src).not.toContain('{effortAvailable && modelEffortOpen && (')
    expect(src).toContain('{effortMenuAvailable && (')
  })

  it('effortAvailable 为假时菜单里不渲染推理强度选项行（不伪造等级）', () => {
    const body = effortMenuBody()
    expect(body).toContain('{effortAvailable && (')
    // 选项块整体在 effortAvailable 闸门内，状态行在闸门之外
    expect(body.indexOf('{effortAvailable && (')).toBeLessThan(
      body.indexOf('<EnhancedModeStatus />'),
    )
  })
})

describe('③ 扳手菜单入口已彻底移除，不得复活', () => {
  it('ChatInputBar 里没有 wfMenu / 工具箱相关残留', () => {
    expect(src).not.toMatch(/wfMenu/)
    expect(src).not.toContain('input-bar-toolbox')
    expect(src).not.toContain('IconWrench')
    expect(src).not.toContain('formatPrimaryShortcut')
  })

  it('四个已删除的 prop 在 ChatInputBar / ChatPanel 两侧都无残留', () => {
    for (const prop of [
      'showDesktopToolbar',
      'onToggleDesktopToolbar',
      'onOpenWorkflowCanvas',
      'onOpenWorkflowList',
    ]) {
      expect(src).not.toContain(prop)
      expect(panelSrc).not.toContain(prop)
    }
  })

  it('i18n 词条 wfMenu.* 与 help.shortcut.desktopToolbar 已从 zh / en 同时删除', () => {
    for (const dict of [zh, en]) {
      expect(dict).not.toContain("'wfMenu.")
      expect(dict).not.toContain('help.shortcut.desktopToolbar')
    }
  })

  it('chat-input.css 里 .input-bar-toolbox-* 规则已清空', () => {
    expect(chatInputCss).not.toContain('input-bar-toolbox')
  })
})
