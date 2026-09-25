import { useState } from 'react'
import { useLanguage } from '../../locales'
import type { RenamePreview } from './variableRename'

export function VariableRenameDialog({
  preview,
  onApply,
  onCancel,
}: {
  preview: RenamePreview
  onApply: (choices: Record<string, boolean>) => void
  onCancel: () => void
}) {
  const { t } = useLanguage()
  const [choices, setChoices] = useState<Record<string, boolean>>({})
  return (
    <div className="wfc-intent-mask" onClick={onCancel}>
      <div
        className="wfc-intent"
        role="dialog"
        aria-modal="true"
        aria-label={t('workflowEditor.rename.title')}
        onClick={event => event.stopPropagation()}
      >
        <div className="wfc-intent-head">
          <h3>{t('workflowEditor.rename.title')}</h3>
          <code>
            {preview.oldName} → {preview.newName}
          </code>
        </div>
        <div className="wfc-intent-body">
          <p>{t('workflowEditor.rename.hint')}</p>
          {preview.error && <p role="alert">{t(`workflowEditor.rename.${preview.error}`)}</p>}
          {!preview.changes.length && <p>{t('workflowEditor.rename.noReferences')}</p>}
          {preview.changes.map(change => (
            <section key={change.id}>
              <strong>
                {change.stepName || change.stepId} · {change.fieldPath}
              </strong>
              {change.ambiguous && (
                <label>
                  <span>{t('workflowEditor.rename.ambiguous')}</span>
                  <select
                    aria-label={`${change.stepId} ${change.fieldPath}`}
                    value={change.id in choices ? String(choices[change.id]) : ''}
                    onChange={event =>
                      setChoices(value => ({
                        ...value,
                        [change.id]: event.target.value === 'true',
                      }))
                    }
                  >
                    <option value="" disabled>
                      {t('workflowEditor.rename.choose')}
                    </option>
                    <option value="true">{t('workflowEditor.rename.update')}</option>
                    <option value="false">{t('workflowEditor.rename.keep')}</option>
                  </select>
                </label>
              )}
              <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
                − {change.before}
                {'\n'}+ {change.after}
              </pre>
            </section>
          ))}
        </div>
        <div className="wfc-intent-foot">
          <button type="button" className="wfc-btn" onClick={onCancel}>
            {t('common.cancel')}
          </button>
          <button
            type="button"
            className="wfc-btn wfc-btn--primary"
            disabled={
              !!preview.error ||
              preview.changes.some(change => change.ambiguous && !(change.id in choices))
            }
            onClick={() => onApply(choices)}
          >
            {t('workflowEditor.rename.apply')}
          </button>
        </div>
      </div>
    </div>
  )
}
