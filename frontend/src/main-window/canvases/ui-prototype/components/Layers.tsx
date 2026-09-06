import { ReactNode, useMemo, useState } from 'react'
import { Reorder, useDragControls } from 'motion/react'
import { Frame, Group, Item, KIND_SPEC, Palette, explodeGroup, isPhoneFrame } from '../lib/tokens'
import { Icon } from './M3Node'
import { Lang, KIND_TEXT, t, useLang } from '../lib/i18n'

/* Rows never animate their size: opening a row only adds rows under it, so
 * nothing stretches. Only the drag itself moves.
 *
 * The panel is a tree with two kinds of level. A z level lists what is drawn on
 * top of what, front first: the runs of a screen, and the runs hidden inside a
 * free group. A run level lists the parts of one connected run in reading order
 * (left to right, top to bottom). Every level is reordered by dragging the
 * handle; the page turns the new order back into a group. */

function nameOf(it: Item, lang: Lang) {
  const spec = KIND_SPEC[it.kind]
  const noun = KIND_TEXT[lang][it.kind]?.noun ?? spec.label
  return (
    it.label.trim() || (it.kind === 'iconButton' || it.kind === 'fab' ? (it.icon ?? noun) : noun)
  )
}

function runLabel(g: Group, lang: Lang) {
  const first = g.items[0]
  const noun = KIND_TEXT[lang][first.kind]?.noun ?? KIND_SPEC[first.kind].label
  return g.free
    ? `${t('group', lang)} × ${g.items.length}`
    : g.items.length > 1
      ? `${noun} × ${g.items.length}`
      : nameOf(first, lang)
}

