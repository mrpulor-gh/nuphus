import { createContext } from 'react'

/** Canvas supplies identity and changes refreshKey when execution progress arrives. */
export const WorkflowTraceContext = createContext<{
  workflowId: string
  refreshKey?: string | number
} | null>(null)
