import { useLanguage } from '../../locales'
import type { DesktopActionResult } from '../lib/desktopActionResult'

/** A separate receipt badge, intentionally not the generic green "completed". */
export function DesktopActionStatus({ result }: { result: DesktopActionResult }) {
  const { t } = useLanguage()
  const prefix =
    result.dispatchState === 'sent' && result.state !== 'sent'
      ? `${t('desktopAction.sent')} · `
      : ''
  return (
    <span
      className={`desktop-action-status is-${result.state}`}
      title={`${t(`desktopAction.${result.state}.detail`)}${result.deliveryMode ? ` · ${t(`desktopAction.delivery.${result.deliveryMode}`)}` : ''}`}
      data-dispatch-state={result.dispatchState}
      data-action-state={result.state}
      data-delivery-mode={result.deliveryMode}
    >
      {prefix}
      {t(`desktopAction.${result.state}`)}
    </span>
  )
}
