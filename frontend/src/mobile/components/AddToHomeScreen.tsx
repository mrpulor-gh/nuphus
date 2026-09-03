/**
 * AddToHomeScreen — 主屏幕图标引导条
 *
 * 显示时机：配对成功（visible）+ 非 standalone 模式（浏览器内打开）+ 未关闭过。
 * 目的：引导用户用一个主屏图标随时独立开启，不被微信/抖音等宿主 App 锁死。
 *
 * 图标心智（发布口径，2026-09-03）：
 * - 中继 HTTPS origin（r.nuphus.com 等）：**推荐常驻图标**——在家打开自动直连局域网，
 *   出门自动走中继，一个图标家里家外通用。
 * - 局域网 http origin：**不再引导加局域网 http 图标当远程入口**（旧坑：http 出门白屏）。
 *   家内直连扫码/输地址即可，无需图标；如需出门也能用，用中继 HTTPS 地址添加图标。
 *
 * 交互：
 * - 按 origin 区分 localStorage 记录（公网/局域网各自独立关闭）。
 * - standalone 模式（已从桌面启动）不显示。
 * - 文案按平台区分：iOS Safari / Android Chrome。
 */
import { useEffect, useState } from 'react'
import { X } from 'lucide-react'
import { isPrivateHost, isStandalone } from '../connection'
import { t } from '../i18n'

const A2HS_DISMISSED_KEY = 'nuphus_mobile_a2hs_dismissed'
const A2HS_DISMISSED_KEY_LAN = 'nuphus_mobile_a2hs_dismissed_lan'

function detectStandalone(): boolean {
  return isStandalone()
}

function detectIos(): boolean {
  return /iphone|ipad|ipod/i.test(navigator.userAgent)
}

export default function AddToHomeScreen({ visible }: { visible: boolean }) {
  const isLan = isPrivateHost(window.location.hostname)
  const dismissKey = isLan ? A2HS_DISMISSED_KEY_LAN : A2HS_DISMISSED_KEY
  const [dismissed, setDismissed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(dismissKey) === '1'
    } catch {
      return false
    }
  })
  const [standalone, setStandalone] = useState(detectStandalone)

  useEffect(() => {
    const mq = window.matchMedia('(display-mode: standalone)')
    const onChange = () => setStandalone(detectStandalone())
    mq.addEventListener?.('change', onChange)
    return () => mq.removeEventListener?.('change', onChange)
  }, [])

  const dismiss = () => {
    setDismissed(true)
    try {
      localStorage.setItem(dismissKey, '1')
    } catch {
      /* ignore */
    }
  }

  if (!visible || standalone || dismissed) return null

  const ios = detectIos()
  const titleText = isLan ? t('mobile.a2hsLanTitle') : t('mobile.a2hsTitle')
  const stepText = isLan
    ? t('mobile.a2hsLanNote')
    : ios
      ? t('mobile.a2hsIosStep')
      : t('mobile.a2hsAndroidStep')

  return (
    <div className="mobile-a2hs" role="note">
      <div className="mobile-a2hs-icon" aria-hidden="true">
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M12 3v10" />
          <path d="M8 8l4-4 4 4" />
          <rect x="4" y="14" width="16" height="7" rx="2" />
        </svg>
      </div>
      <div className="mobile-a2hs-body">
        <div className="mobile-a2hs-title">{titleText}</div>
        <div className="mobile-a2hs-steps">{stepText}</div>
      </div>
      <button
        type="button"
        className="mobile-a2hs-close"
        onClick={dismiss}
        aria-label={t('mobile.closeHint')}
      >
        <X size={15} aria-hidden="true" />
      </button>
    </div>
  )
}