function Row({
  id,
  p,
  depth,
  icon,
  label,
  on,
  onSelect,
  open,
  onToggle,
  onDragging,
  children,
}: {
  id: string
  p: Palette
  depth: number
  icon: ReactNode
  label: string
  on: boolean
  onSelect: (add: boolean) => void
  /** set when the row can open to show what it holds */
  open?: boolean
  onToggle?: () => void
  onDragging: (dragging: boolean) => void
  children?: ReactNode
}) {
  const lang = useLang()
  const controls = useDragControls()
  const h = depth === 0 ? 40 : 36
  return (
    <Reorder.Item
      value={id}
      layout="position"
      transition={{ layout: { duration: 0 } }}
      dragListener={false}
      dragControls={controls}
      onDragStart={() => onDragging(true)}
      onDragEnd={() => onDragging(false)}
      style={{
        listStyle: 'none',
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        position: 'relative',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          height: h,
          padding: '0 6px 0 2px',
          marginLeft: depth * 14,
          borderRadius: depth === 0 ? 14 : 12,
          background: on ? p.secondaryContainer : depth === 0 ? p.surfaceContainerLow : p.surface,
          color: on ? p.onSecondaryContainer : p.onSurface,
          userSelect: 'none',
        }}
      >
        <span
          onPointerDown={e => {
            e.preventDefault()
            controls.start(e)
          }}
          style={{
            cursor: 'grab',
            color: p.outline,
            display: 'grid',
            placeItems: 'center',
            width: depth === 0 ? 24 : 20,
            height: h,
            touchAction: 'none',
          }}
        >
          <Icon name="drag_indicator" size={18} />
        </span>
        <button
          onClick={e => onSelect(e.shiftKey)}
          style={{
            flex: 1,
            minWidth: 0,
            height: h,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            border: 'none',
            background: 'transparent',
            color: 'inherit',
            cursor: 'pointer',
            padding: 0,
            textAlign: 'left',
          }}
        >
          <span
            style={{
              display: 'inline-flex',
              gap: 2,
              color: on ? p.onSecondaryContainer : p.primary,
            }}
          >
            {icon}
          </span>
          <span
            style={{
              fontSize: 12,
              fontWeight: depth === 0 ? 600 : 500,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {label}
          </span>
        </button>
        {onToggle && (
          <button
            onClick={onToggle}
            title={t(open ? 'hideParts' : 'showParts', lang)}
            aria-expanded={open}
            className="m3-press"
            style={{
              width: 28,
              height: 28,
              borderRadius: 14,
              border: 'none',
              background: 'transparent',
              color: on ? p.onSecondaryContainer : p.onSurfaceVariant,
              cursor: 'pointer',
              padding: 0,
              display: 'grid',
              placeItems: 'center',
              flex: '0 0 auto',
            }}
          >
            <span
              style={{
                display: 'inline-flex',
                transform: open ? 'rotate(90deg)' : 'none',
                transition: 'transform 160ms',
              }}
            >
              <Icon name="chevron_right" size={20} />
            </span>
          </button>
        )}
      </div>
      {open && children}
    </Reorder.Item>
  )
}

/** One reorderable level: drag the handles; `values` is the shown order. */
function Level({
  values,
  onReorder,
  children,
}: {
  values: string[]
  onReorder: (next: string[]) => void
  children: ReactNode
}) {
  return (
    <Reorder.Group
      axis="y"
      values={values}
      onReorder={onReorder}
      style={{ display: 'flex', flexDirection: 'column', gap: 4, padding: 0, margin: 0 }}
    >
      {children}
    </Reorder.Group>
  )
}

/** the parts of one connected run, in reading order */
function RunParts({
  run,
  p,
  depth,
  sel,
  onSelect,
  onReorder,
  onDragging,
}: {
  run: Group
  p: Palette
  depth: number
  sel: Set<string>
  onSelect: (itemIds: string[], add: boolean) => void
  /** the run's parts in a new reading order */
  onReorder: (ids: string[]) => void
  onDragging: (dragging: boolean) => void
}) {
  const lang = useLang()
  const ids = run.items.map(it => it.id)
  return (
    <Level values={ids} onReorder={onReorder}>
      {run.items.map(it => (
        <Row
          key={it.id}
          id={it.id}
          p={p}
          depth={depth}
          icon={<Icon name={KIND_SPEC[it.kind].paletteIcon} size={16} />}
          label={nameOf(it, lang)}
          on={sel.has(it.id)}
          onSelect={add => onSelect([it.id], add)}
          onDragging={onDragging}
        />
      ))}
    </Level>
  )
}

/** The z-order of one screen, top layer first, reordered by dragging the handles.
 *  A connected run opens to its parts in reading order; a free group opens to the
 *  runs it holds, front first, and each of those opens in turn. */
export function LayersPanel({
  p,
  frames,
  frameId,
  onFrame,
  groups,
  widths,
  selectedIds,
  onSelect,
  onReorder,
  onReorderItems,
  onDragging,
}: {
  p: Palette
  frames: Frame[]
  frameId: string | null
  onFrame: (id: string) => void
  /** the runs on the chosen frame, in canvas order (bottom first) */
  groups: Group[]
  widths: Record<string, number>
  selectedIds: string[]
  /** `add` is set when Shift was held, to extend the selection */
  onSelect: (itemIds: string[], add: boolean) => void
  /** new order, top layer first */
  onReorder: (topFirst: string[]) => void
  /** a group's parts in a new order: back to front for a free group, reading order for a run */
  onReorderItems: (groupId: string, ids: string[]) => void
  /** a drag on any level starting or ending, so the page can record one undo step for the whole drag */
  onDragging: (dragging: boolean) => void
}) {
  const lang = useLang()
  const topFirst = useMemo(() => [...groups].reverse(), [groups])
  const ids = topFirst.map(g => g.id)
  const sel = new Set(selectedIds)
  const [openIds, setOpenIds] = useState<Set<string>>(() => new Set())
  const toggle = (id: string) =>
    setOpenIds(cur => {
      const next = new Set(cur)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  /** the runs hidden in a free group, front first, and the flat back-to-front list they make */
  const freeRuns = (g: Group) => [...explodeGroup(g, widths)].reverse()
  const flatten = (runsTopFirst: Group[]) =>
    [...runsTopFirst].reverse().flatMap(r => r.items.map(it => it.id))

  const groupBody = (g: Group, depth: number) => {
    if (!g.free) {
      return (
        <RunParts
          run={g}
          p={p}
          depth={depth}
          sel={sel}
          onSelect={onSelect}
          onReorder={order => onReorderItems(g.id, order)}
          onDragging={onDragging}
        />
      )
    }
    const runs = freeRuns(g)
    const runIds = runs.map(r => r.id)
    const byId = new Map(runs.map(r => [r.id, r]))
    const reorderRuns = (next: string[]) =>
      onReorderItems(g.id, flatten(next.map(id => byId.get(id)).filter((r): r is Group => !!r)))
    return (
      <Level values={runIds} onReorder={reorderRuns}>
        {runs.map(r => {
          const many = r.items.length > 1
          const open = many && openIds.has(r.id)
          return (
            <Row
              key={r.id}
              id={r.id}
              p={p}
              depth={depth}
              icon={r.items.slice(0, 3).map((it, k) => (
                <Icon key={k} name={KIND_SPEC[it.kind].paletteIcon} size={16} />
              ))}
              label={runLabel(r, lang)}
              on={r.items.some(it => sel.has(it.id))}
              onSelect={add =>
                onSelect(
                  r.items.map(it => it.id),
                  add,
                )
              }
              open={many ? open : undefined}
              onToggle={many ? () => toggle(r.id) : undefined}
              onDragging={onDragging}
            >
              <RunParts
                run={r}
                p={p}
                depth={depth + 1}
                sel={sel}
                onSelect={onSelect}
                onDragging={onDragging}
                onReorder={order => {
                  /* the run's members take each other's places in the list; every other part keeps its own */
                  const members = new Set(r.items.map(it => it.id))
                  let k = 0
                  onReorderItems(
                    g.id,
                    g.items.map(it => (members.has(it.id) ? order[k++] : it.id)),
                  )
                }}
              />
            </Row>
          )
        })}
      </Level>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {frames.length > 1 && (
        <div
          className="no-scrollbar"
          style={{
            display: 'flex',
            gap: 6,
            padding: '12px 12px 4px',
            overflowX: 'auto',
            flex: '0 0 auto',
          }}
        >
          {frames.map(f => {
            const on = f.id === frameId
            return (
              <button
                key={f.id}
                onClick={() => onFrame(f.id)}
                className="m3-press"
                style={{
                  height: 32,
                  padding: '0 12px 0 8px',
                  borderRadius: 16,
                  border: 'none',
                  background: on ? p.primary : p.surfaceContainerHigh,
                  color: on ? p.onPrimary : p.onSurfaceVariant,
                  fontSize: 12,
                  fontWeight: 600,
                  cursor: 'pointer',
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 6,
                  whiteSpace: 'nowrap',
                  flex: '0 0 auto',
                }}
              >
                <Icon name={isPhoneFrame(f) ? 'smartphone' : 'desktop_windows'} size={16} />
                {f.name || t('screen', lang)}
              </button>
            )
          })}
        </div>
      )}
      <div
        className="no-scrollbar"
        style={{ flex: 1, overflowY: 'auto', padding: '8px 10px 12px' }}
      >
        {topFirst.length === 0 ? (
          <div style={{ padding: 24, textAlign: 'center', color: p.outline, fontSize: 12 }}>
            <Icon name="layers_clear" size={32} />
            <div style={{ marginTop: 8 }}>{t('noLayers', lang)}</div>
          </div>
        ) : (
          <Level values={ids} onReorder={onReorder}>
            {topFirst.map(g => {
              const canOpen = g.free || g.items.length > 1
              const open = canOpen && openIds.has(g.id)
              return (
                <Row
                  key={g.id}
                  id={g.id}
                  p={p}
                  depth={0}
                  icon={
                    g.free ? (
                      <Icon name="group_work" size={18} />
                    ) : (
                      g.items
                        .slice(0, 3)
                        .map((it, k) => (
                          <Icon key={k} name={KIND_SPEC[it.kind].paletteIcon} size={18} />
                        ))
                    )
                  }
                  label={runLabel(g, lang)}
                  on={g.items.some(it => sel.has(it.id))}
                  onSelect={add =>
                    onSelect(
                      g.items.map(it => it.id),
                      add,
                    )
                  }
                  open={canOpen ? open : undefined}
                  onToggle={canOpen ? () => toggle(g.id) : undefined}
                  onDragging={onDragging}
                >
                  {groupBody(g, 1)}
                </Row>
              )
            })}
          </Level>
        )}
      </div>
    </div>
  )
}
