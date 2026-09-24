import { useEffect, useState } from 'react'
import { activityLabels, type ExecutionActivity } from '../core/executionActivity'

export function ExecutionActivityLine({ activity }: { activity: ExecutionActivity | null }) {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    if (!activity) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [activity?.startedAt])
  if (!activity) return null
  const seconds = Math.max(0, Math.floor((now - activity.startedAt) / 1000))
  return (
    <div
      role="status"
      style={{ color: 'var(--text-secondary)', fontSize: 12, padding: '10px 16px' }}
    >
      <span>{activityLabels[activity.phase]}</span>
      <span aria-live="off"> · 已用时 {seconds} 秒</span>
    </div>
  )
}
