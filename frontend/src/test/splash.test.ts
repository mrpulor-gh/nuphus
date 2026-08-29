import { describe, expect, it } from 'vitest'
// ?raw 静态导入：构建期内联文件内容，无 node:fs 依赖（jsdom 环境友好），
// 同时让 vite 在 public 资源缺失时直接构建报错——测试环境本身也是一道防线
import splashHtml from '../../public/splash.html?raw'
import splashJs from '../../public/splash.js?raw'

/**
 * Splash「后台下载」按钮防回归钉。
 *
 * 该按钮曾三次被报告"全模型已就绪仍显示"，历次根因不同：后端跳过分支
 * 误发 pct（286ecb2 修）、前端无守卫、用户运行陈旧二进制。
 * 此测试把前端三条防线锁死，任何一条被改坏立即红灯：
 * 1. HTML 默认态：skipWrap/bar 自带 hidden —— 缓存启动零事件 = 纯文字；
 * 2. 亮出按钮必须通过 lastPct<100 门 —— 完成/非下载态绝不出现；
 * 3. ensureSkipTimer 只允许在数字 pct 分支内调用 —— 纯文字阶段不计时。
 * 另有品牌钉：splash 遵循黑白反色视觉，禁旧蓝紫/靛蓝调色板回潮。
 */
describe('splash 后台下载按钮防回归', () => {
  it('防线1：skipWrap 与 bar 默认 hidden（缓存启动零事件=纯文字）', () => {
    expect(splashHtml).toMatch(/<div[^>]*id="bar"[^>]*\shidden/)
    expect(splashHtml).toMatch(/<div[^>]*id="skipWrap"[^>]*\shidden/)
  })

  it('防线2：亮出按钮必须满足 lastPct>=0 且 <100（完成/非下载不出现）', () => {
    expect(splashJs).toMatch(/lastPct\s*>=\s*0\s*&&\s*lastPct\s*<\s*100/)
  })

  it('防线3：ensureSkipTimer 只在数字 pct 分支内调用（共两处：定义+调用）', () => {
    const occ = [...splashJs.matchAll(/ensureSkipTimer\(\)/g)].map(m => m.index ?? -1)
    // 定义 1 处 + 分支内调用 1 处；多出的调用点 = 绕过守卫的回归
    expect(occ.length).toBe(2)
    const trigger = splashJs.indexOf("typeof d.pct === 'number'")
    expect(trigger).toBeGreaterThan(-1)
    expect(occ[0]).toBeLessThan(trigger)
    expect(occ[1]).toBeGreaterThan(trigger)
  })
})

describe('splash 品牌视觉防回归（黑白反色）', () => {
  // 2026-08-28 品牌重构前的旧调色板：蓝紫渐变进度条 + 靛蓝按钮 + 蓝调灰阶。
  // 这些色值一旦回潮 = 彩色样式混入黑白反色视觉，直接红灯。
  const legacyPalette = [
    '#5b5be6', // 旧进度条渐变起
    '#8b8bf5', // 旧进度条渐变止
    '#6366f1', // 旧按钮靛蓝文字
    '#d3d3e0', // 旧按钮描边
    '#f0f0ff', // 旧按钮 hover 底
    '#9a9ab0', // 旧说明文字蓝灰
    '#5a5a6e', // 旧 hint 蓝灰
    '#e9e9ef', // 旧进度条轨道
    '#1a1a2e', // 旧 logo 蓝调墨
  ]

  it('品牌钉1：不得出现旧蓝紫/靛蓝调色板', () => {
    const html = splashHtml.toLowerCase()
    for (const hex of legacyPalette) {
      expect(html).not.toContain(hex)
    }
  })

  it('品牌钉2：进度条填充与按钮反色必须使用墨色 #111111', () => {
    // 样式经 :root 变量单一数据源管理：钉变量定义（锁死色值）+ 变量引用（锁死使用处）。
    // 有人改 --ink/--paper 色值 → 红灯；有人绕开变量直填杂色 → 红灯。
    expect(splashHtml).toMatch(/--ink:\s*#111111/)
    expect(splashHtml).toMatch(/--paper:\s*#ffffff/)
    expect(splashHtml).toMatch(/\.bar-fill\s*\{[^}]*background:\s*var\(--ink\)/)
    expect(splashHtml).toMatch(
      /\.skip-btn:hover\s*\{[^}]*background:\s*var\(--ink\)[^}]*color:\s*var\(--paper\)/,
    )
  })
})
