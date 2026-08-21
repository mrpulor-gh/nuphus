import {
  useState,
  useEffect,
  createContext,
  useContext,
  createElement,
  type ReactNode,
} from 'react'
import zh from './zh'
import en from './en'

type LangPack = Record<string, string>

const packs: Record<string, LangPack> = { zh, en }

interface LangContextType {
  lang: string
  t: (key: string, ...args: string[]) => string
  setLang: (id: string) => void
}

const LangContext = createContext<LangContextType | null>(null)

export function LangProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState(() => {
    return localStorage.getItem('nuphus_language') || 'zh'
  })

  useEffect(() => {
    import('../main-window/lib/api')
      .then(m => {
        m.getLanguage()
          .then(backend => {
            if (backend) {
              const id = backend.startsWith('zh') ? 'zh' : 'en'
              setLangState(id)
              localStorage.setItem('nuphus_language', id)
            }
          })
          .catch(() => {})
      })
      .catch(() => {})
  }, [])

  const setLang = (id: string) => {
    setLangState(id)
    localStorage.setItem('nuphus_language', id)
  }

  const t = (key: string, ...args: string[]): string => {
    const pack = packs[lang] || zh
    let text = pack[key]
    if (text === undefined) {
      text = zh[key] || key
    }
    args.forEach((arg, i) => {
      text = text.replace(`{${i}}`, arg)
    })
    return text
  }

  return createElement(LangContext.Provider, { value: { lang, t, setLang } }, children)
}

export function useLanguage() {
  const ctx = useContext(LangContext)
  if (!ctx) {
    const t = (key: string, ...args: string[]): string => {
      let text = zh[key] || key
      args.forEach((arg, i) => {
        text = text.replace(`{${i}}`, arg)
      })
      return text
    }
    return { lang: 'zh', t, setLang: () => {} }
  }
  return ctx
}
