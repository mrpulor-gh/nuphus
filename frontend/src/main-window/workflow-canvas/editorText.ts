import type { CanvasTranslate } from './presentation'

/** Translate known UI copy, never user-authored names, values or tool output. */
export function editorText(source: string, t: CanvasTranslate): string {
  const key = `workflowEditor.text.${source}`
  const translated = t(key)
  return translated === key ? source : translated
}

export function editorValidationText(source: string, t: CanvasTranslate): string {
  const translated = editorText(source, t)
  if (translated !== source) return translated
  for (const [prefix, key] of [
    ['不能小于 ', 'minimum'],
    ['不能大于 ', 'maximum'],
    ['必须大于 ', 'exclusiveMinimum'],
    ['必须小于 ', 'exclusiveMaximum'],
  ] as const) {
    if (source.startsWith(prefix))
      return t(`workflowEditor.validation.${key}`, source.slice(prefix.length))
  }
  return source
}
