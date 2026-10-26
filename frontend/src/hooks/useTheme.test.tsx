/**
 * useTheme —— 皮肤背景不透明度默认值（#117 回归）
 *
 * 背景：#117 报「系统预设主题下背景图默认不透明度为 35%，应为 0」。
 * tokens.css 里的默认值已改为 0（「没显式设置过」= 不显示），而 0.35 原本承担的
 * 「有图、但用户没拖过滑块」这一可见性语义，改由 withSkinOpacityDefault 在应用覆盖时注入。
 *
 * 本文件钉住四条边界，防止日后有人把 tokens 默认值直接改回 0.35 了事（那会让
 * 系统预设态下历史遗留的 nuphus_skin_bg 残留路径重新以 35% 显示出来）：
 *  ① 系统预设态（无 customTheme）→ 不注入 → 回落 tokens 的 0；
 *  ② 有皮肤图、用户从未设过 → 注入 0.35（保证「选了图看得见效果」）；
 *  ③ 无皮肤图 → 不注入（无图时不需要可见值）；
 *  ④ 有图、用户显式设过 → 原样用用户值，不被默认值覆盖。
 */
import { render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { SKIN_OPACITY_KEY } from './customTheme'
import { ThemeProvider } from './useTheme'

/** 读 documentElement 上该 token 的内联覆盖值（'' = 无覆盖 → 回落 tokens.css 默认） */
const readSkinOpacity = () => document.documentElement.style.getPropertyValue(SKIN_OPACITY_KEY)

/** 预置一个自定义主题并按 useTheme 的存储契约激活它 */
function seedActiveTheme(overrides: Record<string, string>, skin?: string) {
  localStorage.setItem(
    'nuphus_custom_themes',
    JSON.stringify([{ id: 'ct-1', name: 'T', base: 'dark', overrides, ...(skin ? { skin } : {}) }]),
  )
  localStorage.setItem('nuphus_custom_active', 'ct-1')
}

describe('useTheme —— 皮肤背景不透明度默认值（#117）', () => {
  beforeEach(() => {
    localStorage.clear()
    document.documentElement.removeAttribute('style')
    document.documentElement.removeAttribute('data-theme')
  })

  afterEach(() => {
    document.documentElement.removeAttribute('style')
  })

  it('系统预设态：不注入覆盖，交由 tokens.css 的 0 决定（历史残留路径不会被点亮）', () => {
    render(
      <ThemeProvider>
        <div />
      </ThemeProvider>,
    )
    expect(readSkinOpacity()).toBe('')
  })

  it('有皮肤图、用户从未设过不透明度：注入 0.35', () => {
    seedActiveTheme({}, 'C:/img/a.png')
    render(
      <ThemeProvider>
        <div />
      </ThemeProvider>,
    )
    expect(readSkinOpacity()).toBe('0.35')
  })

  it('无皮肤图：不注入', () => {
    seedActiveTheme({})
    render(
      <ThemeProvider>
        <div />
      </ThemeProvider>,
    )
    expect(readSkinOpacity()).toBe('')
  })

  it('用户显式设过：原样使用用户值，不被默认值覆盖', () => {
    seedActiveTheme({ [SKIN_OPACITY_KEY]: '0.8' }, 'C:/img/a.png')
    render(
      <ThemeProvider>
        <div />
      </ThemeProvider>,
    )
    expect(readSkinOpacity()).toBe('0.8')
  })
})
