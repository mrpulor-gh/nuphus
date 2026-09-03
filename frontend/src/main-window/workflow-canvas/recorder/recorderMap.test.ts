/**
 * recorderMap.test.ts — 录制捕获 → 真实 Action / step_draft 映射的纯逻辑测试
 * 关键铁律：每个画布节点必须是真实可执行 Action（tool / sleep），禁止 custom 占位。
 */

import { describe, expect, it } from 'vitest'
import {
  buildStep,
  buildSteps,
  elementExtractScript,
  hasNavigateDraft,
  isSelfCapture,
  NAVIGATE_PREREQ_MESSAGE,
  wheelAmount,
  wheelDirection,
} from './recorderMap'
import type { RecDraft, RecPending } from './recorderTypes'

function basePayload(overrides: Record<string, unknown> = {}) {
  return {
    intent: '点击搜索框',
    exceptionNote: '',
    variable: false,
    ...overrides,
  }
}

describe('isSelfCapture', () => {
  it('识别 Nuphus 自身窗口标题', () => {
    expect(isSelfCapture('Nuphus - 智慧协作伙伴')).toBe(true)
    expect(isSelfCapture('nuphus')).toBe(true)
    expect(isSelfCapture('Chrome')).toBe(false)
    expect(isSelfCapture('')).toBe(false)
    expect(isSelfCapture(null)).toBe(false)
    expect(isSelfCapture(undefined)).toBe(false)
  })
})

describe('wheel 映射', () => {
  it('正 delta 上滚 / 负 delta 下滚', () => {
    expect(wheelDirection(120)).toBe('up')
    expect(wheelDirection(-120)).toBe('down')
    expect(wheelDirection(0)).toBe('up')
    expect(wheelDirection(null)).toBe('up')
  })
  it('120 为一格，不足一格保底 1', () => {
    expect(wheelAmount(120)).toBe(1)
    expect(wheelAmount(-240)).toBe(2)
    expect(wheelAmount(60)).toBe(1)
    expect(wheelAmount(0)).toBe(1)
  })
})

