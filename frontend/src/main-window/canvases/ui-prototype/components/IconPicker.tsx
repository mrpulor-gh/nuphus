import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { Palette } from '../lib/tokens'
import { t, useLang } from '../lib/i18n'

type IconMeta = { n: string; p: number; t: string }

const FONT = '24px "Material Symbols Rounded"'
/** module-level so validation survives re-mounts of the panel */
let cache: IconMeta[] | null = null
const glyphOk = new Map<string, boolean>()

/** Candidates measured per pass. A real Material Symbols glyph is exactly 1em
 *  wide; a name with no glyph falls back to text and measures wider. Icon names
 *  can be as short as "tv"/"4k", so we compare against a known-good reference
 *  glyph rather than a width threshold. */
const BATCH = 400
const SHOWN = 240

export function IconPicker({
  value,
  onChange,
  palette,
}: {
  value: string | null
  onChange: (icon: string | null) => void
  palette: Palette
}) {
  const lang = useLang()
  const [icons, setIcons] = useState<IconMeta[] | null>(cache)
  const [q, setQ] = useState('')
  const [fontReady, setFontReady] = useState(false)
  /* bumped after each measuring pass; the visible list depends on it because
   * glyphOk is a plain map, not React state */
  const [tick, bump] = useState(0)

  const refEl = useRef<HTMLSpanElement>(null)
  const probeEls = useRef<Map<string, HTMLElement>>(new Map())

  useEffect(() => {
    if (cache) return
    fetch('/material-symbols.json')
      .then(r => r.json())
      .then((d: IconMeta[]) => {
        cache = d
        setIcons(d)
      })
      .catch(() => setIcons([]))
  }, [])

  useEffect(() => {
    let alive = true
    const done = () => alive && setFontReady(true)
    if (document.fonts.check(FONT)) {
      done()
      return
    }
    document.fonts.load(FONT, 'search').then(done, done)
    return () => {
      alive = false
    }
  }, [])

  const candidates = useMemo(() => {
    if (!icons) return []
    const s = q.trim().toLowerCase().replace(/\s+/g, '_')
    if (!s) return icons.slice(0, BATCH)
    const raw = q.trim().toLowerCase()
    const starts: IconMeta[] = []
    const rest: IconMeta[] = []
    for (const i of icons) {
      if (i.n.startsWith(s)) starts.push(i)
      else if (i.n.includes(s) || i.t.includes(raw)) rest.push(i)
      if (starts.length + rest.length >= BATCH * 2) break
    }
    return [...starts, ...rest].slice(0, BATCH)
  }, [icons, q])

  const unknown = useMemo(() => candidates.filter(c => !glyphOk.has(c.n)), [candidates])

  useLayoutEffect(() => {
    if (!fontReady || unknown.length === 0) return
    const ref = refEl.current?.getBoundingClientRect().width ?? 0
    if (ref <= 0) return
    let learned = false
    for (const c of unknown) {
      const el = probeEls.current.get(c.n)
      if (!el) continue
      const w = el.getBoundingClientRect().width
      glyphOk.set(c.n, Math.abs(w - ref) < 0.75)
      learned = true
    }
    if (learned) bump(v => v + 1)
  }, [fontReady, unknown])

  const visible = useMemo(
    () => candidates.filter(c => glyphOk.get(c.n) === true).slice(0, SHOWN),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [candidates, fontReady, tick],
  )

  const loading = !icons || !fontReady

  return (
    <div>
      {/* Hidden probes need intrinsic text widths to distinguish missing glyphs. */}
      <div
        aria-hidden
        style={{
          position: 'fixed',
          left: -99999,
          top: 0,
          visibility: 'hidden',
          pointerEvents: 'none',
        }}
      >
        <span ref={refEl} className="msr" style={{ fontSize: 24, width: 'auto' }}>
          search
        </span>
        {unknown.map(c => (
          <span
            key={c.n}
            ref={el => {
              if (el) probeEls.current.set(c.n, el)
              else probeEls.current.delete(c.n)
            }}
            className="msr"
            style={{ fontSize: 24, width: 'auto' }}
          >
            {c.n}
          </span>
        ))}
      </div>

      <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 8 }}>
        <div style={{ position: 'relative', flex: 1, minWidth: 0 }}>
          <span
            className="msr"
            style={{
              position: 'absolute',
              left: 12,
              top: 10,
              fontSize: 20,
              color: palette.outline,
            }}
          >
            search
          </span>
          <input
            value={q}
            onChange={e => setQ(e.target.value)}
            placeholder={icons ? t('searchIcons', lang) : '…'}
            style={{
              width: '100%',
              height: 40,
              paddingLeft: 40,
              paddingRight: 12,
              borderRadius: 20,
              border: 'none',
              background: palette.surfaceContainerHigh,
              color: palette.onSurface,
              fontSize: 14,
              outline: 'none',
            }}
          />
        </div>
      </div>

      <div
        className="no-scrollbar"
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(42px, 1fr))',
          gap: 4,
          height: 244,
          overflowY: 'auto',
          overflowX: 'hidden',
          padding: 6,
          borderRadius: 16,
          background: palette.surfaceContainerLow,
          alignContent: 'start',
        }}
      >
        {visible.map(i => (
          <button
            key={i.n}
            title={i.n}
            onClick={() => onChange(i.n)}
            style={{
              aspectRatio: '1',
              minWidth: 0,
              display: 'grid',
              placeItems: 'center',
              borderRadius: 12,
              border: 'none',
              background: value === i.n ? palette.primary : 'transparent',
              color: value === i.n ? palette.onPrimary : palette.onSurfaceVariant,
              cursor: 'pointer',
            }}
          >
            <span className="msr" style={{ fontSize: 22 }}>
              {i.n}
            </span>
          </button>
        ))}
        {!loading && visible.length === 0 && (
          <div style={{ gridColumn: '1 / -1', padding: 16, fontSize: 13, color: palette.outline }}>
            <span className="msr" style={{ fontSize: 24 }}>
              search_off
            </span>
          </div>
        )}
        {loading && (
          <div style={{ gridColumn: '1 / -1', padding: 16, fontSize: 13, color: palette.outline }}>
            <span className="msr" style={{ fontSize: 24 }}>
              hourglass_top
            </span>
          </div>
        )}
      </div>
    </div>
  )
}
