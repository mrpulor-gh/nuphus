import { useLanguage } from '../../locales'
import { nodeKindDescription, nodeKindLabel } from './presentation'

export function NodeKindBadge({ kind }: { kind: string }) {
  const { t } = useLanguage()
  const label = nodeKindLabel(kind, t)
  return (
    <span className="wfc-node-kind" title={`${label} (${kind}) — ${nodeKindDescription(kind, t)}`}>
      {label}
    </span>
  )
}