describe('buildStep → 真实 Action', () => {
  it('click 事件 → desktop_mouse click', () => {
    const pending: RecPending = {
      action: 'click',
      stepNo: 1,
      event: {
        kind: 'click',
        button: 'left',
        x: 320,
        y: 480,
        wheel_delta: null,
        keys: [],
        window_title: '记事本',
        hwnd: 1234,
        pid: 4242,
        process_name: 'notepad.exe',
        ts_ms: 1,
      },
    }
    const built = buildStep(pending, basePayload())
    expect(built).not.toBeNull()
    expect(built!.kind).toBe('tool')
    expect(built!.do).toEqual({
      tool: 'desktop_mouse',
      with: { action: 'click', x: 320, y: 480, button: 'left' },
    })
    expect(built!.draft.action).toBe('click')
    expect(built!.draft.params).toMatchObject({
      x: 320,
      y: 480,
      button: 'left',
      variable: false,
    })
  })

  it('double_click 事件 → desktop_mouse double_click；right 按钮保留', () => {
    const pending: RecPending = {
      action: 'click',
      stepNo: 1,
      event: {
        kind: 'double_click',
        button: 'right',
        x: 1,
        y: 2,
        wheel_delta: null,
        keys: [],
        window_title: 'App',
        hwnd: 9,
        pid: 4243,
        process_name: null,
        ts_ms: 2,
      },
    }
    const built = buildStep(pending, basePayload())
    expect((built!.do as { with: Record<string, unknown> }).with).toMatchObject({
      action: 'double_click',
      x: 1,
      y: 2,
      button: 'right',
    })
  })

  it('scroll 事件 → desktop_mouse scroll（方向/格数）', () => {
    const pending: RecPending = {
      action: 'scroll',
      stepNo: 2,
      event: {
        kind: 'scroll',
        button: null,
        x: 100,
        y: 200,
        wheel_delta: -360,
        keys: [],
        window_title: '浏览器',
        hwnd: 8,
        pid: 4244,
        process_name: 'chrome.exe',
        ts_ms: 3,
      },
    }
    const built = buildStep(pending, basePayload())
    expect((built!.do as { with: Record<string, unknown> }).with).toMatchObject({
      action: 'scroll',
      direction: 'down',
      amount: 3,
    })
  })

  it('text 捕获定位点击 → desktop_input type（默认 {{input}} 脱敏、send none）', () => {
    const pending: RecPending = {
      action: 'text',
      stepNo: 3,
      event: {
        kind: 'click',
        button: 'left',
        x: 55,
        y: 66,
        wheel_delta: null,
        keys: [],
        window_title: '目标应用',
        hwnd: 77,
        pid: 4245,
        process_name: 'target.exe',
        ts_ms: 4,
      },
    }
    const built = buildStep(pending, {
      intent: '输入关键词',
      exceptionNote: '',
      variable: true,
    })
    expect(built!.do).toEqual({
      tool: 'desktop_input',
      with: { mode: 'type', hwnd: 77, text: '{{input}}', send: 'none' },
    })
    expect(built!.draft.params).toMatchObject({
      text: '{{input}}',
      variable: true,
      window_title: '目标应用',
    })
  })

  it('hotkey → desktop_input hotkey keys 原样传递', () => {
    const pending: RecPending = {
      action: 'hotkey',
      stepNo: 4,
      event: {
        kind: 'hotkey',
        button: null,
        x: 0,
        y: 0,
        wheel_delta: null,
        keys: ['ctrl', 'c'],
        window_title: '目标',
        hwnd: 55,
        pid: 4246,
        process_name: null,
        ts_ms: 5,
      },
    }
    const built = buildStep(pending, basePayload())
    expect((built!.do as { with: Record<string, unknown> }).with).toMatchObject({
      mode: 'hotkey',
      keys: ['ctrl', 'c'],
    })
  })

  it('sleep → sleep 步骤（真实可执行等待）', () => {
    const pending: RecPending = { action: 'sleep', stepNo: 5 }
    const built = buildStep(pending, basePayload({ seconds: 2.5 }))
    expect(built!.kind).toBe('sleep')
    expect(built!.do).toEqual({ sleep: 2.5 })
    expect(built!.draft.params.seconds).toBe(2.5)
  })

  it('region → desktop_find_image 区域锚点（evidence.rect + screenshot）', () => {
    const pending: RecPending = {
      action: 'region',
      stepNo: 6,
      overlay: {
        path: 'C:/plugin/workflows/w1/screenshots/rec_region_20260902.png',
        rect: { x: 10, y: 20, width: 300, height: 80 },
        base64: null,
      },
    }
    const built = buildStep(pending, basePayload())
    expect(built!.do).toEqual({
      tool: 'desktop_find_image',
      with: {
        template_path: 'C:/plugin/workflows/w1/screenshots/rec_region_20260902.png',
        threshold: 0.9,
        region: { x: 10, y: 20, width: 300, height: 80 },
      },
    })
    expect(built!.draft.evidence).toMatchObject({
      screenshot: 'C:/plugin/workflows/w1/screenshots/rec_region_20260902.png',
      rect: { x: 10, y: 20, width: 300, height: 80 },
    })
  })

  it('find_image → desktop_find_image 模板查找', () => {
    const pending: RecPending = {
      action: 'find_image',
      stepNo: 7,
      overlay: {
        path: 'C:/plugin/workflows/w1/screenshots/rec_template_20260902.png',
        rect: { x: 0, y: 0, width: 120, height: 40 },
        base64: 'data:image/png;base64,abc',
      },
    }
    const built = buildStep(pending, basePayload())
    expect((built!.do as { with: Record<string, unknown> }).with).toMatchObject({
      template_path: 'C:/plugin/workflows/w1/screenshots/rec_template_20260902.png',
      threshold: 0.9,
    })
    expect(built!.draft.action).toBe('find_image')
  })
})
describe('buildSteps 特化（chatagent / 找图后点击）', () => {
  function ev(overrides: Record<string, unknown> = {}) {
    return {
      kind: 'click',
      button: 'left',
      x: 55,
      y: 66,
      wheel_delta: null,
      keys: [],
      window_title: '目标应用',
      hwnd: 77,
      pid: 1,
      process_name: null,
      ts_ms: 1,
      ...overrides,
    }
  }

  it('text + target=chatagent → 单个 chat 步骤（基础窗口参数可空）', () => {
    const pending: RecPending = { action: 'text', stepNo: 3, event: ev() as RecPending['event'] }
    const all = buildSteps(pending, {
      intent: '判断当前页面是否已登录',
      exceptionNote: '',
      variable: false,
      target: 'chatagent',
      clickAfter: 'none',
    })
    expect(all).toHaveLength(1)
    const chat = all[0]
    expect(chat.kind).toBe('chat')
    expect(chat.name).toBe('交给 chatagent：判断当前页面是否已登录')
    expect(chat.do).toEqual({
      chat: '此处由 chatagent 处理：判断当前页面是否已登录',
      with: {},
    })
    expect(chat.draft.action).toBe('text_chat')
    expect(chat.draft.params).toMatchObject({ target: 'chatagent' })
    expect(chat.draft.canvas?.kind).toBe('chat')
  })

  it('text + target=window（默认）→ desktop_input 窗口输入（行为不回退）', () => {
    const pending: RecPending = { action: 'text', stepNo: 3, event: ev() as RecPending['event'] }
    const all = buildSteps(pending, {
      intent: '输入关键词',
      exceptionNote: '',
      variable: false,
      target: 'window',
      clickAfter: 'none',
      text: 'hello',
    })
    expect(all).toHaveLength(1)
    expect(all[0].kind).toBe('tool')
    expect((all[0].do as { with: Record<string, unknown> }).with).toMatchObject({
      mode: 'type',
      hwnd: 77,
      text: 'hello',
    })
    expect(all[0].draft.action).toBe('text')
  })

  it('find_image + clickAfter=click → find_image(capture) + desktop_mouse 双节点变量引用', () => {
    const pending: RecPending = {
      action: 'find_image',
      stepNo: 7,
      overlay: {
        path: 'C:/plugin/workflows/w1/screenshots/rec_template_20260902.png',
        rect: { x: 0, y: 0, width: 120, height: 40 },
        base64: 'data:image/png;base64,abc',
      },
    }
    const all = buildSteps(pending, {
      intent: '定位登录按钮',
      exceptionNote: '',
      variable: false,
      target: 'window',
      clickAfter: 'click',
    })
    expect(all).toHaveLength(2)
    // find_image 步骤：capture 输出变量，供后续点击引用
    expect(all[0].kind).toBe('tool')
    expect(all[0].capture).toBe('fi_x7')
    expect(all[0].name).toBe('定位登录按钮')
    expect((all[0].do as { with: Record<string, unknown> }).with).toMatchObject({
      template_path: 'C:/plugin/workflows/w1/screenshots/rec_template_20260902.png',
      threshold: 0.9,
    })
    expect(all[0].draft.action).toBe('find_image')
    expect(all[0].draft.params).toMatchObject({ click_after: 'click' })
    // 点击步骤：坐标引用 find_image capture 输出（真实变量模板，非假步骤）
    expect(all[1].kind).toBe('tool')
    expect(all[1].name).toContain('→ 找图后单击')
    expect(all[1].do).toEqual({
      tool: 'desktop_mouse',
      with: {
        action: 'click',
        x: '{{fi_x7 | json "x"}}',
        y: '{{fi_x7 | json "y"}}',
        button: 'left',
      },
    })
    expect(all[1].draft.action).toBe('click')
    expect(all[1].draft.params).toMatchObject({ source_capture: 'fi_x7', click_after: 'click' })
  })

  it('find_image + clickAfter=double_click → desktop_mouse double_click', () => {
    const pending: RecPending = {
      action: 'find_image',
      stepNo: 8,
      overlay: {
        path: 'C:/plugin/workflows/w1/screenshots/rec_template_20260902b.png',
        rect: { x: 5, y: 5, width: 60, height: 30 },
        base64: null,
      },
    }
    const all = buildSteps(pending, {
      intent: '双击打开设置',
      exceptionNote: '',
      variable: false,
      clickAfter: 'double_click',
    })
    expect(all).toHaveLength(2)
    expect((all[1].do as { with: Record<string, unknown> }).with).toMatchObject({
      action: 'double_click',
      x: '{{fi_x8 | json "x"}}',
      y: '{{fi_x8 | json "y"}}',
    })
    expect(all[0].capture).toBe('fi_x8')
  })

  it('region + clickAfter 被忽略（仅 find_image 支持找图后点击）', () => {
    const pending: RecPending = {
      action: 'region',
      stepNo: 9,
      overlay: {
        path: 'C:/plugin/workflows/w1/screenshots/rec_region_20260902c.png',
        rect: { x: 1, y: 2, width: 300, height: 80 },
        base64: null,
      },
    }
    const all = buildSteps(pending, {
      intent: '标记区域',
      exceptionNote: '',
      variable: false,
      clickAfter: 'click',
    })
    expect(all).toHaveLength(1)
    expect(all[0].draft.action).toBe('region')
  })
})
describe('buildStep → browser 动作组', () => {
  it('browser_navigate：url → browser_navigate with.url，单节点 tool', () => {
    const pending: RecPending = { action: 'browser_navigate', stepNo: 10 }
    const step = buildStep(pending, {
      intent: '打开官网首页',
      exceptionNote: '',
      variable: false,
      url: 'https://example.com',
    })
    expect(step).not.toBeNull()
    expect(step!.kind).toBe('tool')
    expect(step!.do).toEqual({ tool: 'browser_navigate', with: { url: 'https://example.com' } })
    expect(step!.draft.action).toBe('browser_navigate')
    expect(step!.draft.params).toMatchObject({ url: 'https://example.com' })
    expect(step!.draft.canvas?.kind).toBe('tool')
    expect(step!.draft.canvas?.tool).toBe('browser_navigate')
  })

  it('browser_navigate：缺 URL → 无法构造（面板校验兜底）', () => {
    const pending: RecPending = { action: 'browser_navigate', stepNo: 11 }
    const all = buildSteps(pending, {
      intent: '打开网页',
      exceptionNote: '',
      variable: false,
      url: '   ',
    })
    expect(all).toHaveLength(0)
  })

  it('browser_click：捕获 selector → browser_click with.selector，draft 带元素上下文', () => {
    const pending: RecPending = {
      action: 'browser_click',
      stepNo: 12,
      browserCapture: {
        selector: '#submit-btn',
        tag: 'button',
        text: '提交',
        href: null,
        url: 'https://example.com/form',
      },
    }
    const step = buildStep(pending, {
      intent: '点击提交按钮',
      exceptionNote: '',
      variable: false,
    })
    expect(step).not.toBeNull()
    expect(step!.kind).toBe('tool')
    expect(step!.do).toEqual({ tool: 'browser_click', with: { selector: '#submit-btn' } })
    expect(step!.draft.action).toBe('browser_click')
    expect(step!.draft.params).toMatchObject({
      selector: '#submit-btn',
      tag: 'button',
      text: '提交',
      url: 'https://example.com/form',
    })
    // 铁律：selector 必须是捕获的 CSS，禁止 @N 动态引用进入参数
    expect(step!.draft.params.selector).not.toMatch(/^@/)
  })

  it('browser_click：无捕获（无 browserCapture）→ 无法构造', () => {
    const pending: RecPending = { action: 'browser_click', stepNo: 13 }
    const all = buildSteps(pending, {
      intent: '点击某元素',
      exceptionNote: '',
      variable: false,
    })
    expect(all).toHaveLength(0)
  })

  it('browser_extract：无捕获 → browser_extract 空 with，意图说明记录 draft', () => {
    const pending: RecPending = { action: 'browser_extract', stepNo: 14 }
    const step = buildStep(pending, {
      intent: '获取列表结果',
      exceptionNote: '',
      variable: false,
      contentNote: '获取搜索结果第一条标题',
    })
    expect(step).not.toBeNull()
    expect(step!.kind).toBe('tool')
    expect(step!.do).toEqual({ tool: 'browser_extract', with: {} })
    expect(step!.draft.action).toBe('browser_extract')
    expect(step!.draft.params).toMatchObject({ content_note: '获取搜索结果第一条标题' })
  })

  it('browser_extract：contentNote 为空 → draft 记默认说明，仍如实生成空 with', () => {
    const pending: RecPending = { action: 'browser_extract', stepNo: 15 }
    const step = buildStep(pending, {
      intent: '获取页面',
      exceptionNote: '',
      variable: false,
      contentNote: '   ',
    })
    expect(step).not.toBeNull()
    expect(step!.do).toEqual({ tool: 'browser_extract', with: {} })
    expect(step!.draft.params.content_note).toBe('获取当前页面主要内容')
  })

  it('browser_extract：已选元素（browserCapture selector）→ browser_exec h.extract 元素级提取', () => {
    const pending: RecPending = {
      action: 'browser_extract',
      stepNo: 16,
      browserCapture: {
        selector: 'button[data-testid="submit"]',
        tag: 'button',
        text: '提交',
        href: null,
        url: 'https://example.com/form',
      },
    }
    const step = buildStep(pending, {
      intent: '提取提交按钮文案',
      exceptionNote: '',
      variable: false,
      contentNote: '',
    })
    expect(step).not.toBeNull()
    expect(step!.kind).toBe('tool')
    // 铁律：元素级提取不生成 browser_extract（schema 无 selector）→ browser_exec h.extract
    expect(step!.do).toEqual({
      tool: 'browser_exec',
      with: { script: 'h.extract("button[data-testid=\\"submit\\"]")' },
    })
    expect(step!.draft.action).toBe('browser_extract')
    expect(step!.draft.params).toMatchObject({
      content_note: '获取该元素内容',
      selector: 'button[data-testid="submit"]',
      url: 'https://example.com/form',
    })
    expect(step!.draft.canvas?.tool).toBe('browser_exec')
    expect(step!.draft.canvas?.with).toEqual({
      script: 'h.extract("button[data-testid=\\"submit\\"]")',
    })
  })

  it('elementExtractScript：含引号 selector 经 JSON.stringify 转义，不破坏 JS 字面量', () => {
    expect(elementExtractScript(`a[href="/x'"]`)).toBe(`h.extract("a[href=\\"/x'\\"]")`)
  })
})

