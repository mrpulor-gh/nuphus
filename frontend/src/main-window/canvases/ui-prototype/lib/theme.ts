import { createContext, useContext } from 'react'
import { DEFAULT_THEME, FONTS, FontKey, LANG_FONT, Theme } from './tokens'
import type { Lang } from './i18n'

/** The document theme, read by parts that render differently under it
 *  (emphasized type, motion scheme). Shape goes through the tokens helpers. */
export const ThemeContext = createContext<Theme>(DEFAULT_THEME)
export const useTheme = () => useContext(ThemeContext)

const loaded = new Set<FontKey>()

/** Fetch a Google font the first time it is chosen; the built-in faces need nothing.
 *  `onReady` fires once the face is usable, so measured widths can be refreshed. */
export function ensureFontLoaded(key: FontKey, onReady?: () => void) {
  const f = FONTS.find(x => x.key === key)
  if (typeof document === 'undefined') return
  const ready = () => document.fonts?.ready.then(() => onReady?.())
  if (!f?.google || loaded.has(key)) {
    ready()
    return
  }
  loaded.add(key)
  const link = document.createElement('link')
  link.rel = 'stylesheet'
  link.href = `https://fonts.googleapis.com/css2?family=${f.google}&display=swap`
  link.onload = () => {
    /* the stylesheet is in, but the face itself downloads on first use: load it explicitly */
    const face = f.family.split(',')[0].trim().replace(/^'|'$/g, '')
    Promise.all([400, 500, 600, 700].map(w => document.fonts.load(`${w} 16px "${face}"`))).then(
      ready,
      ready,
    )
  }
  document.head.appendChild(link)
}

const loadedLangs = new Set<Lang>()

/** Fetch the Noto Sans face for a language the first time it is used. */
export function ensureLangFontLoaded(lang: Lang, onReady?: () => void) {
  const f = LANG_FONT[lang]
  if (typeof document === 'undefined') return
  const ready = () => document.fonts?.ready.then(() => onReady?.())
  if (!f || loadedLangs.has(lang)) {
    ready()
    return
  }
  loadedLangs.add(lang)
  const link = document.createElement('link')
  link.rel = 'stylesheet'
  link.href = `https://fonts.googleapis.com/css2?family=${f.google}&display=swap`
  link.onload = () => {
    const face = f.family.replace(/^'|'$/g, '')
    Promise.all([400, 500, 600, 700].map(w => document.fonts.load(`${w} 16px "${face}"`))).then(
      ready,
      ready,
    )
  }
  document.head.appendChild(link)
}
