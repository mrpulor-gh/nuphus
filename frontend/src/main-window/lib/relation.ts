// Relationship Config — 身份关系设置

const LS_ASSISTANT_NAME = 'nuphus_rel_assistant'
const LS_USER_LABEL = 'nuphus_rel_user_label'

export interface RelationConfig {
  assistantName: string
  userLabel: string
}

const DEFAULTS: RelationConfig = {
  assistantName: 'Nuphus',
  userLabel: 'USER',
}

export function loadRelation(): RelationConfig {
  try {
    return {
      assistantName: localStorage.getItem(LS_ASSISTANT_NAME) || DEFAULTS.assistantName,
      userLabel: localStorage.getItem(LS_USER_LABEL) || DEFAULTS.userLabel,
    }
  } catch {
    return DEFAULTS
  }
}

export function saveRelation(config: RelationConfig): void {
  try {
    localStorage.setItem(LS_ASSISTANT_NAME, config.assistantName)
    localStorage.setItem(LS_USER_LABEL, config.userLabel)
  } catch {
    /* noop */
  }
}
