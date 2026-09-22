/**
 * 轻反馈胶囊**共享层**契约 —— 两处消费方（island / 模型页页内反馈）用同一份形态。
 *
 * 背景（大王 2026-09）：模型页切模型弹的是自己的绿色实底 toast（rgba(--success-rgb,.92)
 * + z 10000），与岛是两种观感。本层把胶囊形态抽成 .app-pill（styles/app-pill.css）+
 * AppPill（ui/AppPill.tsx），两处共用；**只统一视觉，不换通道**（模型页反馈留在自己的
 * 宿主里，改走全局岛会被 2500 宿主压住）。
 *
 * 这里断言两件事：
 *   ① 标记：AppPill 渲染共享类 + 语义图标 + 指示点（长短文案变体）
 *   ② CSS：岛胶囊的视觉声明**逐项等于改造前的冻结表**，且视觉只由共享类声明
 *      （jsdom 不跑样式级联，computed style 拿不到 → 用声明级契约代替，
 *        实机观感由 devtools 复核）
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { AppPill } from '../ui/AppPill'
import { IconAlertCircle, IconAlertTriangle, IconCheck, IconInfo } from '../ui/Icons'

const SOURCE_FILES = import.meta.glob('/src/**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

type FsLike = { readFileSync: (path: string, encoding: string) => string }

/**
 * CSS 源码读取（声明级契约用）。
 * vitest 默认不加载样式（`?raw` 对 .css 返回空串），所以这里直接经 node:fs 读原文；
 * 类型用本地最小声明避开为一个测试引入 @types/node。
 */
async function readStyle(relPath: string): Promise<string> {
  const spec = 'node:fs'
  const fs = (await import(/* @vite-ignore */ spec)) as unknown as FsLike
  const cwd = (globalThis as unknown as { process: { cwd(): string } }).process.cwd()
  return fs.readFileSync(`${cwd}/${relPath}`, 'utf8')
}

// ── 极简 CSS 解析（只认顶层规则；@media/@keyframes 先整体摘掉）──

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '')
}

/** 摘掉 @at-rule 块（含嵌套大括号）与 @import 行，只留顶层普通规则 */
function stripAtBlocks(css: string): string {
  let out = ''
  for (let i = 0; i < css.length; i++) {
    if (css[i] !== '@') {
      out += css[i]
      continue
    }
    const braceStart = css.indexOf('{', i)
    const semicolon = css.indexOf(';', i)
    if (braceStart === -1 || (semicolon !== -1 && semicolon < braceStart)) {
      i = semicolon === -1 ? css.length : semicolon
      continue
    }
    let depth = 0
    let j = braceStart
    for (; j < css.length; j++) {
      if (css[j] === '{') depth++
      else if (css[j] === '}' && --depth === 0) break
    }
    i = j
  }
  return out
}

/** 某选择器（精确匹配选择器列表中的一项）合并后的声明表 */
function declarations(css: string, selector: string): Record<string, string> {
  const merged: Record<string, string> = {}
  const body = stripAtBlocks(stripComments(css))
  const RULE = /([^{}]+)\{([^{}]*)\}/g
  let match: RegExpExecArray | null
  while ((match = RULE.exec(body))) {
    if (!match[1].split(',').some(sel => sel.trim() === selector)) continue
    for (const decl of match[2].split(';')) {
      const colon = decl.indexOf(':')
      if (colon < 0) continue
      merged[decl.slice(0, colon).trim()] = decl
        .slice(colon + 1)
        .trim()
        .replace(/\s+/g, ' ')
    }
  }
  return merged
}

/**
 * 冻结表：改造前 `.app-island-pill` 的视觉声明（git 上一版 styles/app-island.css）。
 * 抽共享类后必须**逐项不变**，只是来源从 `.app-island-pill` 挪到 `.app-pill`。
 * 两处刻意的等价改名：`--app-island-h` → `--app-pill-h`（同为 34px，见 :root）；
 * `appIslandIn` → `appPillIn`（同一组关键帧，搬到共享层供两处共用）。
 */
