import { useEffect, useState } from 'react'
import { CONTRASTS, Contrast, PALETTES, Palette, Theme } from '../lib/tokens'
import { isHex, onColorFor, schemeFromSeed } from '../lib/color'
import { t, useLang } from '../lib/i18n'
import { Section, Segmented } from './ui'
import { Icon } from './M3Node'

/** roles the author can override by hand; their "on" color follows automatically */
type Role = Exclude<keyof Palette, 'seed'>
const TUNABLE: { key: Role; on?: Role }[] = [
  { key: 'primary', on: 'onPrimary' },
  { key: 'primaryContainer', on: 'onPrimaryContainer' },
  { key: 'secondaryContainer', on: 'onSecondaryContainer' },
  { key: 'tertiaryContainer', on: 'onTertiaryContainer' },
  { key: 'surfaceContainer' },
  { key: 'surfaceContainerHigh' },
  { key: 'inverseSurface', on: 'inverseOnSurface' },
]

/** the palette's key colors as overlapping dots, right-aligned in the row */
function Swatches({ pal, p }: { pal: Palette; p: Palette }) {
  const colors = [
    pal.primary,
    pal.primaryContainer,
    pal.secondaryContainer,
    pal.tertiaryContainer,
    pal.surfaceContainerHigh,
  ]
  return (
    <span style={{ display: 'inline-flex', flex: '0 0 auto', marginLeft: 'auto', paddingLeft: 8 }}>
      {colors.map((c, i) => (
        <span
          key={i}
          style={{
            width: 20,
            height: 20,
            borderRadius: 10,
            background: c,
            marginLeft: i === 0 ? 0 : -7,
            boxShadow: `0 0 0 2px ${p.surfaceContainerLow}, inset 0 0 0 1px rgba(0,0,0,0.08)`,
            zIndex: colors.length - i,
            position: 'relative',
          }}
        />
      ))}
    </span>
  )
}

function ColorField({
  value,
  onChange,
  p,
  label,
}: {
  value: string
  onChange: (hex: string) => void
  p: Palette
  label: string
}) {
  const [text, setText] = useState(value)
  useEffect(() => setText(value), [value])
  return (
    <label style={{ display: 'flex', alignItems: 'center', gap: 8, height: 36 }}>
      <span
        style={{
          position: 'relative',
          width: 28,
          height: 28,
          borderRadius: 14,
          background: value,
          boxShadow: 'inset 0 0 0 1px rgba(0,0,0,0.12)',
          flex: '0 0 auto',
          overflow: 'hidden',
        }}
      >
        <input
          type="color"
          value={value}
          onChange={e => onChange(e.target.value.toUpperCase())}
          aria-label={label}
          style={{
            position: 'absolute',
            inset: -8,
            width: 44,
            height: 44,
            opacity: 0,
            cursor: 'pointer',
          }}
        />
      </span>
      <span
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: 12,
          color: p.onSurface,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {label}
      </span>
      <input
        value={text}
        onChange={e => {
          setText(e.target.value)
          if (isHex(e.target.value)) onChange(e.target.value.toUpperCase())
        }}
        onBlur={() => setText(value)}
        spellCheck={false}
        style={{
          width: 76,
          height: 28,
          padding: '0 8px',
          borderRadius: 8,
          border: 'none',
          background: p.surfaceContainerHigh,
          color: p.onSurface,
          fontSize: 12,
          fontFamily: 'ui-monospace, monospace',
          outline: 'none',
        }}
      />
    </label>
  )
}

/** A settings row in the same shape as the palette rows: icon disc, label, and the
 *  value or switch at the trailing edge. The whole row is the control. */
function SettingRow({
  p,
  icon,
  label,
  onClick,
  pressed,
  children,
}: {
  p: Palette
  icon: string
  label: string
  onClick: () => void
  pressed?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      onClick={onClick}
      aria-pressed={pressed}
      className="m3-press"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        height: 48,
        padding: '0 12px 0 10px',
        borderRadius: 16,
        border: 'none',
        background: p.surfaceContainerLow,
        color: p.onSurface,
        cursor: 'pointer',
        textAlign: 'left',
        width: '100%',
      }}
    >
      <span
        style={{
          width: 28,
          height: 28,
          borderRadius: 14,
          background: p.secondaryContainer,
          color: p.onSecondaryContainer,
          display: 'grid',
          placeItems: 'center',
          flex: '0 0 auto',
        }}
      >
        <Icon name={icon} size={18} />
      </span>
      <span
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: 13,
          fontWeight: 600,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {label}
      </span>
      {children}
    </button>
  )
}

