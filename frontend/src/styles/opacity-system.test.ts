/**
 * 不透明度体系契约测试 —— 钉住「滑块 → 变量 → 消费者」这条链路的三个断面。
 *
 * 为什么需要这组用例（2026-09-26 第二轮，大王三条指令）：
 *
 * ① 输入框不跟透明度变化：`--input-bg` 虽在 `OPACITY_COLOR_KEYS` 里，但切主题时
 *    覆盖被 `stripOpacityColorKeys` 剥掉后是否**重派**，取决于「用户是否拖过」这一
 *    判定 —— 判定若只活在被 CompactModal 卸载即归零的组件 ref 上，最常见的
 *    「拖滑块 → 关弹窗 → 再切主题」路径就会静默丢透明度。
 *    故断言必须包含：消费键在集合内、集合覆盖 `--panel-bg`、intent 是持久层。
 *
 * ② 控制面板（设置中心）是弹窗却用不透明硬色 `--surface-0` → 滑块看不见它。
 *    修法是让它消费弹窗族语义键，故这里同时断言 CSS 消费者与 tokens 定义。
 *
 * ③ user/assistant 气泡一律去 border。用 CSS 源码断言（jsdom 不做布局/级联计算）。
 *
 * 读取方式沿用 app-shell-layout.test.ts 的既有结论：`?raw` 在本项目 Vite 配置下
 * 对 .css 返回空串（断言会静默落空），`node:fs` 又缺 @types/node —— 故用运行时
 * 动态引入 + 类型收敛。这些保证只在 CSS 文本层面可验证，故按源码断言。
 */

import { describe, expect, it } from 'vitest'
import { OPACITY_COLOR_KEYS, OPACITY_INTENT_KEY_ORDER } from '../hooks/customTheme'

/**
 * 运行时加载 Node 的 fs。
 *
 * 模块名由变量拼出（`'node:' + 'fs'`）：本项目 tsconfig 未启用 @types/node，
 * 写成字面量会被 tsc 直接报 TS2307；拼写后 tsc 无从静态解析，运行时（vitest
 * 跑在 Node 上）照常可用。readFileSync 接受 URL 对象，故入参如实标成 any。
 */
const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

/** 去掉注释，避免注释里提到的选择器/属性名把断言骗过去 */
const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

const tokensCss = stripComments(read('./tokens.css'))
const settingsCss = stripComments(read('./settings-center.css'))
const chatMessagesCss = stripComments(read('./chat-messages.css'))
const chatInputCss = stripComments(read('./chat-input.css'))

/** 取一条顶层规则的声明块（首个匹配） */
const ruleBody = (css: string, selector: string): string => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const re = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, 's')
  return re.exec(css)?.[1] ?? ''
}

describe('CSS 源码确实读进来了（防"读空文件导致断言静默通过"）', () => {
  it('四个样式文件非空且含预期选择器', () => {
    expect(tokensCss.length).toBeGreaterThan(1000)
    expect(settingsCss).toContain('.settings-center-panel')
    expect(chatMessagesCss).toContain('.message-bubble')
    expect(chatInputCss).toContain('.chat-input-area')
  })
})

describe('① 滑块键集合必须包含输入框 / 面板 / 弹窗', () => {
  it('--input-bg 在集合内（输入框背景的可调性由此保证）', () => {
    expect(OPACITY_COLOR_KEYS).toContain('--input-bg')
  })

  it('--panel-bg 在集合内（设置中心并于弹窗透明度体系）', () => {
    expect(OPACITY_COLOR_KEYS).toContain('--panel-bg')
  })

  it('--modal-bg 与气泡双键在集合内（原有能力不得回退）', () => {
    expect(OPACITY_COLOR_KEYS).toContain('--modal-bg')
    expect(OPACITY_COLOR_KEYS).toContain('--msg-user-bg')
    expect(OPACITY_COLOR_KEYS).toContain('--msg-assistant-bg')
  })

  it('意图持久层覆盖集合内全部颜色通道（漏一个 → 该通道重开页面后丢透明度）', () => {
    // intent 是「跨卸载」的用户声明存储；集合里每加一个**语义域**，intent 必须同步，
    // 否则该域重开页面后丢透明度。
    //
    // 关系不是 1:1：`--msg-user-bg` / `--msg-assistant-bg` 两个键由同一个 `bubbles`
    // 通道驱动（一次拖动同时改两者，见 ThemesPage.handleBubbleOpacity），
    // 其余三个键各占一个通道。故颜色键数 = 通道数 + 1。这个 +1 是**被断言锚定的常量**：
    // 若有人新增一个独立语义域却忘了加 intent 通道，或把气泡拆成两个通道，这里立刻失败。
    const colorChannels = OPACITY_INTENT_KEY_ORDER.filter(c => c !== 'skin')
    expect(OPACITY_COLOR_KEYS.length).toBe(colorChannels.length + 1)

    // 且并集必须恰好是那 4 个颜色域，不允许出现「跑了但没记住」的通道
    expect(colorChannels).toEqual(['bubbles', 'input', 'panel', 'modal'])
  })
})

