/**
 * Models 弹窗「模型名后收藏星标」契约（源码断言）。
 *
 * 为什么按源码断言而不是把弹窗跑起来：ChatPanel 的 models 弹窗依赖整套
 * savedConfigs / IPC bridge / locale provider，真实渲染成本与收益不成比例；
 * 本组要钉的是**结构契约**——这几条在 jsdom 里都会渲染成同一棵空树：
 *
 * ① 星标紧跟模型名渲染（`.model-provider-model-name` 内、GO 徽章之后）；
 * ② 「没有收藏就不渲染」——不是置灰占位（未收藏是默认态，不需要空图标）；
 * ③ 星标与置顶排序共用同一份 `favorites`（provider 限定，不各读一次）；
 * ④ i18n 词条 zh / en 双语齐备。
 */

import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

const panelSrc = read('./ChatPanel.tsx')
const zh = read('../../locales/zh.ts')
const en = read('../../locales/en.ts')
const componentsCss = read('../../styles/components.css')

/** 取 `<span className="model-provider-model-name">` 到该名称 span 收尾之间的片段 */
function modelNameSpan(): string {
  const start = panelSrc.indexOf('<span className="model-provider-model-name">')
  if (start < 0) throw new Error('model name span not found')
  const end = panelSrc.indexOf('</span>', panelSrc.indexOf('model-popup-fav', start))
  if (end < 0) throw new Error('model name span end not found')
  return panelSrc.slice(start, end)
}

describe('源码确实读进来了（防"读空文件导致断言静默通过"）', () => {
  it('锚点齐备', () => {
    expect(panelSrc.length).toBeGreaterThan(5000)
    expect(panelSrc).toContain('model-popup-fav')
    expect(zh).toContain("'modelManager.favorited'")
    expect(en).toContain("'modelManager.favorited'")
  })
})

describe('弹窗模型名后的收藏星标', () => {
  it('星标在模型名 span 内、GO 徽章之后（紧跟着名称）', () => {
    const nameSpan = modelNameSpan()
    expect(nameSpan.indexOf('model-popup-fav')).toBeGreaterThan(nameSpan.indexOf('model-go-badge'))
  })

  it('没有收藏就不渲染（条件渲染，不是占位/置灰）', () => {
    const nameSpan = modelNameSpan()
    expect(nameSpan).toContain('{favorites?.includes(model.id) && (')
    // 不允许出现「常驻 + is-on 类名切换」这种占位写法
    expect(nameSpan).not.toMatch(/model-popup-fav[^>]*is-on/)
  })

  it('星标与置顶排序共用同一份 favorites（provider 限定，取一次）', () => {
    const mapStart = panelSrc.indexOf('savedConfigs.map(cfg => {')
    expect(mapStart).toBeGreaterThan(0)
    const block = panelSrc.slice(mapStart, panelSrc.indexOf('model-popup-fav', mapStart))
    expect(block).toContain('const favorites = getProviderFavorites(localStorage, cfg.provider)')
    // 排序消费的是同一个变量，而不是再调一次 getProviderFavorites
    expect(block).toMatch(/orderProviderModels\([\s\S]*?\bfavorites\s*,?\s*\)/)
    expect(block).not.toContain(
      'getProviderFavorites(localStorage, cfg.provider),\n                          )',
    )
  })

  it('CSS 提供 .model-popup-fav（accent 小星，不占横向布局）', () => {
    expect(componentsCss).toContain('.model-popup-fav')
  })

  it('GO 徽标与名称之间必须有间隔（防止 deepseek-flashGO 贴字）', () => {
    const go = componentsCss.slice(
      componentsCss.indexOf('.model-go-badge {'),
      componentsCss.indexOf('}', componentsCss.indexOf('.model-go-badge {')),
    )
    expect(go).toMatch(/margin-left:\s*5px/)
  })
})