/** the switch of a settings row, drawn without its own button */
function Knob({ on, p }: { on: boolean; p: Palette }) {
  return (
    <span
      aria-hidden
      style={{
        position: 'relative',
        width: 44,
        height: 26,
        borderRadius: 13,
        background: on ? p.primary : p.surfaceContainerHighest,
        border: on ? '2px solid transparent' : `2px solid ${p.outline}`,
        boxSizing: 'border-box',
        transition: 'background 160ms',
        flex: '0 0 auto',
      }}
    >
      <span
        style={{
          position: 'absolute',
          top: '50%',
          left: on ? 20 : 3,
          width: on ? 18 : 12,
          height: on ? 18 : 12,
          marginTop: on ? -9 : -6,
          borderRadius: 9,
          background: on ? p.onPrimary : p.outline,
          transition: 'left 160ms, width 160ms, height 160ms, margin 160ms',
        }}
      />
    </span>
  )
}

/** The color tab of the theme panel: light / dark and contrast, preset themes,
 *  a seed-based custom scheme with per-role tweaks, and the dynamic-color switch. */
export function ColorPanel({
  p,
  paletteKey,
  onPalette,
  custom,
  onCustom,
  dynamic,
  onDynamic,
  theme,
  onTheme,
}: {
  p: Palette
  paletteKey: string
  onPalette: (key: string) => void
  custom: Palette | null
  onCustom: (pal: Palette) => void
  dynamic: boolean
  onDynamic: (on: boolean) => void
  theme: Theme
  onTheme: (patch: Partial<Theme>) => void
}) {
  const lang = useLang()
  const contrastLabel = (c: Contrast) =>
    c === 'high'
      ? t('contrastHigh', lang)
      : c === 'medium'
        ? t('contrastMedium', lang)
        : t('contrastStandard', lang)
  const [tab, setTab] = useState<'templates' | 'custom'>(
    paletteKey === 'custom' ? 'custom' : 'templates',
  )
  const [seed, setSeed] = useState(custom?.seed ?? custom?.primary ?? '#6750A4')

  const applySeed = (hex: string) => {
    setSeed(hex)
    onCustom(schemeFromSeed(hex))
    onPalette('custom')
  }
  const cur = custom ?? schemeFromSeed(seed)

  return (
    <div
      className="no-scrollbar"
      style={{ height: '100%', overflowY: 'auto', padding: '12px 12px 20px' }}
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginBottom: 14 }}>
        <SettingRow
          p={p}
          icon={theme.dark ? 'dark_mode' : 'light_mode'}
          label={t('brightness', lang)}
          onClick={() => onTheme({ dark: !theme.dark })}
          pressed={theme.dark}
        >
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              fontSize: 13,
              fontWeight: 600,
              color: p.primary,
            }}
          >
            {theme.dark ? t('dark', lang) : t('light', lang)}
            <Icon name="swap_horiz" size={18} />
          </span>
        </SettingRow>
        <SettingRow
          p={p}
          icon="routine"
          label={t('bothModes', lang)}
          onClick={() => onTheme({ bothModes: !theme.bothModes })}
          pressed={theme.bothModes}
        >
          <Knob on={theme.bothModes} p={p} />
        </SettingRow>
        <SettingRow
          p={p}
          icon="contrast"
          label={t('contrast', lang)}
          onClick={() => {
            const i = CONTRASTS.findIndex(c => c.key === theme.contrast)
            onTheme({ contrast: CONTRASTS[(i + 1) % CONTRASTS.length].key })
          }}
        >
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              fontSize: 13,
              fontWeight: 600,
              color: p.primary,
            }}
          >
            {contrastLabel(theme.contrast)}
            <Icon name="chevron_right" size={18} />
          </span>
        </SettingRow>
      </div>

      <Segmented<'templates' | 'custom'>
        options={[
          { key: 'templates', icon: 'palette', label: t('templates', lang) },
          { key: 'custom', icon: 'colorize', label: t('customColor', lang) },
        ]}
        value={tab}
        onChange={setTab}
        p={p}
        height={38}
      />

      {tab === 'templates' ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginTop: 12 }}>
          {PALETTES.map(pal => {
            const on = pal.key === paletteKey
            return (
              <button
                key={pal.key}
                onClick={() => onPalette(pal.key)}
                aria-pressed={on}
                className="m3-press"
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  height: 48,
                  padding: '0 12px 0 10px',
                  borderRadius: 16,
                  border: 'none',
                  background: on ? p.secondaryContainer : p.surfaceContainerLow,
                  color: on ? p.onSecondaryContainer : p.onSurface,
                  cursor: 'pointer',
                  textAlign: 'left',
                }}
              >
                <span
                  style={{
                    width: 28,
                    height: 28,
                    borderRadius: 14,
                    background: `linear-gradient(135deg, ${pal.primary} 50%, ${pal.primaryContainer} 50%)`,
                    flex: '0 0 auto',
                  }}
                />
                <span
                  style={{
                    fontSize: 13,
                    fontWeight: 600,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    minWidth: 0,
                  }}
                >
                  {pal.label}
                </span>
                <Swatches pal={pal} p={p} />
              </button>
            )
          })}
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginTop: 12 }}>
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              flexWrap: 'wrap',
              justifyContent: 'flex-end',
              gap: 8,
              padding: 10,
              borderRadius: 16,
              background: paletteKey === 'custom' ? p.secondaryContainer : p.surfaceContainerLow,
              color: paletteKey === 'custom' ? p.onSecondaryContainer : p.onSurface,
            }}
          >
            <div style={{ flex: '1 1 180px', minWidth: 0 }}>
              <ColorField value={seed} onChange={applySeed} p={p} label={t('seedColor', lang)} />
            </div>
            {paletteKey !== 'custom' && (
              <button
                onClick={() => applySeed(seed)}
                className="m3-press"
                style={{
                  height: 32,
                  padding: '0 12px',
                  borderRadius: 16,
                  border: 'none',
                  background: p.primary,
                  color: p.onPrimary,
                  fontSize: 12,
                  fontWeight: 600,
                  cursor: 'pointer',
                }}
              >
                {t('useThis', lang)}
              </button>
            )}
          </div>
          <div
            style={{ fontSize: 11, lineHeight: 1.5, color: p.onSurfaceVariant, padding: '0 4px' }}
          >
            {t('seedHint', lang)}
          </div>
          <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', padding: '0 2px' }}>
            {[
              '#6750A4',
              '#0B57D0',
              '#2E6A45',
              '#984061',
              '#8B5000',
              '#00696E',
              '#B3261E',
              '#4A4459',
            ].map(c => (
              <button
                key={c}
                onClick={() => applySeed(c)}
                title={c}
                aria-label={c}
                className="m3-press"
                style={{
                  width: 26,
                  height: 26,
                  borderRadius: 13,
                  border: 'none',
                  padding: 0,
                  background: c,
                  cursor: 'pointer',
                  outline: seed === c ? `2px solid ${p.primary}` : '2px solid transparent',
                  outlineOffset: 2,
                }}
              />
            ))}
          </div>
          <Section
            id="color-tune"
            icon="tune"
            title={t('fineTune', lang)}
            p={p}
            defaultOpen={false}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
              {TUNABLE.map(r => (
                <ColorField
                  key={r.key}
                  value={cur[r.key]}
                  label={r.key}
                  p={p}
                  onChange={hex => {
                    const next: Palette = { ...cur, [r.key]: hex }
                    if (r.on) (next as Record<string, string>)[r.on] = onColorFor(hex)
                    onCustom(next)
                    onPalette('custom')
                  }}
                />
              ))}
            </div>
          </Section>
        </div>
      )}

      <div style={{ marginTop: 16, display: 'flex', flexDirection: 'column', gap: 6 }}>
        <SettingRow
          p={p}
          icon="wallpaper"
          label={t('dynamicColor', lang)}
          onClick={() => onDynamic(!dynamic)}
          pressed={dynamic}
        >
          <Knob on={dynamic} p={p} />
        </SettingRow>
        <div
          style={{ fontSize: 11, lineHeight: 1.5, color: p.onSurfaceVariant, padding: '2px 4px 0' }}
        >
          {dynamic ? t('dynamicOnHint', lang) : t('dynamicOffHint', lang)}
        </div>
      </div>
    </div>
  )
}
