import type { IntentStage } from './intentTypes'

export function intentDraftKey(workspace: string, workflow: string): string {
  return `nuphus.workflow.intent.v1:${JSON.stringify([workspace, workflow])}`
}

export function readIntentDraft(
  storage: Pick<Storage, 'getItem'>,
  key: string,
): IntentStage[] | null {
  const raw = storage.getItem(key)
  if (!raw) return null
  const value: unknown = JSON.parse(raw)
  if (
    !value ||
    typeof value !== 'object' ||
    !('version' in value) ||
    value.version !== 1 ||
    !('stages' in value) ||
    !Array.isArray(value.stages)
  ) {
    throw new Error('Unsupported draft format')
  }
  const ids = new Set<string>()
  const validId = (id: unknown) => {
    if (typeof id !== 'string' || !id || ids.has(id)) return false
    ids.add(id)
    return true
  }
  for (const stage of value.stages) {
    if (
      !stage ||
      !validId(stage.id) ||
      typeof stage.name !== 'string' ||
      !Array.isArray(stage.steps) ||
      stage.steps.some(
        (step: unknown) =>
          !step ||
          typeof step !== 'object' ||
          !('id' in step) ||
          !validId(step.id) ||
          !('intent' in step) ||
          typeof step.intent !== 'string',
      )
    ) {
      throw new Error('Invalid draft data')
    }
  }
  return value.stages as IntentStage[]
}

export function writeIntentDraft(
  storage: Pick<Storage, 'setItem'>,
  key: string,
  stages: IntentStage[],
): void {
  storage.setItem(key, JSON.stringify({ version: 1, stages }))
}
