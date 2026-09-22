/**
 * AppIsland.tsx — 聊天区 header / 全屏宿主标题栏中央的 island 轻反馈层
 *
 * 落点（按优先级解析，见 islandChannel.ts 的「落点锚点」）：
 *   ① 全屏宿主锚点（设置中心 / 模型页标题栏的 .island-slot）—— 宿主压在聊天视图之上时
 *   ② 聊天区 `.chat-header` 中央的锚点
 *   ③ 都没有（聊天视图未挂载：启动屏 / 错误屏）→ 回落窗口级顶部居中（position: fixed）
 * 岛经 React Portal 挂进锚点内静态占位，随容器宽度自适应；**任何时刻都有落点**。
 *
 * 层级：跟随宿主层叠，不提高全局 z-index（依据见 app-island.css）。
 * 形态/语义：共享层 ui/AppPill.tsx + styles/app-pill.css（与模型页页内反馈同一份实现）。
 * 生命周期：进场 0.2s；「活跃态」（info）持续呼吸；退场淡出+上移；点击即关；
 *           role="status" + aria-live 播报（错误用 assertive，由本组件提供播报区）。
 *
 * 交互三态闭环：hover（暂停自动消失计时）/ focus-visible（box-shadow 焦点环）/
 * active（底色加深）。多条提示由 islandChannel.ts 队列串行，这里一次只渲染一条。
 */
import { useEffect, useState, useSyncExternalStore, type CSSProperties } from 'react'
import { createPortal } from 'react-dom'
import {
  dismissIslandToast,
  getAppIslandSnapshot,
  getIslandAnchor,
  isActiveFeedback,
  pauseIslandDwell,
  resumeIslandDwell,
  startAppFocusTracking,
  subscribeAppIsland,
  subscribeIslandAnchor,
  type AppIslandToast,
} from './islandChannel'
import { AppPill } from './AppPill'
import '../styles/app-island.css'

interface AppIslandProps {
  className?: string
  style?: CSSProperties
}

/** 系统级「减少动态效果」偏好；环境无 matchMedia 时按「不限动效」处理 */
function prefersReducedMotion(): boolean {
  return (
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

/** 命中偏好时挂 --still，关掉进场位移与呼吸（媒体查询为系统级兜底） */
function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(prefersReducedMotion)

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)')
    const onChange = () => setReduced(mq.matches)
    onChange()
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])

  return reduced
}

function AppIslandPill({
  toast,
  exiting,
  still,
}: {
  toast: AppIslandToast
  exiting: boolean
  still: boolean
}) {
  const classes = [
    'app-island-pill',
    // 呼吸只给活跃态；reduced-motion 下不挂呼吸，退场时摘掉以免与退场动画抢同一批属性
    !exiting && !still && isActiveFeedback(toast.type) ? 'app-island-pill--active' : '',
    exiting ? 'app-island-pill--exit' : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <AppPill
      tone={toast.type}
      interactive
      className={classes}
      onMouseEnter={pauseIslandDwell}
      onMouseLeave={resumeIslandDwell}
      onClick={dismissIslandToast}
    >
      {toast.message}
    </AppPill>
  )
}

export function AppIsland({ className, style }: AppIslandProps = {}) {
  const { toast, exiting } = useSyncExternalStore(subscribeAppIsland, getAppIslandSnapshot)
  // 落点：按优先级解析出的锚点（宿主 → 聊天 header）；null = 原地（窗口级回落定位）
  const anchor = useSyncExternalStore(subscribeIslandAnchor, getIslandAnchor)
  const still = usePrefersReducedMotion()

  // 焦点跟踪 = island / HUD 的分流依据；挂载即对齐真实焦点
  useEffect(() => startAppFocusTracking(), [])

  const island = (
    <div
      className={[
        'app-island',
        // 锚点内 = 静态占位变体（取消 fixed / transform，见 app-island.css）
        anchor && 'app-island--anchored',
        still && 'app-island--still',
        className,
      ]
        .filter(Boolean)
        .join(' ')}
      style={style}
      // 播报区常驻（有内容才插入胶囊）：live region 先存在才可靠播报
      role="status"
      aria-live={toast?.type === 'error' ? 'assertive' : 'polite'}
      aria-atomic="true"
    >
      {/* key 绑定提示 id：换条时重新挂载，进场动画得以重放 */}
      {toast && <AppIslandPill key={toast.id} toast={toast} exiting={exiting} still={still} />}
    </div>
  )

  // 锚点未就绪（聊天视图与宿主都未挂载）时不挂 portal：原地渲染即「窗口级顶部居中」
  return anchor ? createPortal(island, anchor) : island
}
