/**
 * header 右侧三按钮的 hover 名称浮层（仿桌面工具条 .desktop-toolbar-label）。
 *
 * 为什么钉源码：jsdom 不做布局与级联计算，浮层的显隐/定位无法在测试里
 * 真实触发。项目既有先例：opacity-system.test.ts 与
 * ChatInputBar.enhancedStatus.test.ts 都对 CSS/TSX 做源码级断言。
 *
 * 钉四件事：
 * ① 三个按钮都挂了 .chat-header-btn-label，且浮层文案走 i18n（不是硬编码中文）；
 * ② 原生 title 已移除 —— 否则浏览器灰底方框与自定义浮层会同时出现（双提示）；
 * ③ 三个按钮本体都有 position:relative —— 否则浮层的包含块是
 *    .chat-header-right（position:relative），三个浮层会重叠在同一位置；
 * ④ 浮层配方与工具条 .desktop-toolbar-label 同源，且未合并进三条本体规则
 *    （合并会让 opacity-system.test.ts:236 的单选择器断言静默落空）。
 */
import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')
const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

const panelTsx = read('../main-window/chat/ChatPanel.tsx')
const messagesCss = stripComments(read('./chat-messages.css'))
const toolbarCss = stripComments(read('./desktop-toolbar.css'))

/** 取一条顶层规则的声明块（首个匹配），与 opacity-system.test.ts 同法 */
const ruleBody = (css: string, selector: string): string => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const re = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, 's')
  return re.exec(css)?.[1] ?? ''
}

const HEADER_BTNS = [
  {
    cls: '.chat-header-settings-btn',
    bare: 'chat-header-settings-btn',
    label: "t('app.settings')",
  },
  {
    cls: '.chat-header-appearance-btn',
    bare: 'chat-header-appearance-btn',
    label: "t('app.appearance')",
  },
  {
    cls: '.chat-header-browser-btn',
    bare: 'chat-header-browser-btn',
    label: "t('browser.toggleTitle')",
  },
]

describe('header 右列按钮的 hover 名称浮层', () => {
  it('三个按钮各挂一个 label span，文案走 i18n', () => {
    const spans =
      panelTsx.match(/<span className="chat-header-btn-label">\{([^}]+)\}<\/span>/g) ?? []
    expect(spans).toHaveLength(3)
    for (const s of spans) expect(s).toMatch(/\{t\('[^']+'\)\}/)
    // 设置 / 外观用短名，浏览器用带说明的 toggleTitle
    expect(panelTsx).toContain(`<span className="chat-header-btn-label">{t('app.settings')}</span>`)
    expect(panelTsx).toContain(
      `<span className="chat-header-btn-label">{t('app.appearance')}</span>`,
    )
    expect(panelTsx).toContain(
      `<span className="chat-header-btn-label">{t('browser.toggleTitle')}</span>`,
    )
  })

  it('原生 title 已从三个按钮移除（避免与自定义浮层双提示）', () => {
    for (const { bare, label } of HEADER_BTNS) {
      // 浏览器按钮的 className 是模板串（chat-header-browser-btn${browserOpening ? …}），
      // 故按裸类名定位，不匹配 `className="…"` 整串。
      const idx = panelTsx.indexOf(bare)
      expect(idx).toBeGreaterThan(-1)
      const block = panelTsx.slice(idx, panelTsx.indexOf('</button>', idx))
      expect(block).not.toContain('title=')
      expect(block).toContain('aria-label=')
      expect(block).toContain(label)
    }
  })

  it('三个按钮本体都有 position:relative（浮层各自贴自己的按钮）', () => {
    const body = ruleBody(messagesCss, HEADER_BTNS.map(b => b.cls).join(',\n'))
    expect(body).toMatch(/position:\s*relative/)
    // 逐条单选择器再确认一次（防合并规则把某按钮漏掉）
    for (const { cls } of HEADER_BTNS) {
      expect(ruleBody(messagesCss, cls)).not.toBe('')
    }
  })

  it('浮层配方与工具条 .desktop-toolbar-label 同源', () => {
    const tip = ruleBody(messagesCss, '.chat-header-btn-label')
    const src = ruleBody(toolbarCss, '.desktop-toolbar-label')
    for (const prop of [
      'position: absolute',
      'right: calc(100% + 8px)',
      'transform: translateY(-50%)',
      'border-radius: 6px',
      'backdrop-filter: blur(8px) saturate(1.2)',
      'color: var(--fg-1)',
      'font-size: var(--fz-xs)',
      'opacity: 0',
      'visibility: hidden',
      'pointer-events: none',
    ]) {
      expect(tip).toContain(prop)
      // 工具条侧必须同样具备（配方同源的另一半）
      expect(src).toContain(prop)
    }
    // 磨砂底色档位与工具条一致（85% 而非 header 按钮本体的 62%）
    expect(tip).toContain('color-mix(in srgb, var(--panel-bg) 85%, transparent)')
    expect(tip).toContain('--border')
  })

  it('hover / focus-visible 均能唤出浮层，三个按钮都覆盖', () => {
    const block = ruleBody(
      messagesCss,
      [
        '.chat-header-settings-btn:hover > .chat-header-btn-label',
        '.chat-header-appearance-btn:hover > .chat-header-btn-label',
        '.chat-header-browser-btn:hover > .chat-header-btn-label',
        '.chat-header-settings-btn:focus-visible > .chat-header-btn-label',
        '.chat-header-appearance-btn:focus-visible > .chat-header-btn-label',
        '.chat-header-browser-btn:focus-visible > .chat-header-btn-label',
      ].join(',\n'),
    )
    expect(block).toContain('opacity: 1')
    expect(block).toContain('visibility: visible')
  })

  it('三条本体规则未被合并（opacity-system.test.ts 的单选择器断言仍可取块）', () => {
    for (const { cls } of HEADER_BTNS) {
      const body = ruleBody(messagesCss, cls)
      // 本体块里必须仍是完整的 header 配方（30x30 + 62% 磨砂）
      expect(body).toContain('width: 30px')
      expect(body).toContain('color-mix(in srgb, var(--panel-bg) 62%, transparent)')
      expect(body).toContain('border-radius: 8px')
    }
  })
})
