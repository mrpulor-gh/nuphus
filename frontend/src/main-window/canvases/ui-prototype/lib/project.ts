import { Doc, KIND_ORDER, Kind, VARIANTS, isPlace, isPlatform } from './tokens'

/* A project file is the Doc as JSON, nothing more. Reading one back only checks
 * the shape the editor relies on; the same migrations that run on a saved
 * document then bring an older file up to date. */

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null

const KINDS = new Set<string>(KIND_ORDER)

const validTabs = (tabs: unknown) =>
  tabs === undefined ||
  (Array.isArray(tabs) &&
    tabs.every(
      tab =>
        isRecord(tab) &&
        typeof tab.label === 'string' &&
        (typeof tab.icon === 'string' || tab.icon === null || tab.icon === undefined),
    ))

const validCorners = (c: unknown) =>
  c === undefined || (isRecord(c) && ['tl', 'tr', 'bl', 'br'].every(k => Number.isFinite(c[k])))

const validItem = (item: unknown) =>
  isRecord(item) &&
  validCorners(item.corners) &&
  typeof item.id === 'string' &&
  typeof item.kind === 'string' &&
  KINDS.has(item.kind as Kind) &&
  typeof item.label === 'string' &&
  (typeof item.icon === 'string' || item.icon === null) &&
  VARIANTS.some(variant => variant.key === item.variant) &&
  (item.supporting === undefined || typeof item.supporting === 'string') &&
  (item.selected === undefined || Number.isFinite(item.selected)) &&
  (item.note === undefined || typeof item.note === 'string') &&
  validTabs(item.tabs)

const validGroup = (group: unknown) =>
  isRecord(group) &&
  typeof group.id === 'string' &&
  Number.isFinite(group.x) &&
  Number.isFinite(group.y) &&
  (group.axis === 'x' || group.axis === 'y') &&
  Array.isArray(group.items) &&
  group.items.length > 0 &&
  group.items.every(validItem)

const validFrame = (frame: unknown) =>
  isRecord(frame) &&
  typeof frame.id === 'string' &&
  typeof frame.name === 'string' &&
  Number.isFinite(frame.x) &&
  Number.isFinite(frame.y) &&
  (frame.w === undefined || (Number.isFinite(frame.w) && (frame.w as number) > 0)) &&
  (frame.h === undefined || (Number.isFinite(frame.h) && (frame.h as number) > 0)) &&
  (frame.note === undefined || typeof frame.note === 'string') &&
  (frame.place === undefined || isPlace(frame.place))

/** whether a parsed file has the shape of a document the editor can open */
export const isProject = (value: unknown): value is Doc =>
  isRecord(value) &&
  Array.isArray(value.groups) &&
  Array.isArray(value.frames) &&
  value.groups.every(validGroup) &&
  value.frames.every(validFrame) &&
  (value.platform === undefined || isPlatform(value.platform))

/** the file name a project is saved under: ui-proto-canvas, followed by the app's name when it has one */
export const projectFileName = (doc: Doc) => {
  const name = doc.title
    .trim()
    .replace(/[\\/:*?"<>|]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
  return name ? `ui-proto-canvas ${name}.json` : 'ui-proto-canvas.json'
}

/** hands the document to the browser as a JSON download */
export function saveProject(doc: Doc) {
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(doc, null, 2)], { type: 'application/json' }),
  )
  const a = document.createElement('a')
  a.href = url
  a.download = projectFileName(doc)
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 0)
}

/** reads a chosen file back into a document, or null when it is not one */
export async function readProject(file: File): Promise<Doc | null> {
  try {
    const next: unknown = JSON.parse(await file.text())
    return isProject(next) ? next : null
  } catch {
    return null
  }
}
