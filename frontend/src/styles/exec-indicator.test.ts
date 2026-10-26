/**
 * exec-indicator.css 契约测试 —— 钉住「复用 mobile 环绕手法 + 反色弧条 + 零硬编码绘制色」。
 *
 * 为什么按源码断言（沿用 desktop-toolbar.test.ts / opacity-system.test.ts 的既有结论）：
 * jsdom 不做布局、不做级联计算、不跑动画 —— conic-gradient 弧、mask 成环、
 * 「动画只在执行态」这类约束在渲染层验证不了，只能钉源码。
 *
 * 必须钉死的纪律（对应任务要求）：
 * ① 环绕手法逐条复用 frontend/src/mobile/mobile.css:1614-1638 的
 *    conic-gradient 画 1/4 弧 + radial-gradient mask 成环 + 1s linear 旋转 360°；
 * ② 弧条色是**主题反色**（--fg-1，亮主题深弧 / 暗主题白弧），不得出现硬编码色值；
 * ③ 业务 CSS 不得写 [data-theme=…] 覆盖块（双主题规则，差异只允许活在 tokens.css）；
 * ④ 完成态环绕停止（旋转动画只挂在执行态上）+ 完整闭环；
 * ⑤ 退场时长只由 TS 常量注入的 --exec-leave-duration 决定，CSS 不另写数值；
 * ⑥ 定位：absolute 挂进 .chat-header-left（不能是 fixed —— 会被顶部 in-flow 的
 *    更新提示 / macOS 权限通告压住或反压），left 4px 避开会话色块的 24px 命中热区；
 * ⑦ prefers-reduced-motion 降级：环不转但仍在、退场只淡出不做位移。
 */

import { describe, expect, it } from 'vitest'

/**
 * 运行时加载 Node 的 fs。模块名由变量拼出：本项目 tsconfig 未启用 @types/node，
 * 写成字面量会被 tsc 报 TS2307（与 opacity-system.test.ts 同一处理）。
 */
const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

/** 去掉注释，避免注释里提到的属性名把断言骗过去 */
const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

const css = stripComments(read('./exec-indicator.css'))
const mobileCss = stripComments(read('../mobile/mobile.css'))

/** 按单选择器取声明块 */
const ruleBody = (src: string, selector: string): string => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const re = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, 's')
  return re.exec(src)?.[1] ?? ''
}

describe('exec-indicator.css 源码确实读进来了（防"读空文件导致断言静默通过"）', () => {
  it('本文件与参照文件非空且含预期选择器', () => {
    expect(css.length).toBeGreaterThan(1000)
    expect(css).toContain('.exec-indicator')
    expect(mobileCss.length).toBeGreaterThan(10000)
    expect(mobileCss).toContain('.mobile-composer-stop')
  })
})

