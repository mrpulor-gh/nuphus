/**
 * 底部主导航（移动端唯一常驻导航入口）
 *
 * 为什么是底部 Tab 而不是「点 logo 侧滑抽屉」：
 * - 入口必须可见：品牌标识不携带「可点开菜单」的 affordance，用户无从推断；
 * - 单手可达：底部在拇指区，顶部左上角在右手持机时够不到；
 * - 层级明确：页面是常驻导航目标，不是覆盖在聊天上的浮层，不需要遮罩/滑出。
 *
 * 结构：三等分按钮（图标 + 文字），选中态用颜色与字重表达；高度含底部安全区，
 * 由 CSS 变量 --mobile-tabbar-h 统一给页面让位（见 mobile.css）。
 */

import { List, MessageCircle, Settings } from 'lucide-react'
import type { MobilePage } from './NavBar'
import { t } from '../i18n'

interface Props {
  page: MobilePage
  onChange: (page: MobilePage) => void
}

const TABS: { key: MobilePage; labelKey: string; Icon: typeof MessageCircle }[] = [
  { key: 'chat', labelKey: 'mobile.tabChat', Icon: MessageCircle },
  { key: 'sessions', labelKey: 'mobile.tabSessions', Icon: List },
  { key: 'settings', labelKey: 'mobile.tabSettings', Icon: Settings },
]

export default function TabBar({ page, onChange }: Props) {
  return (
    <nav className="mobile-tabbar" role="tablist" aria-label={t('mobile.tabChat')}>
      {TABS.map(({ key, labelKey, Icon }) => {
        const active = page === key
        return (
          <button
            key={key}
            type="button"
            role="tab"
            aria-selected={active}
            className={`mobile-tabbar-item${active ? ' is-active' : ''}`}
            onClick={() => onChange(key)}
          >
            <Icon size={20} strokeWidth={active ? 2.2 : 1.8} aria-hidden="true" />
            <span className="mobile-tabbar-label">{t(labelKey)}</span>
          </button>
        )
      })}
    </nav>
  )
}
