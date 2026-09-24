import { useEffect, useState } from 'react'
import { listen } from '../../core/bridge'
import type { NuphusEvent } from '../../core/types'
import { projectExecutionActivity, type ExecutionActivity } from '../../core/executionActivity'
import { ExecutionActivityLine } from '../../ui/ExecutionActivityLine'

export function LiveExecutionActivity({ active }: { active: boolean }) {
  const [activity, setActivity] = useState<ExecutionActivity | null>(null)
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    listen<{ event: NuphusEvent }>('nuphus-event', ({ event }) => {
      if (!disposed) setActivity(prev => projectExecutionActivity(prev, event))
    }).then(fn => {
      if (disposed) fn()
      else unlisten = fn
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])
  useEffect(() => {
    if (!active) setActivity(null)
    else setActivity(prev => prev ?? { startedAt: Date.now(), phase: 'preparing', calls: [] })
  }, [active])
  return <ExecutionActivityLine activity={active ? activity : null} />
}
