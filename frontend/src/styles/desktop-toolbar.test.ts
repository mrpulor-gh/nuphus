/**
 * desktop-toolbar.css 契约测试 —— 钉死「右侧固定按钮列 + hover 从上到下逐次显形」的几条硬约束。
 *
 * 为什么按源码断言（沿用 opacity-system.test.ts 的既有结论）：jsdom 不做布局与
 * 级联计算，`:hover`、`transform` 的最终位置、透明元素是否仍占盒都验证不了；
 * 这些约束的失效形态恰恰是「鼠标够不着 / 热区比看起来大 / 错峰没生效」这种肉眼难察的回归。
 *
 * 必须钉死的纪律：
 * ① 默认态走 opacity（**不是 pointer-events:none / display:none**）——那会让 hover 永远触发不了。
 * ② 热区 = 按钮列自身盒子：dock 不得用 padding / margin / inset 撑出比列更大的隐形触发区。
 * ③ 错峰只作用在 opacity/transform 上，且**收起方向零延迟**（否则鼠标划过留下拖尾）。
 * ④ 竖条包裹层与把手已删除，不得复活。
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

const toolbarCss = stripComments(read('./desktop-toolbar.css'))
const chatMessagesCss = stripComments(read('./chat-messages.css'))

/** 按单选择器取声明块（合并选择器会静默取空 —— 这正是 chat-messages.css 那三条不能合并的原因） */
const ruleBody = (css: string, selector: string): string => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const re = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, 's')
  return re.exec(css)?.[1] ?? ''
}

describe('desktop-toolbar.css 源码确实读进来了（防"读空文件导致断言静默通过"）', () => {
  it('本文件与参照文件非空且含预期选择器', () => {
    expect(toolbarCss.length).toBeGreaterThan(500)
    expect(toolbarCss).toContain('.desktop-toolbar-dock')
    expect(chatMessagesCss.length).toBeGreaterThan(1000)
  })
})

