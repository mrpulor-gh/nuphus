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
