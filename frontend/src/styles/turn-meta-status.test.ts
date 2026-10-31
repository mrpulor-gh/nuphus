/**
 * 执行状态指示的样式契约（2026-10-31 大王定 v4「去边框均衡器」）：
 *
 * ① 执行中图标 = 两根竖条高低起伏、相位错开半拍（右条 -0.55s），与虚拟形象
 *    working 态同源（avatar.css `nvEq`：1.1s / scaleY .55→1），去掉外壳只留
 *    双竖。**不能是旋转 svg**——旋转的语义是「加载等待」，长任务会被读成卡住；
 *    均衡器读的是「正在动手」。
 * ② 只允许 scaleY 驱动（transform-origin: bottom），turn-meta-* 关键帧禁见 rotate。
 * ③ 无动画降级时（prefers-reduced-motion）animation:none，并保留「两根不等高」
 *    的静态姿态——否则退化成两根等长直线，状态信息全丢。
 * ④ 动作行 chip 左侧不留 padding，左侧距离交给该 SVG 图标自身宽度。
 * ⑤ 耐心 chip（>60s 的 Running/Testing/Building/Checking）的流光必须真的可见：
 *    它曾被 `.turn-meta-bar--actions .turn-meta-chip`（0,2,0，后出现）覆盖掉
 *    background / color，渐变裁字完全失效（肉眼只剩静态灰字）。
 *
 * ①③④⑤ 都是「代码看着对、界面上看不到」的类型，只能靠样式文本 + 权重钉死。
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

describe('执行中状态图标：去边框均衡器（双竖高低起伏，非旋转）', () => {
  it('组件渲染双竖（容器 + 左/右条），旧回路全家与历史 spinner 绝迹', () => {
    for (const cls of [
      'turn-meta-thinking',
      'turn-meta-thinking-bar-l',
      'turn-meta-thinking-bar-r',
    ]) {
      expect(tsx).toContain(cls)
    }
    // 旧版「突触回路」（轨道/节点/脉冲/彗尾/涟漪）与更早的 spinner/点阵必须绝迹，防回潮
    for (const stale of [
      'turn-meta-thinking-axis',
      'turn-meta-thinking-node-l',
      'turn-meta-thinking-node-r',
      'turn-meta-thinking-pulse',
      'turn-meta-thinking-tail',
      'turn-meta-thinking-ring-l',
      'turn-meta-thinking-ring-r',
      'turn-meta-spinner',
      'turn-meta-thinking-dot',
      'turn-meta-thinking-core',
    ]) {
      expect(tsx).not.toContain(stale)
    }
  })

  it('关键帧只用 scaleY、禁见 rotate；右条相位错开半拍；旧关键帧绝迹', () => {
    const kf = css.match(/@keyframes turn-meta-eq\s*\{[\s\S]*?\n\}/)?.[0] ?? ''
    expect(kf).not.toBe('')
    expect(kf).toMatch(/scaleY\(/)
    expect(kf).not.toMatch(/rotate/)
    // 右条 -0.55s = 半个 1.1s 周期：两根条任意时刻高低不同
    // （不能用 ruleBody：合并选择器 `.turn-meta-thinking-bar-l,\n.turn-meta-thinking-bar-r {`
    //  会先命中 transform-box 那条，取不到动画声明）
    const rightRule =
      css.match(
        /\.turn-meta-thinking-bar-r\s*\{[^}]*animation:\s*turn-meta-eq\s*1\.1s[^}]*\}/,
      )?.[0] ?? ''
    expect(rightRule).not.toBe('')
    expect(rightRule).toMatch(/animation-delay:\s*-0\.55s/)
    // 旧回路关键帧不得残留（防止样式与组件各走一半）
    for (const name of [
      'turn-meta-pulse',
      'turn-meta-tail',
      'turn-meta-node-l',
      'turn-meta-node-r',
      'turn-meta-ring-l',
      'turn-meta-ring-r',
    ]) {
      expect(css).not.toMatch(new RegExp(`@keyframes ${name}\\b`))
    }
  })

  it('动作行 chip 左侧不留 padding，左侧距离交给 SVG 图标', () => {
    const body = ruleBody('.turn-meta-bar--actions .turn-meta-chip {')
    expect(body).toMatch(/padding:\s*1px 6px 1px 0/)
  })

  it('无动画降级（prefers-reduced-motion）保留「两根不等高」静态姿态', () => {
    // 文件里早有其它 prefers-reduced-motion 块（如流光降级），须锁定图标那块
    const block =
      [...css.matchAll(/@media \(prefers-reduced-motion: reduce\) \{[\s\S]*?\n\}/g)].find(m =>
        m[0].includes('.turn-meta-thinking-bar-l'),
      )?.[0] ?? ''
    expect(block).not.toBe('')
    expect(block).toContain('.turn-meta-thinking-bar-r')
    expect(block).toContain('animation: none')
    // 两根条静止时高度不同（避免退化成两条等长直线）
    expect(block).toMatch(/scaleY\(0\.62\)/)
    expect(block).toMatch(/scaleY\(0\.92\)/)
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