describe('C3 前置检查 helper', () => {
  const navDraft = { action: 'browser_navigate', intent: '', params: {}, evidence: {} } as RecDraft
  const clickDraft = { action: 'click', intent: '', params: {}, evidence: {} } as RecDraft
  const extractDraft = {
    action: 'browser_extract',
    intent: '',
    params: {},
    evidence: {},
  } as RecDraft

  it('含 browser_navigate 草稿 → 通过；仅 click/extract → 不通过', () => {
    expect(hasNavigateDraft([navDraft])).toBe(true)
    expect(hasNavigateDraft([clickDraft, extractDraft])).toBe(false)
    expect(hasNavigateDraft([navDraft, extractDraft])).toBe(true)
    expect(hasNavigateDraft([])).toBe(false)
  })

  it('阻断文案精确含「请先添加『打开网址』步骤」', () => {
    expect(NAVIGATE_PREREQ_MESSAGE).toContain('请先添加『打开网址』步骤')
  })
})

describe('hotkey 修饰键全链路保留（D2）', () => {
  it('ctrl+d → do.with.keys 与 draft.params.keys 均含 ctrl、d，不丢修饰键', () => {
    const pending: RecPending = {
      action: 'hotkey',
      stepNo: 4,
      event: {
        kind: 'hotkey',
        button: null,
        x: 0,
        y: 0,
        wheel_delta: null,
        keys: ['ctrl', 'd'],
        window_title: '目标',
        hwnd: 55,
        pid: 4246,
        process_name: null,
        ts_ms: 5,
      },
    }
    const built = buildStep(pending, basePayload())
    expect(built).not.toBeNull()
    expect((built!.do as { with: Record<string, unknown> }).with).toMatchObject({
      mode: 'hotkey',
      keys: ['ctrl', 'd'],
    })
    expect(built!.draft.params.keys).toEqual(['ctrl', 'd'])
  })
})