const FROZEN_PILL_VISUAL: Record<string, string> = {
  display: 'inline-flex',
  'align-items': 'center',
  gap: 'var(--space-2)',
  height: 'var(--app-pill-h)',
  'max-width': '100%',
  'min-width': '0',
  padding: '0 var(--space-4)',
  border: '1px solid var(--line-2)',
  'border-radius': '999px',
  background: 'var(--surface-1)',
  'box-shadow': 'var(--shadow-elevated)',
  color: 'var(--fg-1)',
  'font-family': 'inherit',
  'font-size': 'var(--fz-md)',
  'line-height': 'var(--leading-tight)',
  'text-align': 'left',
  animation: 'appPillIn var(--dur-med) var(--ease-out) both',
}

/** 视觉属性（这些只能由共享类声明，岛/页内反馈都不许各自改写） */
const VISUAL_PROPS = Object.keys(FROZEN_PILL_VISUAL)

const APP_PILL_CSS = 'src/styles/app-pill.css'
const APP_ISLAND_CSS = 'src/styles/app-island.css'
const MODELS_CSS = 'src/styles/models.css'
const SETTINGS_CENTER_CSS = 'src/styles/settings-center.css'

describe('AppPill：共享标记', () => {
  it('按相位给共享类 + 语义图标 + 指示点，不出现 inline 颜色', () => {
    const cases = [
      ['info', IconInfo],
      ['success', IconCheck],
      ['warning', IconAlertTriangle],
      ['error', IconAlertCircle],
    ] as const

    for (const [tone, Icon] of cases) {
      const probe = render(<Icon size={14} />)
      const expected = probe.container.querySelector('svg')?.innerHTML
      probe.unmount()

      const { container, unmount } = render(<AppPill tone={tone}>文案</AppPill>)
      const pill = container.firstElementChild as HTMLElement
      expect(pill.className).toContain('app-pill')
      expect(pill.className).toContain(`app-pill--${tone}`)
      expect(pill.getAttribute('style')).toBeNull()
      expect(container.querySelector('.app-pill-icon')?.innerHTML).toBe(expected)
      expect(container.querySelector('.app-pill-dot')).not.toBeNull()
      expect(container.querySelector('.app-pill-text')?.textContent).toBe('文案')
      unmount()
    }
  })

  it('interactive 渲染 button（岛的点击关闭 / hover 暂停），缺省是纯展示 div', () => {
    const onClick = vi.fn()
    const { container, unmount } = render(
      <AppPill tone="success" interactive onClick={onClick}>
        可点
      </AppPill>,
    )
    const button = screen.getByRole('button')
    expect(button.tagName).toBe('BUTTON')
    expect(button.className).toContain('app-pill--success')
    expect(button.className).not.toContain('app-pill--multiline')
    button.click()
    expect(onClick).toHaveBeenCalledTimes(1)
    unmount()

    const plain = render(<AppPill tone="error">展示态</AppPill>)
    expect(plain.container.firstElementChild?.tagName).toBe('DIV')
    expect(plain.container.firstElementChild?.className).not.toContain('app-pill--multiline')
    plain.unmount()
    expect(container).toBeTruthy()
  })

  it('multiline：长文案换行变体（页内反馈承载错误详情）', () => {
    const { container } = render(
      <AppPill tone="error" multiline>
        切换失败：后端返回了一段很长的错误详情
      </AppPill>,
    )
    expect(container.firstElementChild?.className).toContain('app-pill--multiline')
  })
})

