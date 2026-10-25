/**
 * 执行状态指示的样式契约（2026-10-25 大王定 v3「突触回路」）：
 *
 * ① 执行中图标 = 两枚神经元 + 微光轨道 + 往返脉冲（彗尾 0.05s 相位滞后），
 *    到达即端点闪爆 + 小涟漪，反向再发。**不能是旋转 svg**——旋转的语义是
 *    「加载等待」，长任务时会被读成卡住。突触回路的往返读的是「在推进」。
 * ② 图标以 translate/scale/opacity 驱动，任何 turn-meta-* 关键帧禁见 rotate。
 * ③ 无动画降级时（prefers-reduced-motion）全家族 animation:none，静场只余
 *    轨道 + 双端节点（脉冲/彗尾基态 opacity 0，由关键帧点亮）。
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

describe('执行中状态图标：突触回路pulse往返而非旋转', () => {
  it('组件渲染回路全家（轨道/双端/脉冲/彗尾/涟漪×2），旧版类绝迹', () => {
    for (const cls of [
      'turn-meta-thinking',
      'turn-meta-thinking-axis',
      'turn-meta-thinking-node-l',
      'turn-meta-thinking-node-r',
      'turn-meta-thinking-pulse',
      'turn-meta-thinking-tail',
      'turn-meta-thinking-ring-l',
      'turn-meta-thinking-ring-r',
    ]) {
      expect(tsx).toContain(cls)
    }
    // 历史 spinner（旋转）与 v1 四角点 / v2 火花簇类必须绝迹，防回潮
    expect(tsx).not.toContain('turn-meta-spinner')
    expect(tsx).not.toContain('turn-meta-thinking-dot')
    expect(tsx).not.toContain('turn-meta-thinking-core')
  })

  it('行进关键帧 translate 驱动，全部关键帧禁见 rotate，旧关键帧绝锈', () => {
    for (const name of [
      'turn-meta-pulse',
      'turn-meta-tail',
      'turn-meta-node-l',
      'turn-meta-node-r',
      'turn-meta-ring-l',
      'turn-meta-ring-r',
    ]) {
      const kf = css.match(new RegExp(`@keyframes ${name}\\s*\\{[\\s\\S]*?\\n\\}`))?.[0] ?? ''
      expect(kf).not.toBe('')
    }
    for (const name of ['turn-meta-pulse', 'turn-meta-tail']) {
      const kf = css.match(new RegExp(`@keyframes ${name}\\s*\\{[\\s\\S]*?\\n\\}`))?.[0] ?? ''
      expect(kf).not.toBe('')
      expect(kf).not.toMatch(/rotate/)
      expect(kf).toMatch(/translateX\(/)
    }
    // 旋转=加载等待=禁；旧图案关键帧绝迹防回潮
    //（`ring\b` 会误把 `ring-l` 当边界，ring 系用负向前瞻放行）
    expect(css).not.toMatch(/@keyframes turn-meta-thinking-pulse\b/)
    expect(css).not.toMatch(/@keyframes turn-meta-spin\b/)
    expect(css).not.toMatch(/@keyframes turn-meta-dot-/)
    expect(css).not.toMatch(/@keyframes turn-meta-core\b/)
    expect(css).not.toMatch(/@keyframes turn-meta-ring(?!-l|-r)\b/)
  })

  it('动作行 chip 左侧不留 padding，左侧距离交给 SVG 图标', () => {
    const body = ruleBody('.turn-meta-bar--actions .turn-meta-chip {')
    expect(body).toMatch(/padding:\s*1px 6px 1px 0/)
  })

  it('无动画降级（prefers-reduced-motion）仍然可读：只余轨道+双端', () => {
    // 文件里早有其它 prefers-reduced-motion 块（如流光降级），须锁定图标那块
    const block =
      [...css.matchAll(/@media \(prefers-reduced-motion: reduce\) \{[\s\S]*?\n\}/g)].find(
        (m) => m[0].includes('.turn-meta-thinking-pulse,'),
      )?.[0] ?? ''
    expect(block).not.toBe('')
    expect(block).toContain('.turn-meta-thinking-tail')
    expect(block).toContain('.turn-meta-thinking-node-l')
    expect(block).toContain('.turn-meta-thinking-ring-l')
    expect(block).toContain('animation: none')
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
