/**
 * inspector-panel.test.ts — 样式铁律守卫（静态验证）。
 *
 * 把设计标准的实现约束固化为可执行断言：零色值字面量、焦点环形态、圆角阶梯、
 * 动效预算、类名前缀、容器查询阈值。任何后续提交违反即红，不靠人肉 review。
 *
 * 读取方式沿用既有结论（background-tasks.test.ts / opacity-system.test.ts）：
 * `?raw` 在本项目 Vite 配置下对 .css 返回空串，`node:fs` 又缺 @types/node ——
 * 故模块名由变量拼出 + 运行时动态引入 + 类型收敛。断言前剥掉注释，
 * 避免注释里提到的属性名/选择器把断言骗过去。
 */

import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')
const stripComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, '')

const css = stripComments(read('./inspector-panel.css'))
const sources = ['PanelFrame.tsx', 'SectionCard.tsx', 'FieldRow.tsx', 'controls.tsx', 'icons.tsx'].map(
  read,
)

describe('inspector-panel.css 铁律', () => {
  it('样式源确实读进来了（防「读空文件导致断言静默通过」）', () => {
    expect(css.length).toBeGreaterThan(2000)
    expect(css).toContain('.wfp-panel')
    expect(css).toContain('.wfp-sec')
    expect(css).toContain('.wfp-frow')
  })

  it('零色值字面量（hex / rgb( / hsl( / 命名色）', () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/)
    expect(css).not.toMatch(/rgba?\(\s*\d/)
    expect(css).not.toMatch(/hsla?\(\s*\d/)
    const colorDecls = css.match(
      /(?:color|background|border|fill|stroke|box-shadow|caret-color)\s*:[^;}]+/gi,
    )
    for (const decl of colorDecls ?? []) {
      expect(decl).not.toMatch(
        /\b(?:red|blue|green|white|black|gray|grey|orange|purple|yellow|pink|cyan|magenta|gold|silver)\b/i,
      )
    }
  })

  it('焦点环统一 box-shadow 0 0 0 2px var(--accent-glow)，不用 outline', () => {
    expect(css).toContain('box-shadow: 0 0 0 2px var(--accent-glow)')
    // 每条 outline 声明都必须恰好是 none（\s* 会回退空格骗过负向先行断言，故逐条归一化比对）
    for (const decl of css.match(/outline\s*:[^;}]+/g) ?? []) {
      expect(decl.replace(/\s+/g, '')).toBe('outline:none')
    }
  })

  it('圆角阶梯：面板 4 > 分区 3 > 控件 2 > 徽章 1，不越级到 5/6', () => {
    expect(css).toContain('var(--radius-4)')
    expect(css).toContain('var(--radius-3)')
    expect(css).toContain('var(--radius-2)')
    expect(css).toContain('var(--radius-1)')
    expect(css).not.toContain('var(--radius-5)')
    expect(css).not.toContain('var(--radius-6)')
  })

  it('动效仅 var(--dur-fast)：无 dur-med、无裸秒数时长', () => {
    expect(css).not.toContain('--dur-med')
    const transitions = css.match(/transition[^;}]+/g) ?? []
    expect(transitions.length).toBeGreaterThan(0)
    for (const t of transitions) {
      expect(t).toContain('--dur-fast')
      expect(t).not.toMatch(/\d+m?s\b/)
    }
  })

  it('类名前缀全部 wfp-（与组件文件对应；is-active/is-on 为状态修饰类）', () => {
    const classes = [...css.matchAll(/\.([a-zA-Z][\w-]*)/g)].map(m => m[1])
    expect(classes.length).toBeGreaterThan(0)
    for (const cls of classes) {
      expect(cls.startsWith('wfp-') || cls === 'is-active' || cls === 'is-on').toBe(true)
    }
  })

  it('两列/堆叠切换机械化：容器查询阈值 + 132px 定宽 + stack 覆盖', () => {
    expect(css).toContain('container-type: inline-size')
    expect(css).toContain('@container (min-width: 520px)')
    expect(css).toContain('grid-template-columns: 132px 1fr')
    expect(css).toContain('.wfp-frow--stack')
    // 堆叠修饰类必须赢过容器查询（特异性或书写顺序），不能靠 !important
    expect(css).not.toContain('!important')
  })

  it('高级分区为虚线边框（设计标准 .padv 形态）', () => {
    expect(css).toMatch(/\.wfp-adv\s*\{[^}]*border:\s*1px dashed var\(--line-2\)/)
    expect(css).toContain('.wfp-adv__summary')
    expect(css).toContain('.wfp-adv[open] .wfp-adv__chev')
  })

  it('焦点环两种变体都在（外环 + inset 环，均 accent-glow）', () => {
    expect(css).toContain('box-shadow: 0 0 0 2px var(--accent-glow)')
    expect(css).toContain('box-shadow: inset 0 0 0 2px var(--accent-glow)')
  })

  it('无双主题覆盖块、无 !important、无新字体族', () => {
    expect(css).not.toContain('[data-theme')
    expect(css).not.toContain('!important')
    const fontDecls = css.match(/font-family\s*:[^;}]+/g) ?? []
    for (const decl of fontDecls) {
      expect(decl).toMatch(/var\(--font-(?:mono|ui)\)|inherit/)
    }
  })
})

describe('组件族源码纪律', () => {
  it('五个源码文件都读到了', () => {
    for (const src of sources) {
      expect(src.length).toBeGreaterThan(200)
    }
  })

  it('零 emoji（图标一律 inline SVG）', () => {
    for (const src of [...sources, read('./inspector-panel.css')]) {
      expect(src).not.toMatch(/[\u{1F000}-\u{1FAFF}\u{2600}-\u{27BF}\u{2B00}-\u{2BFF}\u{FE0F}]/u)
    }
  })

  it('图标全部 stroke=currentColor（无 fill 硬编码）', () => {
    const icons = sources[sources.length - 1]
    expect(icons).toContain('stroke="currentColor"')
    expect(icons).not.toMatch(/fill="(?!none")/)
  })
})
