/**
 * AddToHomeScreen — 主屏幕图标引导条
 *
 * 显示时机：配对成功（visible）+ 非 standalone 模式（浏览器内打开）+ 未关闭过。
 * 目的：引导用户用一个主屏图标随时独立开启，不被微信/抖音等宿主 App 锁死。
 *
 * 图标心智（2026-11-07 本地化口径）：
 * - iOS/Android 均默认通过无线局域网访问本机 Nuphus。
 * - 自定义服务端由用户自行部署；HTTPS 远程入口可按需添加到主屏幕。
 *
 * 交互：
 * - 按 origin 区分 localStorage 记录（公网/局域网各自独立关闭）。
 * - standalone 模式（已从桌面启动）不显示。
 * - 文案按平台区分：iOS Safari / Android Chrome。
 */
import { useEffect, useState } from 'react'
import { X, Share } from 'lucide-react'
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
        <Share size={18} strokeWidth={2} aria-hidden="true" />
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
