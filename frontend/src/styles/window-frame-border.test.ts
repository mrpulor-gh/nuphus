/**
 * 窗口最外圈 1px 主题内描边 —— 四个全屏面的共享契约（2026-10-09 定案）。
 *
 * 契约要点（全部可回溯到源码，钉住防回归）：
 * ① 色一律走 --line-2（tokens.css 三主题各自取值，禁硬编码）；
 * ② 主窗口 .app-shell 直接 box-shadow —— 其子元素（title-bar / chat-area）背景透明，
 *    不会被盖断；
 * ③ 三个全屏宿主（preview .pv-page / 模型页 .models-page-host / 画布 .canvas-workbench-host）
 *    顶部是实色工具栏（--surface-1），照抄 box-shadow 会被子元素盖断（盒阴影画在
 *    自身背景之上、子元素之下）→ 经 ::after 覆盖层绘制：absolute + inset 0 +
 *    z-index 50 + pointer-events:none，三者缺一不可（缺 z-index 会被工具栏盖断，
 *    缺 pointer-events 会截获边缘点击）。
 *
 * 为什么钉源码：jsdom 不做布局与级联计算，视觉契约只能在 CSS 文本层面验证；
 * 项目既有先例：app-shell-layout.test.ts / chat-header-btn-label.test.ts /
 * opacity-system.test.ts（均为 CSS/TSX 源码级断言）。
 */
import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

/** 去掉注释，避免注释里提到的选择器/属性把断言骗过去 */
const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

/** 取一条规则（含伪元素选择器）的声明块；注释先剥离 */
const ruleBody = (css: string, selector: string): string => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const re = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, 's')
  return re.exec(stripComments(css))?.[1] ?? ''
}

const shellCss = read('./components.css')
const previewCss = read('../main-window/chat/preview-overlay.css')
const modelsCss = read('./models.css')

describe('窗口最外圈 1px 主题内描边', () => {
  it('主窗口 .app-shell：inset box-shadow + --line-2', () => {
    const body = ruleBody(shellCss, '.app-shell')
    expect(body).toContain('box-shadow: inset 0 0 0 1px var(--line-2)')
  })

  // 三个全屏宿主：顶部实色工具栏会盖断盒阴影，故统一走 ::after 覆盖层。
  // z 50：高于宿主内部全部常驻层（preview 侧最高 13），低于应用级浮层；
  // pointer-events:none 保证不截获任何交互。
  const hosts: Array<{ name: string; css: string; selector: string }> = [
    { name: '文件预览 .pv-page', css: previewCss, selector: '.pv-page::after' },
    { name: '模型页 .models-page-host', css: modelsCss, selector: '.models-page-host::after' },
    {
      name: '画布 .canvas-workbench-host',
      css: shellCss,
      selector: '.canvas-workbench-host::after',
    },
  ]

  for (const { name, css, selector } of hosts) {
    it(`${name}：::after 覆盖层画全圈描边，不挡交互`, () => {
      const body = ruleBody(css, selector)
      expect(body, `${name} 缺少描边覆盖层规则`).not.toBe('')
      expect(body).toContain('box-shadow: inset 0 0 0 1px var(--line-2)')
      expect(body, `${name} 覆盖层必须脱离布局流`).toContain('position: absolute')
      expect(body, `${name} 覆盖层必须铺满宿主`).toContain('inset: 0')
      expect(body, `${name} z-index 须高于宿主内部常驻层`).toContain('z-index: 50')
      expect(body, `${name} 不得截获交互`).toContain('pointer-events: none')
    })
  }
})
