import type { WorkflowEnhancedMode } from '../lib/api'

export const WORKFLOW_ENHANCED_MODE_CHANGED_EVENT = 'nuphus:workflow-enhanced-mode-changed'
export const WORKFLOW_ENHANCED_MODE_REFRESH_EVENT = 'nuphus:workflow-enhanced-mode-refresh'

export function publishWorkflowEnhancedMode(state: WorkflowEnhancedMode): void {
  window.dispatchEvent(
    new CustomEvent<WorkflowEnhancedMode>(WORKFLOW_ENHANCED_MODE_CHANGED_EVENT, {
      detail: state,
    }),
  )
}

export function requestWorkflowEnhancedModeRefresh(): void {
  window.dispatchEvent(new Event(WORKFLOW_ENHANCED_MODE_REFRESH_EVENT))
}
