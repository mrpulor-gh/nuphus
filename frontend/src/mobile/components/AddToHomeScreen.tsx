/**
 * AddToHomeScreen — 「添加到主屏幕」引导条（PWA 桌面快捷方式入口）
 *
 * 显示时机：配对成功（visible）+ 非 standalone 模式（浏览器内打开）+ 未关闭过。
 * 目的：扫码/输密码配对成功后引导用户把手机端生成桌面快捷方式——之后随时独立开启，
 * 不被微信/抖音等宿主 App 锁死（打开后无法回到其他主应用的问题根源）。
 *
 * 两种 origin 都显示，但目的不同：
 * - 公网 origin（r.example.com 等）：引导生成桌面快捷方式，异地/中继入口。
 * - 局域网 origin（192.168.x.x）：**全屏直连**关键入口——iOS 从局域网地址添加主屏幕
 *   后，PWA scope 绑定局域网 origin：standalone 全屏 + 同 origin 直连桌面（速度最快，
 *   无 Mixed Content 拦截，也无需绕中继）。这是「同 WiFi 全屏 + 直连」的唯一实现路径。
 *   注：桌面 IP 变化后该快捷方式失效，需重新添加（局域网 DHCP 一般长期稳定）。
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
    ? ios
      ? t('mobile.a2hsLanIosStep')
      : t('mobile.a2hsLanAndroidStep')
    : ios
      ? t('mobile.a2hsIosStep')
      : t('mobile.a2hsAndroidStep')

  return (
    <div className="mobile-a2hs" role="note">
      <div className="mobile-a2hs-icon" aria-hidden="true">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
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