describe('② 控制面板消费弹窗族语义键（而非不透明硬色）', () => {
  it('面板底走 --panel-bg', () => {
    const body = ruleBody(settingsCss, '.settings-center-panel')
    expect(body).toMatch(/background:\s*var\(--panel-bg\)/)
  })

  it('面板不再使用不透明硬色 --surface-0 作为底色', () => {
    const body = ruleBody(settingsCss, '.settings-center-panel')
    expect(body).not.toMatch(/background:\s*var\(--surface-0\)/)
  })

  it('面板保留毛玻璃层（与 CompactModal 同范式，缺它则半透明底叠影）', () => {
    const body = ruleBody(settingsCss, '.settings-center-panel')
    expect(body).toMatch(/backdrop-filter:\s*blur\(24px\)/)
    expect(body).toMatch(/-webkit-backdrop-filter:\s*blur\(24px\)/)
  })

  it('三主题都定义了 --panel-bg，且与各自 --modal-bg 同源', () => {
    const panelDefs = tokensCss.match(/--panel-bg:\s*[^;]+;/g) ?? []
    const modalDefs = tokensCss.match(/--modal-bg:\s*[^;]+;/g) ?? []
    // :root（dark）+ light + tech
    expect(panelDefs).toHaveLength(3)
    expect(modalDefs).toHaveLength(3)
    const valueOf = (s: string) => s.replace(/^--[\w-]+:\s*/, '').replace(/;$/, '').trim()
    expect(panelDefs.map(valueOf)).toEqual(modalDefs.map(valueOf))
  })

  it('宿主遮罩 --overlay-bg 未被本次改动波及（仍在宿主规则上）', () => {
    expect(ruleBody(settingsCss, '.settings-center-host')).toMatch(/background:\s*var\(--overlay-bg\)/)
  })
})

describe('① 消费端接得住变量', () => {
  it('.chat-input-area 底走 --input-bg（滑块 → 变量 → 消费者贯通）', () => {
    expect(ruleBody(chatInputCss, '.chat-input-area')).toMatch(/background:\s*var\(--input-bg\)/)
  })
})

describe('③ user / assistant 气泡一律无 border', () => {
  it('.message-row.user .message-bubble 无 border', () => {
    const body = ruleBody(chatMessagesCss, '.message-row.user .message-bubble')
    expect(body).not.toMatch(/\bborder\s*:/)
    expect(body).not.toMatch(/\bborder-(top|right|bottom|left|color|width|style)\s*:/)
  })

  it('.message-row.assistant .message-bubble 无 border', () => {
    const body = ruleBody(chatMessagesCss, '.message-row.assistant .message-bubble')
    expect(body).not.toMatch(/\bborder\s*:/)
    expect(body).not.toMatch(/\bborder-(top|right|bottom|left|color|width|style)\s*:/)
  })

  it('ThinkingIndicator 的 border 不受波及（它不是气泡）', () => {
    // 该选择器块以 .thinking-indicator 开头，含 border: 1px solid var(--border-secondary)
    const idx = chatMessagesCss.indexOf('.thinking-indicator')
    expect(idx).toBeGreaterThan(-1)
    expect(chatMessagesCss.slice(idx, idx + 600)).toMatch(
      /border:\s*1px solid var\(--border-secondary\)/,
    )
  })

  it('气泡仍保留各自底色（去 border 不等于去背景）', () => {
    expect(ruleBody(chatMessagesCss, '.message-row.user .message-bubble')).toMatch(
      /background:\s*var\(--msg-user-bg\)/,
    )
    expect(ruleBody(chatMessagesCss, '.message-row.assistant .message-bubble')).toMatch(
      /background:\s*var\(--msg-assistant-bg\)/,
    )
  })

  it('未用 !important 绕过（铁律：必须让链路贯通）', () => {
    const blocks = [
      ruleBody(chatMessagesCss, '.message-row.user .message-bubble'),
      ruleBody(chatMessagesCss, '.message-row.assistant .message-bubble'),
      ruleBody(settingsCss, '.settings-center-panel'),
    ]
    for (const block of blocks) expect(block).not.toContain('!important')
  })
})
