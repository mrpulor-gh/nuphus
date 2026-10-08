import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { PreviewOverlay } from './PreviewOverlay'

// 应用内浏览器的入口已从 PreviewOverlay 整体移除（大王定案：远程浏览与本地
// 文件预览分离）。2026-10-08 壳+内容架构落地后，入口进一步收敛为「header 按钮
// 直达独立窗口」，主窗口不再挂任何控制条。本文件钉住「移除干净」：
//   · DOM 层：预览覆盖层不出现任何浏览器控制条（.bp-root / .pv-browser-slot）
//   · 交互层：工具栏没有浏览器入口按钮
//   · 源码层：PreviewOverlay.tsx / preview-overlay.css 不留 browser 死代码
//   · 迁移层：ChatPanel 无 slot / 无 BrowserPanel；header 按钮改为开/唤出窗口，
//     壳页面（frontend/src/browser-frame/）与 Rust 命令契约就位
// BrowserPanel 已随本次重构删除（控制条搬进独立窗口的壳页面）；
// 入口行为由 test/chat-header-browser.test.tsx 覆盖，
// 壳页面契约由 src/browser-frame/BrowserFrame.test.tsx 覆盖。

/**
 * 运行时加载 Node 的 fs。模块名由变量拼出（`'node:' + 'fs'`）：本项目 tsconfig
 * 未启用 @types/node，写成字面量会被 tsc 报 TS2307；运行时（vitest 跑在 Node 上）
 * 照常可用。沿用 styles/opacity-system.test.ts 的既有做法。
 */
const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

/** 读取本目录下的源码 / 样式文本（源码级断言用） */
const readSource = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

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

describe('PreviewOverlay 浏览器入口已完整移除', () => {
  it('渲染后 DOM 无浏览器控制条（.bp-root / .pv-browser-slot 都不出现）', () => {
    render(<PreviewOverlay path="C:/work/demo.md" onClose={() => undefined} />)

    expect(document.querySelector('.bp-root')).toBeNull()
    expect(document.querySelector('.bp-tabs')).toBeNull()
    expect(document.querySelector('.pv-browser-slot')).toBeNull()
  })

  it('工具栏不再有浏览器入口按钮', () => {
    render(<PreviewOverlay path="C:/work/demo.md" onClose={() => undefined} />)

    expect(screen.queryByRole('button', { name: /浏览器|Browser/ })).toBeNull()
    // 原有工具栏其余按钮不受影响（关闭 / 定位 / 系统打开仍在）
    expect(screen.getByRole('button', { name: /系统打开|Open/ })).toBeTruthy()
  })

  it('PreviewOverlay.tsx 源码无 browser 死代码', () => {
    const source = readSource('./PreviewOverlay.tsx')

    expect(source).not.toContain('BrowserPanel')
    expect(source).not.toContain('pv-browser-slot')
    expect(source).not.toContain('browserOpen_')
    expect(source).not.toContain('browserLabel')
    // 浏览器按钮专用图标（Globe）随入口一并移除，不在 import 列表里残留
    expect(source).not.toMatch(/^\s*Globe,\s*$/m)
    expect(source).not.toContain("'browser.open'")
  })

  it('preview-overlay.css 无 .pv-browser-slot 死样式', () => {
    expect(readSource('./preview-overlay.css')).not.toContain('pv-browser-slot')
  })

  it('主窗口不再挂浏览器控制条：ChatPanel 无 slot / 无 BrowserPanel', () => {
    const source = readSource('./ChatPanel.tsx')

    // 常驻挂载槽与面板整体移除（含 state / import）
    expect(source).not.toContain('chat-browser-slot')
    expect(source).not.toContain('BrowserPanel')
    expect(source).not.toContain('showBrowser')
    // header 入口仍在，但行为从「切显隐」变成「开/唤出独立窗口」
    expect(source).toContain('chat-header-browser-btn')
    expect(source).toContain('openBrowserWindow')
    expect(source).toContain('browserOpen')
    // 样式表里的挂载槽样式一并清掉
    expect(readSource('../../styles/chat-messages.css')).not.toContain('chat-browser-slot')
  })

  it('壳页面与 Rust 命令契约就位（44px 控制器搬进独立窗口）', () => {
    const frame = readSource('../../browser-frame/BrowserFrame.tsx')

    // 壳页面驱动 content webview：命令全部以窗口 label 为参数
    expect(frame).toContain('browserNavigate')
    expect(frame).toContain('browserGoBack')
    expect(frame).toContain('browserListWindows')
    expect(frame).toContain('browserRecordAction')
    // 自身 label 来自 tauri 注入的 currentWindow metadata（非 -frame 后缀手术）
    expect(frame).toContain('getCurrentWindow()')
    // vite 多入口就位，否则壳页打不进 dist
    expect(readSource('../../../vite.config.ts')).toContain('browser-frame.html')
  })
})
