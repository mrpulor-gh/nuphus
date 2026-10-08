import { render, screen, fireEvent, act } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { PreviewOverlay } from './PreviewOverlay'
import { hudUpdate } from '../lib/api'

// P1 HTML 标注回流：overlay 脚本经 preview:// 注入后向父窗口 postMessage。
// P1 覆盖父窗口侧契约（listener/source 防伪造/握手兜底）；P2 覆盖「发送给 Agent」
// 面板操作（序列化引用 / 放弃 / 空载荷禁用）。协议字段以 preview_protocol.rs 为准。

vi.mock('../lib/api', () => ({
  readFile: vi.fn(() => Promise.resolve('')),
  readFileBase64: vi.fn(() => Promise.resolve('')),
  openPath: vi.fn(() => Promise.resolve()),
  revealPath: vi.fn(() => Promise.resolve()),
  hudUpdate: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: vi.fn((p: string, protocol = 'asset') => `http://${protocol}.localhost/${p}`),
}))

vi.mock('pdfjs-dist', () => ({ GlobalWorkerOptions: {}, getDocument: vi.fn() }))

/** 以指定 source 派发 message（jsdom 的 init 支持 source，显式 defineProperty 双保险） */
function dispatchMessage(data: unknown, source: unknown) {
  const event = new MessageEvent('message', { data })
  Object.defineProperty(event, 'source', { value: source, configurable: true })
  act(() => {
    window.dispatchEvent(event)
  })
}

/** 渲染 HTML 预览并触发 iframe onLoad，返回该 iframe */
function renderHtmlPreview(path = 'C:/work/demo.html', onSendAnnotations?: (ref: unknown) => void) {
  render(
    <PreviewOverlay path={path} onClose={() => undefined} onSendAnnotations={onSendAnnotations} />,
  )
  const iframe = document.querySelector('iframe.pv-iframe') as HTMLIFrameElement | null
  if (!iframe) throw new Error('HTML 预览分支未渲染 iframe')
  fireEvent.load(iframe)
  return iframe
}

/** P4：✎ 悬浮编辑按钮（HTML 预览唯一的标注入口） */
function editToggle(): HTMLButtonElement {
  const btn = document.querySelector('button.pv-edit-toggle')
  if (!btn) throw new Error('未渲染编辑入口按钮')
  return btn as HTMLButtonElement
}

/** 进入编辑态（点 ✎）并返回该 iframe 的 postMessage spy（验证下行指令） */
function enterAnnotating(iframe: HTMLIFrameElement) {
  const postSpy = vi.spyOn(iframe.contentWindow as Window, 'postMessage')
  fireEvent.click(editToggle())
  return postSpy
}

/** 面板标题的当前文案（列表展开时可见） */
function panelTitle(): string {
  return document.querySelector('.pv-annot-panel-title')?.textContent ?? ''
}

/** 列表项批注输入框（按序号取，从 1 开始） */
function commentInput(n: number): HTMLTextAreaElement {
  const list = document.querySelectorAll('.pv-annot-input')
  const el = list[n - 1]
  if (!el) throw new Error(`未找到第 ${n} 条的批注输入框`)
  return el as HTMLTextAreaElement
}

const EMPTY_PAYLOAD = {
  type: 'nuphus:annotations',
  file: 'demo.html',
  annotations: [],
}

/** P3 区域框选 payload：单条带 region 附加字段（既有字段零改） */
const REGION_PAYLOAD = {
  type: 'nuphus:annotations',
  file: 'demo.html',
  annotations: [
    {
      css_selector: 'body > div.hero:nth-child(2)',
      outer_html_snippet: '<div class="hero">…</div>',
      rect: { x: 10, y: 20, w: 300, h: 180 },
      region: { x: 12, y: 24, w: 280, h: 160 },
      dpr: 1.25,
      comment: '这一块整体不对',
    },
  ],
}

const PAYLOAD = {
  type: 'nuphus:annotations',
  file: 'demo.html',
  annotations: [
    {
      css_selector: 'body > div.a:nth-child(2) > p',
      outer_html_snippet: '<p>标题</p>',
      rect: { x: 12, y: 34, w: 120, h: 24 },
      dpr: 1.25,
      comment: '标题改成蓝色',
    },
    {
      css_selector: 'footer:nth-child(3)',
      outer_html_snippet: '<footer>…</footer>',
      rect: { x: 0, y: 300, w: 800, h: 40 },
      dpr: 1.25,
      comment: '',
    },
  ],
}

