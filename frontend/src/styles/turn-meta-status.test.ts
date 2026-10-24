/**
 * 执行状态指示的样式契约（2026-10-24）：
 *
 * ① 执行中图标 = 四角圆点顺时针依次换位 → 向正方形中心聚合 → 停顿 → 分裂回四角，
 *    循环。**不能是旋转 svg**——旋转的语义是「加载等待」，长任务时会被读成卡住。
 * ② 动作行 chip 左侧不留 padding，左侧距离交给该 SVG 图标自身宽度。
 * ③ 耐心 chip（>60s 的 Running/Testing/Building/Checking）的流光必须真的可见：
 *    它曾被 `.turn-meta-bar--actions .turn-meta-chip`（0,2,0，后出现）覆盖掉
 *    background / color，渐变裁字完全失效（肉眼只剩静态灰字）。
 *
 * 这两条都是「代码看着对、界面上看不到」的类型，只能靠样式文本 + 选择器权重钉死。
 */

import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')
const stripComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, '')

const css = stripComments(read('./chat-messages.css'))
const tsx = read('../ui/TurnMetaBar.tsx')

/** 取选择器后第一个 `{...}` 的主体（单层，足够断言扁平规则） */
const ruleBody = (selector: string) => {
  const i = css.indexOf(selector)
  if (i < 0) return ''
  const start = css.indexOf('{', i)
  const end = css.indexOf('}', start)
  return start < 0 || end < 0 ? '' : css.slice(start + 1, end)
}

describe('执行中状态图标：四角依次换位 + 中心聚合而非旋转', () => {
  it('组件渲染四个角点，且不再引用旧 spinner / 三点类', () => {
    for (const suffix of ['--tl', '--tr', '--br', '--bl']) {
      expect(tsx).toContain(`turn-meta-thinking-dot${suffix}`)
    }
    expect(tsx).not.toContain('turn-meta-spinner')
    expect(tsx).not.toContain('turn-meta-thinking-dot--2')
    expect(tsx).not.toContain('turn-meta-thinking-dot--3')
  })

  it('四角关键帧只做位移/聚合，不含旋转', () => {
    for (const name of [
      'turn-meta-dot-tl',
      'turn-meta-dot-tr',
      'turn-meta-dot-br',
      'turn-meta-dot-bl',
    ]) {
      const kf = css.match(new RegExp(`@keyframes ${name}\\s*\\{[\\s\\S]*?\\n\\}`))?.[0] ?? ''
      expect(kf).not.toBe('')
      expect(kf).not.toMatch(/rotate/)
      expect(kf).toMatch(/translate\(/)
    }
    expect(css).not.toMatch(/@keyframes turn-meta-thinking-pulse\b/)
    expect(css).not.toMatch(/@keyframes turn-meta-spin\b/)
  })

  it('动作行 chip 左侧不留 padding，左侧距离交给 SVG 图标', () => {
    const body = ruleBody('.turn-meta-bar--actions .turn-meta-chip {')
    expect(body).toMatch(/padding:\s*1px 6px 1px 0/)
  })

  it('无动画降级（prefers-reduced-motion）仍然可读', () => {
    expect(css).toMatch(
      /@media \(prefers-reduced-motion: reduce\) \{\s*\.turn-meta-thinking-dot \{\s*animation:\s*none/,
    )
  })
})

describe('耐心 chip 的流光不被通用 chip 规则覆盖', () => {
  it('流光选择器追加了 patience 类，权重压过 .turn-meta-bar--actions .turn-meta-chip', () => {
    // 通用规则存在（正是当年覆盖源），且流光规则用复合选择器提高权重
    expect(css).toContain('.turn-meta-bar--actions .turn-meta-chip {')
    expect(css).toContain('.turn-meta-bar--actions .turn-meta-chip.turn-meta-chip--patience')
  })

  it('流光规则自带渐变裁字与 -webkit-text-fill-color', () => {
    const body = ruleBody('.turn-meta-bar--actions .turn-meta-chip.turn-meta-chip--patience')
    expect(body).toMatch(/background:\s*linear-gradient/)
    expect(body).toMatch(/background-clip:\s*text/)
    expect(body).toMatch(/-webkit-text-fill-color:\s*transparent/)
    expect(body).toMatch(/animation:\s*turn-meta-sheen/)
    // inline-flex 会让文本变匿名 flex item，裁字在部分 Chromium 下失效
    expect(body).toMatch(/display:\s*inline-block/)
  })
})
