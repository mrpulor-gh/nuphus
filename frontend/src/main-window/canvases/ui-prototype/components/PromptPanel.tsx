import { useEffect, useMemo, useState } from 'react'
import { buildPrompt } from '../lib/prompt'
import { Doc, Palette, Platform, defaultPlatformOf } from '../lib/tokens'
import { Icon } from './M3Node'
import { Field, IconBtn, Segmented } from './ui'
import { t, useLang } from '../lib/i18n'

export function PromptPanel({
  doc,
  widths,
  palette: p,
  onDoc,
}: {
  doc: Doc
  widths: Record<string, number>
  palette: Palette
  onDoc: (patch: Partial<Doc>) => void
}) {
  const lang = useLang()
  const generated = useMemo(() => buildPrompt(doc, widths, undefined, lang), [doc, widths, lang])
  const edited = doc.promptEdit !== undefined
  const text = edited ? doc.promptEdit! : generated
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) return
    const t = setTimeout(() => setCopied(false), 1400)
    return () => clearTimeout(t)
  }, [copied])

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
    } catch {}
  }

  const projectButton = (icon: string, label: string, onClick: () => void) => (
    <button
      onClick={onClick}
      className="m3-press"
      style={{
        flex: 1,
        height: 42,
        borderRadius: 21,
        border: 'none',
        background: p.secondaryContainer,
        color: p.onSecondaryContainer,
        fontSize: 13,
        fontWeight: 600,
        cursor: 'pointer',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 7,
      }}
    >
      <Icon name={icon} size={19} />
      {label}
    </button>
  )

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 12, gap: 10 }}>
      <Field
        value={doc.title}
        onChange={title => onDoc({ title })}
        placeholder={t('appName', lang)}
        p={p}
        icon="smartphone"
      />
      <Field
        value={doc.brief}
        onChange={brief => onDoc({ brief })}
        placeholder={t('brief', lang)}
        p={p}
        icon="lightbulb"
        multiline
        rows={3}
      />
      <Segmented<Platform>
        options={[
          { key: 'android', icon: 'android', label: 'Android', title: t('targetAndroid', lang) },
          { key: 'web', icon: 'language', label: 'Web', title: t('targetWeb', lang) },
        ]}
        value={doc.platform ?? defaultPlatformOf(doc.frames, doc.frame)}
        onChange={platform => onDoc({ platform })}
        p={p}
        height={40}
      />
      <div style={{ position: 'relative', flex: 1, minHeight: 0, display: 'flex' }}>
        <textarea
          className="no-scrollbar"
          value={text}
          onChange={e => onDoc({ promptEdit: e.target.value })}
          spellCheck={false}
          aria-label={t('prompt', lang)}
          style={{
            flex: 1,
            minHeight: 0,
            width: '100%',
            borderRadius: 18,
            border: 'none',
            background: p.surfaceContainerLow,
            padding: edited ? '14px 14px 48px' : 14,
            fontSize: 13,
            lineHeight: 1.75,
            color: p.onSurface,
            fontFamily: 'inherit',
            resize: 'none',
            outline: 'none',
            boxSizing: 'border-box',
          }}
        />
        {edited && (
          <div style={{ position: 'absolute', right: 8, bottom: 8 }}>
            <IconBtn
              icon="undo"
              p={p}
              size={32}
              onClick={() => onDoc({ promptEdit: undefined })}
              title={t('promptReset', lang)}
            />
          </div>
        )}
      </div>
      <button
        onClick={copy}
        className="m3-press"
        style={{
          height: 48,
          borderRadius: 24,
          border: 'none',
          background: copied ? p.tertiaryContainer : p.primary,
          color: copied ? p.onTertiaryContainer : p.onPrimary,
          fontSize: 14,
          fontWeight: 600,
          cursor: 'pointer',
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 8,
          transition: 'background 160ms, color 160ms',
        }}
      >
        <Icon name={copied ? 'check' : 'content_copy'} size={20} />
        {copied ? t('copied', lang) : t('copyPrompt', lang)}
      </button>
    </div>
  )
}
