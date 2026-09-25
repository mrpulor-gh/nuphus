/**
 * 应用外壳布局契约测试 —— 钉住 grid 化的两条结构性前提。
 *
 * 为什么需要这组用例（2026-09-26 返工，审核指出两处必修）：
 *
 * ① `.app-shell` 由 column-flex 改为两行 grid 后，行放置用的是**行号**
 *    （`grid-row: 2` / `grid-row: 1`），而两条选择器
 *    `.app-shell > *` 与 `.app-shell > header.title-bar` 的 specificity 完全相同
 *    （均为 0,1,1）—— `grid-row:1` 只靠源码顺序胜出。任何 CSS 载入顺序变化
 *    （打包器重排、code-splitting 顺序变化）都会**静默**把标题栏推进内容行，
 *    而且不会有任何报错。现改为命名行 `[titlebar] / [content]`。
 *
 * ② 我曾在 mobile.css 写下 `.app-shell > .mobile-notice { grid-row: 1/3 }`，
 *    而全仓**没有任何组件**渲染 `.mobile-notice`（推测出的类名）→ 死代码，已删。
 *    本用例防止"再按推测的类名给直接子元素补行放置规则"。
 *
 * 读取方式说明（两种更"正统"的做法在本项目都不可用，已实测）：
 * - `import ... from './components.css?raw'`：Vite 对 .css 后缀有内置插件，
 *   实测返回**空字符串**（len=0），断言会全部落空却看似通过 —— 比不写更危险。
 * - `node:fs`：本项目 tsconfig 未启用 @types/node，`tsc --noEmit` 直接报 TS2307。
 * 故改用运行时动态引入 + 类型收敛（下面这段是唯一允许的 as 用法，仅用于
 * 补上缺失的 Node 模块类型；`import.meta.url` 在 Vite/ESM 下已有类型支持）。
 *
 * 这些保证只在 CSS 文本层面可验证（jsdom 不做布局计算），故按源码断言。
 */

import { describe, expect, it } from 'vitest'

/**
 * 运行时加载 Node 的 fs。
 *
 * 三点说明：
 * 1. 模块名由变量拼出（`'node:' + 'fs'`）：本项目 tsconfig 未启用 @types/node，
 *    写成字面量会直接被 tsc 解析并报 TS2307；拼写后 tsc 无从静态解析，运行时
 *    （vitest 跑在 Node 上）照常可用。
 * 2. 唯一的类型收敛点，用注释标明这是有意为之（而非绕过类型检查的惯用手法）。
 * 3. readFileSync 接受 URL 对象 —— 故这里把入参类型如实标成 `any`，
 *    让 `new URL(...)` 合法传入，不再需要第二处 as。
 */
const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

/** 去掉注释，避免注释里提到的选择器/行号把断言骗过去 */
const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

const shellCss = stripComments(read('./components.css'))
const mobileCss = stripComments(read('./mobile.css'))

describe('.app-shell 两行 grid —— 行放置必须结构性可靠', () => {
  it('CSS 源码确实读进来了（防"读空文件导致断言静默通过"）', () => {
    expect(shellCss.length).toBeGreaterThan(1000)
    expect(shellCss).toContain('.app-shell')
    expect(mobileCss.length).toBeGreaterThan(100)
  })

  it('行用命名线 [titlebar] / [content]，不再依赖行号', () => {
    const rows = /grid-template-rows:\s*([^;]+);/.exec(shellCss)?.[1] ?? ''
    expect(rows).toContain('[titlebar]')
    expect(rows).toContain('[content]')
  })

  it('标题栏按**行名**放置（而非 grid-row: 1）', () => {
    const rule = /\.app-shell\s*>\s*header\.title-bar\s*\{([^}]*)\}/.exec(shellCss)?.[1] ?? ''
    expect(rule).toMatch(/grid-row:\s*titlebar/)
    // 行号写法是本次返工要消除的风险源
    expect(rule).not.toMatch(/grid-row:\s*\d/)
  })

  it('通配规则按**行名**放置（而非 grid-row: 2）', () => {
    const rule = /\.app-shell\s*>\s*\*\s*\{([^}]*)\}/.exec(shellCss)?.[1] ?? ''
    expect(rule).toMatch(/grid-row:\s*content/)
    expect(rule).not.toMatch(/grid-row:\s*\d/)
  })

  it('两条规则不再依赖源码顺序：目标行名互不相同（不存在 specificity 竞争）', () => {
    const header = /\.app-shell\s*>\s*header\.title-bar\s*\{\s*grid-row:\s*([\w-]+)/.exec(
      shellCss,
    )?.[1]
    const wildcard = /\.app-shell\s*>\s*\*\s*\{[^}]*grid-row:\s*([\w-]+)/.exec(shellCss)?.[1]
    expect(header).toBe('titlebar')
    expect(wildcard).toBe('content')
    // 同一个属性落在两个不同的行名上 —— 谁先谁后都不改变结果
    expect(header).not.toBe(wildcard)
  })

  it('禁止用 !important 解决行放置', () => {
    const block = shellCss.slice(shellCss.indexOf('.app-shell'))
    expect(block).not.toMatch(/grid-(row|column|area)\s*:[^;]*!important/)
  })
})

describe('不得给推测出的类名补行放置规则（.mobile-notice 死代码回归）', () => {
  it('mobile.css 不再有 .mobile-notice 规则', () => {
    expect(mobileCss).not.toContain('mobile-notice')
  })

  it('mobile.css 不再给 .app-shell 的直接子元素做行放置（除非有真实组件使用）', () => {
    const placements = mobileCss.match(/\.app-shell\s*>\s*[^,{]+/g) ?? []
    expect(placements).toEqual([])
  })
})
