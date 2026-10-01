/**
 * 英文语种文件的「中文残留」契约测试。
 *
 * 回归背景（大王 2026-10-01 实测「切英语后中文切换文字是乱码」）：`en.ts` 早期被
 * GBK 误读存档，`'lang.zh': '中文'` 变成 `'涓<U+E15F>枃'`（字节已丢失，含私用区
 * 字符 U+E15F，不可反向还原）。中文模式走 zh.ts 看不出，切英语后该按钮直接渲染乱码。
 *
 * 两条断言：
 * ① 英文值里**不得出现私用区字符**（U+E000–U+F8FF）——那是「GBK 误读 + 字节丢失」
 *   的确定性残渣，正常文本永远不该有；
 * ② 英文值里含中日韩汉字的值必须**只有** `lang.zh`（语言自称，合理），其余一概不允许
 *    ——防止新的漏翻/误翻以「中文」形式混进英文界面。
 *
 * 读取方式同 opacity-system.test.ts：`?raw` 在本项目 Vite 下对 .ts 返回空串，
 * 故用运行时动态引入 node:fs（模块名拼接，绕过未启用 @types/node 的 tsc 静态解析）。
 */

import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

/** 取 `'key': 'value',` 形式的值（词条文件里绝大多数条目都是这一行式） */
function entries(src: string): { key: string; value: string }[] {
  const out: { key: string; value: string }[] = []
  for (const line of src.split(/\r?\n/)) {
    const m = /^\s*'([^']+)':\s*'([^']*)',\s*$/.exec(line)
    if (m) out.push({ key: m[1], value: m[2] })
  }
  return out
}

const en = entries(read('./en.ts'))
const enRaw = read('./en.ts')

describe('en.ts 不得残留 GBK 乱码 / 中文漏翻', () => {
  it('读到了词条（防静默通过）', () => {
    expect(en.length).toBeGreaterThan(300)
    expect(en.some(e => e.key === 'lang.zh')).toBe(true)
  })

  it('① 任何英文值都不得含私用区字符（GBK 误读 + 字节丢失的确定性残渣）', () => {
    const dirty = en.filter(e => /[\ue000-\uf8ff]/.test(e.value))
    expect(dirty.map(e => `${e.key}=${JSON.stringify(e.value)}`)).toEqual([])
  })

  it('② 含汉字的英文值只有 lang.zh（语言自称），且就是「中文」', () => {
    const cjk = en.filter(e => /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/.test(e.value))
    expect(cjk.map(e => e.key)).toEqual(['lang.zh'])
    expect(cjk[0]?.value).toBe('中文')
  })

  it('③ ↑↓ / ° / § 等常用符号没有被啃成乱码（历史三类残码）', () => {
    const byKey = new Map(en.map(e => [e.key, e.value]))
    // 残码特征字符：鈫 戔 啌 掳 搂 鈻 鈼 銆 鈥 冫
    const marks = /[鈫戔啌掳搂鈻鈼銆鈥冫]/
    const dirty = [...byKey].filter(([, v]) => marks.test(v)).map(([k]) => k)
    expect(dirty).toEqual([])
    // 正向钉住三处语义（值必须真的是这些符号，不是被替换成别的）
    expect(byKey.get('common.hint.upDown')).toBe('↑↓ select')
    expect(byKey.get('security.hintUpDown')).toBe('↑↓ select')
    expect(byKey.get('refine.hintSelect')).toBe('↑↓ select')
    expect(byKey.get('tools.rotate180')).toBe('180°')
    expect(byKey.get('tools.ability.pdfRotateDesc')).toBe('Rotate all pages by 90° / 180° / 270°')
    // 多行词条（值在下一行）：直接查原文
    expect(enRaw).toContain('(design doc §8)')
    expect(enRaw).not.toContain('(design doc 搂8)')
  })
})