describe('PreviewOverlay HTML 标注回流（P1）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
  })

  it('iframe onLoad 之后才注册 message listener', () => {
    const addSpy = vi.spyOn(window, 'addEventListener')
    try {
      render(<PreviewOverlay path="C:/work/demo.html" onClose={() => undefined} />)
      const iframe = document.querySelector('iframe.pv-iframe') as HTMLIFrameElement
      // 挂载时只注册 Esc 监听，message listener 必须等 iframe 就绪
      expect(addSpy).not.toHaveBeenCalledWith('message', expect.any(Function))
      fireEvent.load(iframe)
      expect(addSpy).toHaveBeenCalledWith('message', expect.any(Function))
    } finally {
      addSpy.mockRestore()
    }
  })

  it('伪造 source 的 nuphus:annotations 必须被忽略', () => {
    const iframe = renderHtmlPreview()
    // 空对象 / 父窗口自身 / null：都不是本 iframe 的 contentWindow
    dispatchMessage(PAYLOAD, {})
    dispatchMessage(PAYLOAD, window)
    dispatchMessage(PAYLOAD, null)
    // iframe 元素本身也不行（校验基准是 contentWindow，不是 iframe 节点）
    dispatchMessage(PAYLOAD, iframe)
    expect(screen.queryByText('收到 2 条标注')).not.toBeInTheDocument()
    expect(screen.queryByText('标题改成蓝色')).not.toBeInTheDocument()
  })

  it('本 iframe contentWindow 发来的标注进入右侧修改列表', () => {
    const iframe = renderHtmlPreview()
    enterAnnotating(iframe)
    expect(iframe.contentWindow).not.toBeNull()
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    expect(panelTitle()).toContain('2 条标注')
    const panel = document.querySelector('.pv-annot-panel')
    expect(panel).not.toBeNull()
    expect(panel?.querySelector('.pv-annot-panel-file')?.textContent).toBe('demo.html')
    // 批注改由列表项的 textarea 承载（P4：iframe 侧批注框已删）
    expect(commentInput(1).value).toBe('标题改成蓝色')
    expect(commentInput(2).value).toBe('')
    expect(screen.getByText('body > div.a:nth-child(2) > p')).toBeInTheDocument()
    // outerHTML 摘要进 <details> 折叠（截断渲染，防爆上下文）
    expect(screen.getByText('<p>标题</p>')).toBeInTheDocument()
    // rect 摘要行（w×h @(x,y) dpr）
    expect(screen.getByText(/120×24 @\(12,34\) dpr 1\.25/)).toBeInTheDocument()
  })

  it('面板「放弃」按钮清空当前载荷', () => {
    const iframe = renderHtmlPreview()
    enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)
    fireEvent.click(screen.getByRole('button', { name: /放弃/ }))
    expect(screen.queryByText(/2 条标注/)).not.toBeInTheDocument()
  })

  it('onLoad 后 2s 未收到握手 → 显示「标注器未就绪」', () => {
    vi.useFakeTimers()
    try {
      renderHtmlPreview()
      expect(screen.queryByText(/标注器未就绪/)).not.toBeInTheDocument()
      act(() => {
        vi.advanceTimersByTime(2000)
      })
      expect(screen.getByText(/标注器未就绪/)).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it('2s 内收到 nuphus:annotator-ready 握手则不提示未就绪', () => {
    vi.useFakeTimers()
    try {
      const iframe = renderHtmlPreview()
      dispatchMessage({ type: 'nuphus:annotator-ready' }, iframe.contentWindow)
      act(() => {
        vi.advanceTimersByTime(5000)
      })
      expect(screen.queryByText(/标注器未就绪/)).not.toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it('伪造 source 的 ready 握手同样被忽略（仍提示未就绪）', () => {
    vi.useFakeTimers()
    try {
      renderHtmlPreview()
      dispatchMessage({ type: 'nuphus:annotator-ready' }, window)
      act(() => {
        vi.advanceTimersByTime(5000)
      })
      expect(screen.getByText(/标注器未就绪/)).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it('切换预览文件后清空旧标注面板（防串话）', () => {
    const { rerender } = render(<PreviewOverlay path="C:/work/a.html" onClose={() => undefined} />)
    const iframe = document.querySelector('iframe.pv-iframe') as HTMLIFrameElement
    fireEvent.load(iframe)
    fireEvent.click(editToggle())
    dispatchMessage(PAYLOAD, iframe.contentWindow)
    expect(screen.getByText(/2 条标注/)).toBeInTheDocument()

    rerender(<PreviewOverlay path="C:/work/b.html" onClose={() => undefined} />)
    // 切文件同时退出编辑态并清空列表（旧的 .pv-annot-panel 不再是展开态）
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
    // 旧 listener 已拆：接着从旧 iframe 的 contentWindow 发消息也不会复活面板
    dispatchMessage(PAYLOAD, iframe.contentWindow)
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
  })

  it('卸载时移除 message listener', () => {
    const removeSpy = vi.spyOn(window, 'removeEventListener')
    try {
      const { unmount } = render(
        <PreviewOverlay path="C:/work/demo.html" onClose={() => undefined} />,
      )
      const iframe = document.querySelector('iframe.pv-iframe') as HTMLIFrameElement
      fireEvent.load(iframe)
      unmount()
      expect(removeSpy).toHaveBeenCalledWith('message', expect.any(Function))
    } finally {
      removeSpy.mockRestore()
    }
  })

  it('非 HTML 路径不渲染 iframe，也不出现编辑入口', async () => {
    // act 包裹并 flush：txt 走文本读取分支，readFile 的 setState 落在测试体内
    await act(async () => {
      render(<PreviewOverlay path="C:/work/readme.txt" onClose={() => undefined} />)
    })
    expect(document.querySelector('iframe.pv-iframe')).toBeNull()
    expect(document.querySelector('.pv-edit-toggle')).toBeNull()
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
  })
})

// ── P2：发送给 Agent（payload → 既有 quote 引用链路）──
describe('PreviewOverlay 标注「发送给 Agent」（P2）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
  })
  it('发送按钮把序列化引用交给既有发送链路（type=quote，label 含 file+selector+comment）', () => {
    const onSend = vi.fn()
    const iframe = renderHtmlPreview('C:/work/demo.html', onSend)
    enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    fireEvent.click(screen.getByRole('button', { name: /发送给 Agent/ }))

    expect(onSend).toHaveBeenCalledTimes(1)
    const ref = onSend.mock.calls[0][0]
    // 走既有 quote 引用通道（Rust resolve_references 的 quote 分支 = label 原文注入）
    expect(ref.type).toBe('quote')
    expect(ref.label).toContain('【界面标注】demo.html')
    expect(ref.label).toContain('body > div.a:nth-child(2) > p')
    expect(ref.label).toContain('footer:nth-child(3)')
    expect(ref.label).toContain('标题改成蓝色')
    expect(ref.label).toContain('（无）')
    expect(ref.id).toMatch(/^q/)
    // 发送后退出编辑态并清空列表：与 overlay 侧「发送后清空 markers」语义对齐，不重复发送
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
    // 用户可见反馈（禁止静默成功）
    expect(hudUpdate).toHaveBeenCalled()
  })

  it('空 annotations 不允许发送（按钮禁用，点击不进链路）', () => {
    const onSend = vi.fn()
    const iframe = renderHtmlPreview('C:/work/demo.html', onSend)
    enterAnnotating(iframe)
    dispatchMessage(EMPTY_PAYLOAD, iframe.contentWindow)

    expect(screen.getByText(/在页面上点击元素即可标注/)).toBeInTheDocument()
    const sendBtn = screen.getByRole('button', { name: /发送给 Agent/ })
    expect(sendBtn).toBeDisabled()
    fireEvent.click(sendBtn)
    expect(onSend).not.toHaveBeenCalled()
  })

  it('未传发送出口（外部 agent 状态栏内嵌场景）时按钮禁用且 title 说明原因', () => {
    const iframe = renderHtmlPreview()
    enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    const sendBtn = screen.getByRole('button', { name: /发送给 Agent/ })
    expect(sendBtn).toBeDisabled()
    expect(sendBtn).toHaveAttribute('title', expect.stringContaining('不支持发送到对话'))
  })

  it('放弃按钮清空载荷并给出反馈（不静默丢弃）', () => {
    const iframe = renderHtmlPreview('C:/work/demo.html', vi.fn())
    enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    fireEvent.click(screen.getByRole('button', { name: /放弃/ }))
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
    expect(hudUpdate).toHaveBeenCalledWith(expect.stringContaining('已放弃'), 'warning')
  })
})

// ── P3：持久化（父窗口 localStorage）+ 区域框选 region ──
describe('PreviewOverlay 标注持久化与区域框选（P3）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
  })

  it('快照写入父窗口 localStorage（键按预览文件路径）', () => {
    const iframe = renderHtmlPreview('C:/work/demo.html')
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    const raw = localStorage.getItem('nuphus:annotations:C:/work/demo.html')
    expect(raw).not.toBeNull()
    const stored = JSON.parse(raw as string)
    expect(stored).toHaveLength(2)
    expect(stored[0].css_selector).toBe('body > div.a:nth-child(2) > p')
  })

  it('ready 握手后有历史 → 下发 nuphus:annotations-restore 给本 iframe 并提示恢复', () => {
    const key = 'nuphus:annotations:C:/work/demo.html'
    localStorage.setItem(key, JSON.stringify(PAYLOAD.annotations))

    const iframe = renderHtmlPreview('C:/work/demo.html')
    const postSpy = vi.spyOn(iframe.contentWindow as Window, 'postMessage')
    dispatchMessage({ type: 'nuphus:annotator-ready' }, iframe.contentWindow)

    // 恢复指令只发给本 iframe 的 contentWindow（targetOrigin '*'，父→iframe 下行）
    expect(postSpy).toHaveBeenCalledWith(
      { type: 'nuphus:annotations-restore', annotations: PAYLOAD.annotations },
      '*',
    )
    expect(hudUpdate).toHaveBeenCalledWith('已恢复 2 条历史标注', 'done')
    // iframe 恢复后回的同型快照 → 面板标题带「含恢复」角标（列表在编辑态展开）
    dispatchMessage(PAYLOAD, iframe.contentWindow)
    enterAnnotating(iframe)
    expect(panelTitle()).toContain('含恢复 2 条')
    postSpy.mockRestore()
  })

  it('无历史文件 → 不下发 restore 指令（不打扰）', () => {
    const iframe = renderHtmlPreview('C:/work/demo.html')
    const postSpy = vi.spyOn(iframe.contentWindow as Window, 'postMessage')
    dispatchMessage({ type: 'nuphus:annotator-ready' }, iframe.contentWindow)

    for (const call of postSpy.mock.calls) {
      expect((call[0] as { type?: string }).type).not.toBe('nuphus:annotations-restore')
    }
    expect(hudUpdate).not.toHaveBeenCalledWith(expect.stringContaining('已恢复'), 'done')
    postSpy.mockRestore()
  })

  it('发送后清除本地库（刷新不再恢复已处理标注）', () => {
    const key = 'nuphus:annotations:C:/work/demo.html'
    localStorage.setItem(key, JSON.stringify(PAYLOAD.annotations))
    const iframe = renderHtmlPreview('C:/work/demo.html', vi.fn())
    enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    fireEvent.click(screen.getByRole('button', { name: /发送给 Agent/ }))
    expect(localStorage.getItem(key)).toBeNull()
  })

  it('放弃后同样清除本地库', () => {
    const key = 'nuphus:annotations:C:/work/demo.html'
    const iframe = renderHtmlPreview('C:/work/demo.html', vi.fn())
    enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)
    expect(localStorage.getItem(key)).not.toBeNull()

    fireEvent.click(screen.getByRole('button', { name: /放弃/ }))
    expect(localStorage.getItem(key)).toBeNull()
  })

  it('区域标注 payload：region 进面板展示 + 发送时进引用序列化', () => {
    const onSend = vi.fn()
    const iframe = renderHtmlPreview('C:/work/demo.html', onSend)
    enterAnnotating(iframe)
    dispatchMessage(REGION_PAYLOAD, iframe.contentWindow)

    // 面板区域辅助行
    expect(screen.getByText(/区域 280×160 @\(12,24\)/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /发送给 Agent/ }))
    const ref = onSend.mock.calls[0][0]
    // region 只进序列化辅助行，P1/P2 既有字段不受影响
    expect(ref.label).toContain('region: 12,24 280x160')
    expect(ref.label).toContain('body > div.hero:nth-child(2)')
    expect(ref.label).toContain('这一块整体不对')
  })
})