describe('① 环绕手法复用 mobile.css（conic-gradient + mask + 1s 旋转）', () => {
  it('参照物仍在：mobile 的 .mobile-composer-stop::before 就是这套 recipe', () => {
    const mobile = ruleBody(mobileCss, '.mobile-composer-stop::before')
    expect(mobile).toContain('conic-gradient')
    expect(mobile).toContain('mask: radial-gradient')
    expect(mobile).toContain('animation: mobile-stop-spin 1s linear infinite')
  })

  it('本文件照抄同款：1/4 弧（0deg→90deg）+ radial mask 成环', () => {
    const ring = ruleBody(css, '.exec-indicator::before')
    expect(ring).toContain('conic-gradient')
    // 弧条数据：transparent 0deg → 反色 90deg → transparent 90deg（1/4 弧）
    expect(ring).toMatch(/transparent\s+0deg/)
    expect(ring).toMatch(/var\(--exec-arc\)\s+90deg/)
    expect(ring).toMatch(/transparent\s+90deg/)
    // 环：mask 掏空中心（-webkit-mask 与标准 mask 都要有）
    expect(ring).toContain('-webkit-mask: radial-gradient(farthest-side')
    expect(ring).toMatch(/\smask: radial-gradient\(farthest-side/)
    // 环是纯装饰层，不参与命中测试
    expect(ring).toContain('pointer-events: none')
    // 外边距：环外扩（-4px），与头像之间留细间隔
    expect(ring).toContain('inset: -4px')
  })

  it('旋转 360°：keyframes 与用法都在，且 1s linear infinite', () => {
    expect(ruleBody(css, '.exec-indicator.is-running::before')).toContain(
      'animation: exec-indicator-spin 1s linear infinite',
    )
    const kf = css.slice(css.indexOf('@keyframes exec-indicator-spin'))
    expect(kf).toContain('rotate(360deg)')
  })
})

describe('② 弧条色 = 主题反色，且全文件零硬编码绘制色', () => {
  it('--exec-arc 由 --fg-1 派生（本仓库既有反色口径：session-rail / LetterAvatar）', () => {
    const root = ruleBody(css, '.exec-indicator')
    expect(root).toMatch(/--exec-arc:\s*color-mix\(in srgb,\s*var\(--fg-1\)/)
  })

  it('反色键 --fg-1 确有三主题取值（亮主题深弧 / 暗主题白弧 的前提）', () => {
    const tokens = stripComments(read('./tokens.css'))
    const themeBlocks = [
      /(?<=:root\s*\{)[\s\S]*?(?=\n\})/,
      /\[data-theme='light'\]\s*\{[\s\S]*?\n\}/,
      /\[data-theme='fresh'\]\s*\{[\s\S]*?\n\}/,
    ]
    for (const re of themeBlocks) {
      const block = re.exec(tokens)?.[0] ?? ''
      expect(block).toContain('--fg-1:')
    }
  })

  it('没有硬编码绘制色（rgba/hex 一律为零；mask 里的 #000 是 alpha 通道不算绘制色）', () => {
    // 注释已剥离：剩下的色值只可能来自声明
    const painted = css.match(/#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\)|hsla?\([^)]*\)/g) ?? []
    expect(painted).toEqual(['#000', '#000']) // mask / -webkit-mask 两处
  })
})

describe('③ 双主题规则：业务 CSS 不写主题覆盖块', () => {
  it('没有 [data-theme=…] 选择器', () => {
    expect(css).not.toContain('[data-theme')
  })
})

describe('④ 完成态：环绕停止 + 完整闭环（动画只挂执行态）', () => {
  it('旋转动画仅存在于 .is-running::before 一处', () => {
    const spinUses = css.match(/animation:\s*exec-indicator-spin/g) ?? []
    expect(spinUses).toHaveLength(1)
    expect(css.slice(0, css.indexOf('animation: exec-indicator-spin'))).toContain(
      '.is-running::before',
    )
  })

  it('完成态把 1/4 弧切成完整闭环（--success 实心环）', () => {
    expect(ruleBody(css, '.exec-indicator.is-completed::before')).toContain(
      'background: var(--success)',
    )
  })

  it('完成勾走状态色 --success（与闭环同色成一体）', () => {
    const check = ruleBody(css, '.exec-indicator-check')
    expect(check).toContain('background: var(--success)')
    expect(check).toContain('border-radius: 50%')
  })
})

describe('⑤ 退场：时长只由 TS 注入的变量决定', () => {
  it('.is-leaving 消费 --exec-leave-duration，且不写死 ms 数值', () => {
    const leaving = ruleBody(css, '.exec-indicator.is-leaving')
    expect(leaving).toContain('animation: exec-indicator-leave var(--exec-leave-duration)')
    expect(leaving).not.toMatch(/\d+ms/)
  })

  it('退场是「向左移出 + 消失」：keyframes 里同时位移与淡出', () => {
    const kf = css.slice(css.indexOf('@keyframes exec-indicator-leave'))
    expect(kf).toMatch(/translate\(-64px,\s*-50%\)/)
    expect(kf).toContain('opacity: 0')
  })
})

describe('⑥ 定位：挂 header 而非 fixed 到窗口角', () => {
  it('.exec-indicator 是 absolute，不参与 header 布局', () => {
    const root = ruleBody(css, '.exec-indicator')
    expect(root).toContain('position: absolute')
    expect(root).not.toContain('position: fixed')
  })

  it('left: 4px —— 避开会话色块 24px 命中热区（否则左缘点击被色块抢走）', () => {
    const root = ruleBody(css, '.exec-indicator')
    expect(root).toContain('left: 4px')
    const rail = stripComments(read('./session-rail.css'))
    expect(ruleBody(rail, '.session-rail-chip::after')).toContain('width: 24px')
  })

  it('贴 header 左上角顶部（top: 0），不再落到 header 垂直中线上', () => {
    // 2026-10-26 大王真机反馈「整体 DOM 偏右下」：原方案与右侧按钮列对齐中线，
    // 而右侧是纵向三钮、header 被撑高，头像遂落在 header 垂直中央。现改为贴顶。
    const root = ruleBody(css, '.exec-indicator')
    expect(root).toContain('top: 0')
    expect(root).not.toContain('translateY')
  })

  it('头像 36px 圆 + 与消息行头像同底（--surface-3），视觉相融', () => {
    const root = ruleBody(css, '.exec-indicator')
    expect(root).toContain('width: 36px')
    expect(root).toContain('height: 36px')
    expect(root).toContain('border-radius: 50%')
    expect(root).toContain('background: var(--surface-3)')
    const msgAvatar = ruleBody(stripComments(read('./chat-messages.css')), '.message-avatar')
    expect(msgAvatar).toContain('background: var(--void-tertiary)') // 别名 → --surface-3
  })

  it('交互三态齐备：hover / :active / :focus-visible', () => {
    expect(ruleBody(css, '.exec-indicator:hover')).toContain('background: var(--surface-hover)')
    expect(ruleBody(css, '.exec-indicator:active')).toContain('background: var(--surface-active)')
    expect(ruleBody(css, '.exec-indicator:focus-visible')).toContain(
      'outline: 2px solid var(--accent)',
    )
  })
})

describe('⑦ prefers-reduced-motion 降级：状态仍可读、消失不丢失', () => {
  const block = /@media \(prefers-reduced-motion: reduce\)\s*\{[\s\S]*?\n\}/.exec(css)?.[0] ?? ''

  it('降级块存在', () => {
    expect(block).not.toBe('')
  })

  it('环不转但仍在（1/4 弧保留）；退场只淡出不做位移', () => {
    expect(block).toContain('.exec-indicator.is-running::before')
    expect(block).toContain('animation: none')
    expect(block).toContain('animation-name: exec-indicator-leave-fade')
    const fade = css.slice(css.indexOf('@keyframes exec-indicator-leave-fade'))
    expect(fade).toContain('opacity: 0')
    expect(fade).not.toContain('translate')
  })
})
