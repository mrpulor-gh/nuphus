/**
 * 移动端 i18n 轻量接入：
 * - 复用桌面端 zh/en 字典（src/locales），不建第二套翻译资产
 * - 语言跟随 localStorage 'nuphus_language'（与桌面端 LangProvider 同键，
 *   同一浏览器/设备上桌面设置的语言自动生效；未设置默认 zh）
 * - t(key) 纯函数，无 React Context——移动端 PWA 无语言切换 UI，
 *   静态读取足够；新增键必须中英双语同步（locales.test.ts 强制）
 */
import zh from '../locales/zh'
import en from '../locales/en'

const packs: Record<string, Record<string, string>> = { zh, en }

export function mobileLang(): string {
  const stored = localStorage.getItem('nuphus_language')
  return stored && stored in packs ? stored : 'zh'
}

export function t(key: string): string {
  return packs[mobileLang()][key] ?? zh[key] ?? key
}