/**
 * ══════════════════════════════════════════════════════════════════
 * 注入脚本的**真实语法校验**（2026-10-08 实机报障后增设）
 *
 * 事故：preview_protocol.rs 的 overlay 脚本里三元表达式 else 分支误写 `'''`，
 * 浏览器直接拒绝执行整个脚本 → 连 postMessage 握手都发不出 → 预览页显示
 * 「标注器未就绪」。而当时**所有测试全绿**：
 *   - cargo test 只对脚本做子串断言（禁网/协议字段），不解析 JS；
 *   - 本文件的前端用例跑的是父窗口 React 逻辑，从不执行这段 Rust 内嵌脚本。
 * 也就是说「语法错误」是既有断言体系的盲区。本组用例把 Rust 源里的脚本常量
 * 真实取出来交给 JS 引擎解析 —— 这是唯一能拦住此类回归的检查。
 *
 * 读取方式沿用 opacity-system.test.ts 的既有惯例：模块名由变量拼出（tsconfig
 * 未启用 @types/node，字面量 'node:fs' 会被 tsc 报 TS2307）。
 * ══════════════════════════════════════════════════════════════════
 */
describe('注入 overlay 脚本：真实 JS 语法校验（防 Rust 内嵌脚本语法错逃逸）', () => {
  /** 从 src-tauri/src/preview_protocol.rs 的原始字符串里抽出 JS 源码。
   *  路径解析：import.meta.url 在本项目的 vitest 配置下不是 file: 协议
   *  （new URL(x, import.meta.url) 直接抛 "The URL must be of scheme file"，
   *  与脚本内容无关）；改用 process.cwd()（= frontend/）逐级上溯到仓库根。
   *  真正可执行的解析防线另有一道：node frontend/tools/sync-overlay-script.mjs
   *  在搬运进 .rs 之前先 new Function 过一遍。 */
  async function loadOverlayScript(): Promise<string> {
    const { readFileSync } = (await import('node:' + 'fs')) as {
      readFileSync: (p: unknown, enc: string) => string
    }
    const { resolve } = (await import('node:' + 'path')) as {
      resolve: (...p: string[]) => string
    }
    // tsconfig 未装 @types/node，直接写 process 会报 TS2580；取全局再断言。
    const cwd = (globalThis as { process?: { cwd(): string } }).process?.cwd()
    if (!cwd) throw new Error('无法确定 vitest 的工作目录')
    const src = readFileSync(resolve(cwd, '..', 'src-tauri', 'src', 'preview_protocol.rs'), 'utf8')
    const m = src.match(/const ANNOTATION_OVERLAY_SCRIPT: &str = r####"([\s\S]*?)"####/)
    if (!m) throw new Error('未能从 preview_protocol.rs 提取 ANNOTATION_OVERLAY_SCRIPT')
    // 剥掉 <script> 包装，得到纯 JS 源码
    return m[1].replace(/^<script>/, '').replace(/<\/script>\s*$/, '')
  }

  it('脚本能被 JS 引擎成功解析（无 SyntaxError）', async () => {
    const js = await loadOverlayScript()
    expect(js.length).toBeGreaterThan(1000)
    // new Function 只解析不执行 —— 执行需要真实 DOM，交给浏览器
    expect(() => new Function(js)).not.toThrow()
  })

  it("不得出现连续三个同类引号（`'''` 词法必错，曾真实发生）", async () => {
    const js = await loadOverlayScript()
    expect(js).not.toContain("'''")
    expect(js).not.toContain('"""')
  })

  it('模板字符串内不得出现未转义反引号导致的提前闭合（反引号必成对）', async () => {
    const js = await loadOverlayScript()
    const ticks = js.split('`').length - 1
    expect(ticks % 2).toBe(0)
  })

  /**
   * P4 缺陷回归：P3 的 hover 高亮写内联 `el.style.outline`，清除时写回空串，
   * 宿主持久残留 `style=""`（实测 outer_html_snippet 带 `style=""`，宿主若用
   * `[style]` 选择器会被误判）。本版改为 overlay 矩形，宿主 DOM 零写入。
   * 只禁「写宿主」的模式：overlay 自己的节点用 classList 驱动视觉是允许的，
   * buildSelector 读 node.classList 生成 selector 也是只读。 */
  it('不得再写宿主元素的 style/class（高亮走 overlay 矩形）', async () => {
    const js = await loadOverlayScript()
    for (const banned of [
      'style.outline',
      '__nuphusOutline',
      'setAttribute(',
      'el.classList',
      'marker.el.style',
      'el.className',
    ]) {
      expect(js).not.toContain(banned)
    }
    expect(js).toContain('highlightrect')
  })

  /** P4 铁律一：iframe 侧零常驻 UI（编辑入口与列表全在父窗口）。 */
    it('不得残留胶囊/提示条/侧边列表/批注框', async () => {
      const js = await loadOverlayScript()
      for (const banned of [
        'capsule',
        'mgr-item',
        'mgr-empty',
        'data-toggle',
        'data-send',
        'openComment',
        'renderList',
        'textarea',
      ]) {
        expect(js).not.toContain(banned)
      }
    })

    /**
     * 配色必须走主题令牌下发，禁止把色值硬编码进脚本（2026-10-08 大王指令）。
     *
     * 事故背景：脚本原先直接写死蓝 `#2f6fdd` / 琥珀 `#f59e0b`，那是从**外部产品**
     * 的设计文档抄来的数值，既不属于 Nuphus 的 token 体系，也会让标注器配色
     * 与界面 accent 脱钩（切简白/深色/tech 后仍是旧色）。现改为父窗口读自己的
     * CSS 变量（--accent/--on-accent/--warning/--accent-rgb）经postMessage 下发，
     *脚本只认 --pv-annot-* 自定义属性。
     *
     * 断言直接读 .rs 内的真实注入内容——所以它同时是「src.js 与 .rs 漂移」的
     * 防线：2026-10-08 sync-overlay-script.mjs 的 --check 曾因锚点漂移谎报
     * "in sync"，只有这条测试会真的失败。
     */
    it('配色走 --pv-annot-* 令牌，脚本内不得硬编码主题色值', async () => {
        const js = await loadOverlayScript()
        // 令牌定义与消费都在
        for (const required of [
          '--pv-annot-accent',
          '--pv-annot-on-accent',
          '--pv-annot-region',
          'var(--pv-annot-accent)',
          'var(--pv-annot-region)',
          'nuphus:annotate-theme',
          'applyTheme',
        ]) {
          expect(js, `脚本应包含 ${required}`).toContain(required)
        }
        // 真正抄自外部产品的色值已彻底移除（这三色曾写死在样式规则里）
        for (const bannedHex of ['#2f6fdd', 'rgba(47, 111, 221', 'rgba(245, 158, 11']) {
          expect(js, `脚本不应再硬编码外部色值 ${bannedHex}`).not.toContain(bannedHex)
        }
        // 兜底默认值允许存在（Nuphus 自己 token 的字面值：--accent/--on-accent/--warning
        // 在 dark 主题的值），但**只允许**出现在 :host 令牌声明块内；样式规则里必须一律
        // 走 var(--pv-annot-*)，否则父窗口下发的主题令牌会失效（硬编码规则不读变量）。
        const styleStart = js.indexOf('<style>')
        const styleEnd = js.indexOf('</style>')
        const styleBody = js.slice(styleStart, styleEnd)
        const hostBlockEnd = styleBody.indexOf('}', styleBody.indexOf('--pv-annot-shadow'))
        expect(hostBlockEnd, ':host 令牌块应闭合').toBeGreaterThan(0)
        // 令牌块之后的规则区不得再出现任何裸 hex / rgba 色值
        const rules = styleBody.slice(hostBlockEnd)
        for (const m of rules.matchAll(/#[0-9a-fA-F]{3,8}\b|rgba\([^)]*\)/g)) {
          expect(m[0], `样式规则里出现裸色值 ${m[0]}（应走 var(--pv-annot-*)令牌）`).toBe('')
        }
      })
    })

// ══════════════════════════════════════════════════════════════════
// P4：父窗口托管 UI（✎ 悬浮入口 + 右侧修改列表 + 上下行指令）
// ══════════════════════════════════════════════════════════════════
describe('PreviewOverlay 标注 UI 重构（P4：父窗口托管）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
  })

  it('默认态：只有 ✎ 入口，列表不显示（iframe 零 UI，页面原生交互全可用）', () => {
    renderHtmlPreview()
    expect(document.querySelector('.pv-edit-toggle')).not.toBeNull()
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
    expect(document.querySelector('.pv-page')).not.toHaveAttribute('data-annotating')
  })

  it('点 ✎ 进编辑态：下发 annotate-mode + 列表展开 + 空态提示', () => {
    const iframe = renderHtmlPreview()
    const postSpy = enterAnnotating(iframe)

    expect(postSpy).toHaveBeenCalledWith({ type: 'nuphus:annotate-mode', on: true }, '*')
    expect(document.querySelector('.pv-page')).toHaveAttribute('data-annotating')
    expect(editToggle()).toHaveAttribute('aria-pressed', 'true')
    // 零标注也给空态，不留空白面板
    expect(screen.getByText(/在页面上点击元素即可标注/)).toBeInTheDocument()
    postSpy.mockRestore()
  })

  it('再点 ✎ 退编辑态：下发 annotate-mode(false) + 列表收起', () => {
    const iframe = renderHtmlPreview()
    const postSpy = enterAnnotating(iframe)
    fireEvent.click(editToggle())

    expect(postSpy).toHaveBeenCalledWith({ type: 'nuphus:annotate-mode', on: false }, '*')
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
    expect(document.querySelector('.pv-page')).not.toHaveAttribute('data-annotating')
    postSpy.mockRestore()
  })

  it('iframe 内 Esc 请求退出编辑态（父窗口收到后收起列表）', () => {
    const iframe = renderHtmlPreview()
    enterAnnotating(iframe)
    expect(document.querySelector('.pv-annot-panel')).not.toBeNull()

    dispatchMessage({ type: 'nuphus:annotate-exit' }, iframe.contentWindow)
    expect(document.querySelector('.pv-annot-panel')).toBeNull()
    expect(editToggle()).toHaveAttribute('aria-pressed', 'false')
  })

  it('伪造 source 的 annotate-exit 无效（仍处编辑态）', () => {
    const iframe = renderHtmlPreview()
    enterAnnotating(iframe)

    dispatchMessage({ type: 'nuphus:annotate-exit' }, window)
    dispatchMessage({ type: 'nuphus:annotate-exit' }, {})
    expect(document.querySelector('.pv-annot-panel')).not.toBeNull()
  })

  it('批注输入下发 annotate-comment，回流快照写进 localStorage', () => {
    const iframe = renderHtmlPreview()
    const postSpy = enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    fireEvent.change(commentInput(2), { target: { value: '这里也要改' } })
    expect(postSpy).toHaveBeenCalledWith(
      { type: 'nuphus:annotate-comment', index: 1, comment: '这里也要改' },
      '*',
    )

    // iframe 侧更新 markers 后广播的同型快照 → 父窗口落库（持久化路径未新增）
    dispatchMessage(
      {
        ...PAYLOAD,
        annotations: [PAYLOAD.annotations[0], { ...PAYLOAD.annotations[1], comment: '这里也要改' }],
      },
      iframe.contentWindow,
    )
    expect(commentInput(2).value).toBe('这里也要改')
    const stored = JSON.parse(
      localStorage.getItem('nuphus:annotations:C:/work/demo.html') as string,
    ) as { comment: string }[]
    expect(stored[1].comment).toBe('这里也要改')
    postSpy.mockRestore()
  })

  it('单条删除下发 annotate-remove；回流后序号重排且 key 不串条', () => {
    const iframe = renderHtmlPreview()
    const postSpy = enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    // 删第 1 条：iframe 侧摘 marker 后序号重排，回流快照只剩 footer
    fireEvent.click(screen.getByRole('button', { name: '删除这条标注 1' }))
    expect(postSpy).toHaveBeenCalledWith({ type: 'nuphus:annotate-remove', index: 0 }, '*')

    dispatchMessage({ ...PAYLOAD, annotations: [PAYLOAD.annotations[1]] }, iframe.contentWindow)
    // 序号按新位置重排（1），批注仍属于 footer 那一行（key 用 selector 而非 index）
    const indexBadge = document.querySelector('.pv-annot-index')
    expect(indexBadge?.textContent).toBe('1')
    expect(commentInput(1).value).toBe('')
    expect(screen.getByText('footer:nth-child(3)')).toBeInTheDocument()
    expect(screen.queryByText('body > div.a:nth-child(2) > p')).not.toBeInTheDocument()
    postSpy.mockRestore()
  })

  it('定位按钮下发 annotate-focus（selector + 单调 seq）', () => {
    const iframe = renderHtmlPreview()
    const postSpy = enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    fireEvent.click(screen.getByRole('button', { name: '在页面中定位这条标注 1' }))
    fireEvent.click(screen.getByRole('button', { name: '在页面中定位这条标注 1' }))

    const focusCalls = postSpy.mock.calls.filter(
      c => (c[0] as { type?: string }).type === 'nuphus:annotate-focus',
    )
    expect(focusCalls).toHaveLength(2)
    expect(focusCalls[0][0]).toMatchObject({
      type: 'nuphus:annotate-focus',
      selector: 'body > div.a:nth-child(2) > p',
    })
    // seq 单调递增：同一 selector 连续定位时 iframe 侧动画也能重启
    expect((focusCalls[1][0] as { seq: number }).seq).toBeGreaterThan(
      (focusCalls[0][0] as { seq: number }).seq,
    )
    postSpy.mockRestore()
  })

  it('收起态显示待处理角标数量（信息不丢），发送后角标消失', () => {
    const iframe = renderHtmlPreview('C:/work/demo.html', vi.fn())
    enterAnnotating(iframe)
    dispatchMessage(PAYLOAD, iframe.contentWindow)
    fireEvent.click(editToggle())

    const badge = document.querySelector('.pv-edit-badge')
    expect(badge?.textContent).toBe('2')

    fireEvent.click(editToggle())
    fireEvent.click(screen.getByRole('button', { name: /发送给 Agent/ }))
    expect(document.querySelector('.pv-edit-badge')).toBeNull()
  })

  it('握手后补发当前编辑态（iframe 载入前点的 ✎ 不丢）', () => {
    const iframe = renderHtmlPreview()
    // 先点 ✎（此时 iframe 还没握手，指令下发到未初始化的 contentWindow）
    fireEvent.click(editToggle())
    const postSpy = vi.spyOn(iframe.contentWindow as Window, 'postMessage')

    dispatchMessage({ type: 'nuphus:annotator-ready' }, iframe.contentWindow)
    expect(postSpy).toHaveBeenCalledWith({ type: 'nuphus:annotate-mode', on: true }, '*')
    postSpy.mockRestore()
  })

  it('恢复历史标注后进入编辑态，列表带「含恢复」角标', () => {
    localStorage.setItem(
      'nuphus:annotations:C:/work/demo.html',
      JSON.stringify(PAYLOAD.annotations),
    )
    const iframe = renderHtmlPreview('C:/work/demo.html', vi.fn())
    dispatchMessage({ type: 'nuphus:annotator-ready' }, iframe.contentWindow)
    dispatchMessage(PAYLOAD, iframe.contentWindow)

    enterAnnotating(iframe)
        expect(panelTitle()).toContain('2 条标注')
        expect(panelTitle()).toContain('含恢复 2 条')
        expect(commentInput(1).value).toBe('标题改成蓝色')
      })

      /**
       * 回归（2026-10-08 大王报障）：点「放弃」后再进编辑态，旧标注全部回来。
       *
       * 根因：数据真正持有方是 iframe 内的 markers，而「放弃」只清了父窗口的
       * localStorage + React state —— iframe 数据没清，重进编辑态 renderOverlays()
       * 把旧标记全画回来。修法是新增下行指令 `nuphus:annotate-clear-all`。
       * 本组断言钉住：**放弃与发送都必须下发 clear-all**，且clear-all 先于 mode(false)。
       */
      it('放弃：下发 annotate-clear-all 清 iframe 数据，且先于退出编辑态', () => {
        const iframe = renderHtmlPreview('C:/work/demo.html', vi.fn())
        const postSpy = enterAnnotating(iframe)
        dispatchMessage(PAYLOAD, iframe.contentWindow)

        postSpy.mockClear()
        fireEvent.click(screen.getByRole('button', { name: '放弃' }))

        const types = postSpy.mock.calls.map(c => (c[0] as { type?: string }).type)
        expect(types).toContain('nuphus:annotate-clear-all')
        expect(types).toContain('nuphus:annotate-mode')
        // 顺序铁律：先清数据再退编辑态（反了的话 mode(false) 只清视觉，数据留着会回来）
        expect(types.indexOf('nuphus:annotate-clear-all')).toBeLessThan(
          types.lastIndexOf('nuphus:annotate-mode'),
        )
        // 本地库同步清空
        expect(localStorage.getItem('nuphus:annotations:C:/work/demo.html')).toBeNull()
        postSpy.mockRestore()
      })

      it('发送：同样下发 annotate-clear-all（与放弃同源，不能只清父窗口）', () => {
        const onSend = vi.fn()
        const iframe = renderHtmlPreview('C:/work/demo.html', onSend)
        const postSpy = enterAnnotating(iframe)
        dispatchMessage(PAYLOAD, iframe.contentWindow)

        postSpy.mockClear()
        fireEvent.click(screen.getByRole('button', { name: /发送给 Agent/ }))

        const types = postSpy.mock.calls.map(c => (c[0] as { type?: string }).type)
        expect(types).toContain('nuphus:annotate-clear-all')
        expect(onSend).toHaveBeenCalledTimes(1)
        expect(localStorage.getItem('nuphus:annotations:C:/work/demo.html')).toBeNull()
        postSpy.mockRestore()
      })
    })