describe('共享胶囊 CSS 契约', () => {
  it('岛胶囊的视觉声明逐项等于改造前的冻结表（来源：共享类 .app-pill）', async () => {
    const css = await readStyle(APP_PILL_CSS)
    const pill = declarations(css, '.app-pill')
    for (const [prop, value] of Object.entries(FROZEN_PILL_VISUAL)) {
      expect(pill[prop], `${prop} 与改造前不一致`).toBe(value)
    }
    // 高度单一来源：胶囊与岛回落定位共用 --app-pill-h
    expect(declarations(css, ':root')['--app-pill-h']).toBe('34px')
  })

  it('岛独有类只声明定位/交互，不重复任何视觉属性（视觉来源唯一）', async () => {
    const css = await readStyle(APP_ISLAND_CSS)
    const islandOnly = declarations(css, '.app-island-pill')
    for (const prop of VISUAL_PROPS) {
      expect(islandOnly[prop], `.app-island-pill 重复声明了视觉属性 ${prop}`).toBeUndefined()
    }
    expect(Object.keys(islandOnly).sort()).toEqual(['cursor', 'pointer-events', 'transition'])
  })

  it('岛层级仍是 260：全屏宿主可见性靠宿主锚点解决，禁止抬高 z-index', async () => {
    const islandCss = await readStyle(APP_ISLAND_CSS)
    const pillCss = await readStyle(APP_PILL_CSS)
    expect(declarations(islandCss, '.app-island')['z-index']).toBe('260')
    expect(stripComments(pillCss)).not.toMatch(/z-index/)
  })

  it('语义只靠图标 + 指示点：胶囊上不出现彩色底', async () => {
    const css = await readStyle(APP_PILL_CSS)
    expect(declarations(css, '.app-pill').background).toBe('var(--surface-1)')
    expect(css).not.toMatch(/--(success|error|warning|info)-rgb/)
    // 四相位只设 tone 变量，颜色落到图标与指示点
    for (const tone of ['info', 'success', 'warning', 'error']) {
      expect(declarations(css, `.app-pill--${tone}`), tone).toEqual({
        '--app-pill-tone': `var(--${tone})`,
      })
    }
    // 部件声明与改造前的 .app-island-icon / -dot / -text 逐项一致（仅 tone 变量改名）
    expect(declarations(css, '.app-pill-icon')).toEqual({
      flex: 'none',
      color: 'var(--app-pill-tone)',
    })
    expect(declarations(css, '.app-pill-dot')).toEqual({
      flex: 'none',
      width: '2px',
      height: '2px',
      'border-radius': '50%',
      background: 'var(--app-pill-tone)',
    })
    expect(declarations(css, '.app-pill-text')).toEqual({
      'min-width': '0',
      overflow: 'hidden',
      'text-overflow': 'ellipsis',
      'white-space': 'nowrap',
    })
  })

  it('模型页反馈只保留定位与层叠（顶部居中 / z 10000），彩色底已移除', async () => {
    const css = await readStyle(MODELS_CSS)
    const toast = declarations(css, '.feedback-toast')
    expect(toast.position).toBe('fixed')
    expect(toast.top).toBe('var(--space-4)')
    expect(toast['z-index']).toBe('10000')
    expect(toast['justify-content']).toBe('center')
    expect(toast['pointer-events']).toBe('none')
    // 位置语义用 flex 居中实现（共享层的进场动画会改写 transform，不能用 translateX）
    expect(toast.transform).toBeUndefined()
    expect(toast.background).toBeUndefined()
    // 旧的整块绿/红底规则必须彻底消失
    expect(declarations(css, '.feedback-toast--ok')).toEqual({})
    expect(declarations(css, '.feedback-toast--error')).toEqual({})
  })

  it('落点锚点几何共享：三处标题栏都提供定位基准与同一个 .island-slot', async () => {
    const pillCss = await readStyle(APP_PILL_CSS)
    const modelsCss = await readStyle(MODELS_CSS)
    const settingsCss = await readStyle(SETTINGS_CENTER_CSS)
    expect(declarations(pillCss, '.island-slot')).toEqual({
      position: 'absolute',
      inset: '0',
      display: 'flex',
      'align-items': 'center',
      'justify-content': 'center',
      padding: '0 40px',
      'pointer-events': 'none',
    })
    expect(declarations(modelsCss, '.models-page-bar').position).toBe('relative')
    expect(declarations(settingsCss, '.settings-center-bar').position).toBe('relative')
  })
})

describe('消费方接线（源码级）', () => {
  it('岛与模型页页内反馈都用共享组件 AppPill，不再各写一套胶囊类', () => {
    const island = SOURCE_FILES['/src/ui/AppIsland.tsx']
    const models = SOURCE_FILES['/src/main-window/pages/ModelsPage.tsx']

    expect(island).toContain('<AppPill')
    expect(models).toContain('<AppPill')
    expect(models).toContain("tone={pageFeedbackOk ? 'success' : 'error'}")
    // 模型页不再自带第二套观感
    expect(models).not.toContain('feedback-toast--ok')
    expect(models).not.toContain('feedback-toast--error')
  })
})