describe('① 右侧固定按钮列：竖条包裹层与把手已删除', () => {
  it('.desktop-toolbar（竖条容器）规则整体消失', () => {
    expect(ruleBody(toolbarCss, '.desktop-toolbar')).toBe('')
    expect(toolbarCss).not.toMatch(/^\.desktop-toolbar\s*\{/m)
  })

  it('.desktop-toolbar-handle 规则整体消失', () => {
    expect(toolbarCss).not.toContain('desktop-toolbar-handle')
  })

  it('.desktop-toolbar-dock:hover .desktop-toolbar（旧的整列显隐）已移除', () => {
    expect(toolbarCss).not.toContain('.desktop-toolbar-dock:hover .desktop-toolbar {')
  })

  it('.desktop-toolbar-divider（分隔符，随置顶一并移除）已删除', () => {
    expect(toolbarCss).not.toContain('desktop-toolbar-divider')
  })
})

describe('② dock 定位与排布：对齐 .chat-header-right', () => {
  const dock = ruleBody(toolbarCss, '.desktop-toolbar-dock')

  it('fixed + 垂直居中 + 右缘 20px', () => {
    expect(dock).toMatch(/position:\s*fixed/)
    expect(dock).toMatch(/top:\s*50%/)
    expect(dock).toMatch(/transform:\s*translateY\(-50%\)/)
    expect(dock).toMatch(/right:\s*20px/)
  })

  it('右缘 20px 的推导：.chat-header right:16px + padding:4px', () => {
    // 直接从基准文件取当前实际值推导，而不是写死 20 —— 基准若被他人改动，这里会红
    const header = ruleBody(chatMessagesCss, '.chat-header')
    const right = Number(header.match(/right:\s*(\d+)px/)?.[1])
    const pad = Number(header.match(/padding:\s*(\d+)px/)?.[1])
    expect(right + pad).toBe(20)
  })

  it('竖向 flex 列 + gap 4px + 右对齐（与 .chat-header-right 同配方）', () => {
    expect(dock).toMatch(/display:\s*flex/)
    expect(dock).toMatch(/flex-direction:\s*column/)
    expect(dock).toMatch(/align-items:\s*flex-end/)
    expect(dock).toMatch(/gap:\s*4px/)
  })

  it('层级保留 9999', () => {
    expect(dock).toMatch(/z-index:\s*9999/)
  })

  it('热区纪律：dock 无 padding / margin / inset / 铺满式尺寸', () => {
    expect(dock).not.toMatch(/padding/)
    expect(dock).not.toMatch(/margin/)
    expect(dock).not.toMatch(/inset/)
    expect(dock).not.toMatch(/height:\s*100%/)
    expect(dock).toMatch(/pointer-events:\s*auto/)
    expect(dock).not.toMatch(/pointer-events:\s*none/)
  })
})

describe('③ hover 逐次显形：从上到下错峰，收起无拖尾', () => {
  const btn = ruleBody(toolbarCss, '.desktop-toolbar-btn')
  const hover = ruleBody(toolbarCss, '.desktop-toolbar-dock:hover .desktop-toolbar-btn')

  it('默认态 opacity:0 + translateY(-4px)，但仍占布局盒（不得 display:none）', () => {
    expect(btn).toMatch(/opacity:\s*0/)
    expect(btn).toMatch(/transform:\s*translateY\(-4px\)/)
    expect(btn).not.toMatch(/display:\s*none/)
    expect(btn).not.toMatch(/pointer-events:\s*none/)
  })

  it('hover 态翻到 opacity:1 + translateY(0)', () => {
    expect(hover).toMatch(/opacity:\s*1/)
    expect(hover).toMatch(/transform:\s*translateY\(0\)/)
  })

  it('错峰走 CSS 变量 --stagger-delay，且带 0ms 兜底（未注入变量的元素不失效）', () => {
    expect(hover).toMatch(/transition-delay:\s*var\(--stagger-delay,\s*0ms\)/)
  })

  it('错峰只作用于 opacity / transform，后三个属性保持 0 延迟', () => {
    const delays = hover.match(/transition-delay:([^;]*);/s)?.[1] ?? ''
    // 不能按 ',' 裸切：var(--stagger-delay, 0ms) 内部自带逗号。
    // 先整体吃掉 var(...) 再切其余裸值。
    const values = delays.match(/var\([^)]*\)|[^,\s]+/g) ?? []
    expect(values).toHaveLength(5)
    expect(values.slice(0, 2).every(v => v.startsWith('var(--stagger-delay'))).toBe(true)
    expect(values.slice(2).every(v => v === '0ms')).toBe(true)
  })

  it('transition 显式列出属性且**不含 all**（all 会把 delay 套到背景/颜色上）', () => {
    expect(btn).not.toMatch(/transition:\s*all\b/)
    const transition = btn.match(/transition:\s*([^;]*);/s)?.[1] ?? ''
    for (const prop of ['opacity', 'transform', 'background', 'color', 'border-color']) {
      expect(transition).toContain(prop)
    }
  })

  it('默认态 transition-delay 归零 —— 收起必须同步，不能逐个拖尾', () => {
    expect(btn).toMatch(/transition-delay:\s*0ms\s*;/)
  })

  it(':active 压感叠加在入场位移之上，且选择器带 dock:hover 以压过显隐规则', () => {
    const active = ruleBody(toolbarCss, '.desktop-toolbar-dock:hover .desktop-toolbar-btn:active')
    expect(active).toMatch(/transform:\s*translateY\(0\) scale\(0\.94\)/)
    expect(toolbarCss).not.toMatch(/^\.desktop-toolbar-btn:active\s*\{/m)
  })

  it('prefers-reduced-motion 降级：关位移、关错峰，只留 opacity', () => {
    const at = toolbarCss.indexOf('@media (prefers-reduced-motion: reduce)')
    expect(at).toBeGreaterThan(-1)
    // 整个 media 块（到行首 '}' 为止），避免只截到第一条规则让断言半真半假
    const end = toolbarCss.indexOf('\n}', at)
    expect(end).toBeGreaterThan(at)
    const block = stripComments(toolbarCss.slice(at, end))
    // 逐条规则取声明块：**不能**用跨块的 [\s\S]* 贪心匹配 —— 那会一路吃到
    // media 块之外的 hover 规则里，把「块内没写」判成「块内写了」。
    const base = ruleBody(block, '.desktop-toolbar-btn')
    const hoverInBlock = ruleBody(block, '.desktop-toolbar-dock:hover .desktop-toolbar-btn')
    expect(base).toMatch(/transform:\s*none/)
    expect(base).toMatch(/transition-delay:\s*0ms/)
    expect(base).toMatch(/transition:\s*opacity var\(--transition-fast\)/)
    // hover 态的入场位移也要一并关掉，否则会被块外显隐规则的 translateY(0) 顶掉
    expect(hoverInBlock).toMatch(/transform:\s*none/)
  })

  it('结果浮窗的 toolbarFadeIn 入场动画仍在（被 .desktop-toolbar-result 引用，禁止删除）', () => {
    const result = ruleBody(toolbarCss, '.desktop-toolbar-result')
    expect(result).toMatch(/animation:\s*toolbarFadeIn/)
    expect(toolbarCss).toMatch(/@keyframes toolbarFadeIn\s*\{/)
    // 只允许一份定义
    expect(toolbarCss.match(/@keyframes toolbarFadeIn/g)?.length).toBe(1)
  })
})

describe('④ 坐标读数 + 关闭**常驻**（不受整列 hover 显隐约束）', () => {
  const group = ruleBody(toolbarCss, '.desktop-toolbar-pos-group')
  const pos = ruleBody(toolbarCss, '.desktop-toolbar-pos')
  const close = ruleBody(toolbarCss, '.desktop-toolbar-pos-close')

  /**
   * 2026-10-09 大王报障「点击鼠标工具后坐标位置获取不到」。
   * 真根因：整列默认 opacity:0、hover dock 才显形；坐标读数也跟着隐藏，
   * 用户为看读数把鼠标移开 dock 就什么都看不见，关闭钮也一并消失。
   * 故这里**反向**钉死：读数不得再有任何 opacity 闸门，也不得存在
   * 「hover 才显形」的规则 —— 否则该 bug 会原地复活。
   */
  it('读数**没有** opacity 闸门（默认即完全可见）', () => {
    expect(pos).not.toBe('')
    expect(pos).not.toMatch(/opacity:/)
  })

  it('不存在 .desktop-toolbar-dock:hover .desktop-toolbar-pos 显隐规则', () => {
    const hoverRule = ruleBody(toolbarCss, '.desktop-toolbar-dock:hover .desktop-toolbar-pos')
    expect(hoverRule).toBe('')
    // 更强的一条：任何 dock:hover 规则都不得引用 .desktop-toolbar-pos
    // （防换一种写法绕过上一条 —— 空转断言等于没断言）
    const dockHoverRules = toolbarCss.match(/\.desktop-toolbar-dock:hover[^{]*\{[^}]*\}/g) ?? []
    expect(dockHoverRules.length).toBeGreaterThan(0)
    for (const r of dockHoverRules) expect(r).not.toContain('.desktop-toolbar-pos')
  })

  it('读数与关闭在同一行分组内（原实现关闭在读数下方）', () => {
    expect(group).toMatch(/display:\s*flex/)
    expect(group).toMatch(/align-items:\s*center/)
    expect(group).toMatch(/gap:/)
    // 分组整体向左浮出，不参与列排版（否则读数宽度会撑宽热区）
    expect(group).toMatch(/position:\s*absolute/)
    expect(group).toMatch(/right:\s*calc\(100% \+ 8px\)/)
  })

  it('关闭钮沿用同一份磨砂配方，颜色走 --error token', () => {
    expect(close).toMatch(/color:\s*var\(--error\)/)
    expect(close).toContain('color-mix(in srgb, var(--panel-bg) 85%, transparent)')
    expect(close).toContain('var(--border)')
    expect(close).not.toContain('#')
  })

  it('读数与关闭都不吃错峰（它们不是列成员，无 --stagger-delay）', () => {
    expect(pos).not.toContain('--stagger-delay')
    expect(close).not.toContain('--stagger-delay')
    expect(group).not.toContain('--stagger-delay')
  })
})

describe('⑤ 视觉语言沿用 header 右列（30×30 磨砂配方逐字一致）', () => {
  const btn = ruleBody(toolbarCss, '.desktop-toolbar-btn')

  it('按钮为 30×30 方钮 + panel-bg 62% 磨砂 + blur(8px) saturate(1.2)', () => {
    expect(btn).toMatch(/width:\s*30px/)
    expect(btn).toMatch(/height:\s*30px/)
    expect(btn).toMatch(/border:\s*1px solid var\(--border\)/)
    expect(btn).toMatch(/background:\s*color-mix\(in srgb, var\(--panel-bg\) 62%, transparent\)/)
    expect(btn).toMatch(/backdrop-filter:\s*blur\(8px\) saturate\(1\.2\)/)
    expect(btn).toMatch(/color:\s*var\(--spark-secondary\)/)
    expect(btn).toMatch(/border-radius:\s*8px/)
  })

  it('hover 档抬到 85% 混合比并换 spark-primary', () => {
    const hover = ruleBody(toolbarCss, '.desktop-toolbar-btn:hover')
    expect(hover).toMatch(/color-mix\(in srgb, var\(--panel-bg\) 85%, transparent\)/)
    expect(hover).toMatch(/border-color:\s*var\(--glass-3\)/)
    expect(hover).toMatch(/color:\s*var\(--spark-primary\)/)
  })

  it('已移除的置顶硬编码色 #3b82f6 不再出现在样式里', () => {
    expect(toolbarCss).not.toContain('#3b82f6')
    expect(toolbarCss).not.toContain('IconPin')
  })

  it('文字标签绝对定位向左浮出（不撑开 30px 列）', () => {
    const label = ruleBody(toolbarCss, '.desktop-toolbar-label')
    expect(label).toMatch(/position:\s*absolute/)
    expect(label).toMatch(/right:\s*calc\(100%/)
    expect(ruleBody(toolbarCss, '.desktop-toolbar-btn:hover .desktop-toolbar-label')).toMatch(
      /visibility:\s*visible/,
    )
  })
})

describe('⑥ chat-messages.css 的三条 header 按钮规则仍为单选择器（合并会让回归断言静默落空）', () => {
  const selectors = [
    '.chat-header-settings-btn',
    '.chat-header-appearance-btn',
    '.chat-header-browser-btn',
  ]
  for (const selector of selectors) {
    it(`${selector} 仍能按单选择器取到完整磨砂配方`, () => {
      const body = ruleBody(chatMessagesCss, selector)
      expect(body).toMatch(/width:\s*30px/)
      expect(body).toMatch(/height:\s*30px/)
      expect(body).toMatch(/color-mix\(in srgb, var\(--panel-bg\) 62%, transparent\)/)
      expect(body).toMatch(/backdrop-filter:\s*blur\(8px\) saturate\(1\.2\)/)
    })
  }
})
