/**
 * AppPill.tsx — 轻反馈胶囊（应用内提示的**唯一**形态实现）
 *
 * 两处消费方共用同一份标记与同一批共享类（styles/app-pill.css）：
 *   ① island：`interactive` + 生命周期类（呼吸/退场，见 app-island.css）
 *   ② 模型页页内反馈：`multiline`（承载错误详情，不截断），定位与层叠归宿主
 * 只共享形态与语义，不共享容器 —— 定位各归其宿主，层级因此随宿主走，不靠提高 z-index。
 *
 * 语义**只靠图标颜色 + 一枚指示点**（--app-pill-tone），禁止整块绿/黄/红底卡片。
 */
import type { ReactNode } from 'react'
import { IconAlertCircle, IconAlertTriangle, IconCheck, IconInfo } from './Icons'
import type { AppFeedback } from './islandChannel'
import '../styles/app-pill.css'

export interface AppPillProps {
  /** 语义相位：决定图标与着色（与岛通道的 AppFeedback 同一套） */
  tone: AppFeedback
  /** true = 渲染为 <button>（岛：hover 暂停计时 / 点击关闭）；缺省为纯展示 */
  interactive?: boolean
  /** 长文案换行变体（页内反馈承载错误详情；岛恒为单行省略） */
  multiline?: boolean
  /** 生命周期 / 容器类（岛：--active / --exit / 定位等） */
  className?: string
  children: ReactNode
  onMouseEnter?: () => void
  onMouseLeave?: () => void
  onClick?: () => void
}

/**
 * 相位 → 图标（一律经 ui/Icons.tsx 出口）。
 * 逐个 case 渲染而非组件映射表：lucide 的 props 类型（size: string | number）
 * 与通用 ComponentType 不兼容，显式分支既避开类型体操又保持单一出口。
 */
function PhaseIcon({ tone }: { tone: AppFeedback }) {
  const props = { className: 'app-pill-icon', size: 14 }
  switch (tone) {
    case 'success':
      return <IconCheck {...props} />
    case 'warning':
      return <IconAlertTriangle {...props} />
    case 'error':
      return <IconAlertCircle {...props} />
    default:
      return <IconInfo {...props} />
  }
}

export function AppPill({
  tone,
  interactive,
  multiline,
  className,
  children,
  onMouseEnter,
  onMouseLeave,
  onClick,
}: AppPillProps) {
  const classes = [
    'app-pill',
    `app-pill--${tone}`,
    // 长文案换行只给页内反馈；岛恒为单行省略
    multiline && 'app-pill--multiline',
    className,
  ]
    .filter(Boolean)
    .join(' ')

  const body = (
    <>
      <PhaseIcon tone={tone} />
      <span className="app-pill-text">{children}</span>
      <span className="app-pill-dot" aria-hidden="true" />
    </>
  )

  // 显式两分支而非多态组件：button 与 div 的属性/语义不同，映射类型只会把调用点搞复杂
  if (!interactive) return <div className={classes}>{body}</div>
  return (
    <button
      type="button"
      className={classes}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      onClick={onClick}
    >
      {body}
    </button>
  )
}
