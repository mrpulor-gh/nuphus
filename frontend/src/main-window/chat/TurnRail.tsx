/**
 * TurnRail —— 对话窗口右侧的轮次轨。
 *
 * 形态：固定节距的刻度梯，每枚刻度 = 一轮对话；点击（或键盘聚焦回车）跳转到
 * 该轮首条消息。悬停/聚焦弹出预览气泡（提问一行 + 答复三行）。
 *
 * 行为约定：
 * - 刻度不足两枚不渲染（无导航意义）；
 * - 激活轮由滚动位置经 turnRailModel 推断，本组件只透传 messages；
 * - **触底（含刚发送新消息）时激活最新一轮**，用户翻历史时按判定线取阅读位置；
 * - 激活轮变化时刻度梯自动跟进（仅当指针不在轨上——不抢用户手中的滚动）；
 * - 末端渐隐用遮罩实现，不裁剪交互区域；
 * - 尊重 prefers-reduced-motion：平滑滚动回退为瞬时。
 *
 * 实现注意：宿主滚动容器经 ref 透传，**宿主 ref 的 .current 在挂载时的
 * useLayoutEffect 阶段尚未附着**（实测本仓库 React 版本的行为），因此首测与
 * 视口同步一律放在 passive effect + rAF 里做；layout effect 只用于锚点集合
 * 变化后的即时重算（那时 ref 必然已附着）。
 */
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react'
import {
  buildTurnAnchors,
  isAtBottom,
  pickActiveTurn,
  RAIL_INSET_PX,
  TURN_PITCH_PX,
} from './turnRailModel'

/** 组件只依赖消息的 id/role/content，不耦合完整 ChatMessage。 */
export interface TurnRailMessage {
  id: string
  role: string
  content: string
}

interface TurnRailProps {
  messages: readonly TurnRailMessage[]
  /** 滚动容器（.chat-messages）——激活轮推断与跳转定位都基于它 */
  scrollRef: RefObject<HTMLDivElement | null>
  /** i18n：键名见 locales 的 chat.turnRail.* */
  t: (key: string, ...args: string[]) => string
}

/** 单枚刻度按钮的高度 = 节距（命中区与刻度间距同尺寸，10px 好点中）。 */
const MARK_HEIGHT_PX = TURN_PITCH_PX

/** 是否偏好弱化动效（用于滚动行为降级）。 */
function prefersReducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches
}

/** 浏览器侧滚动行为：默认平滑，弱化动效时瞬时。 */
function scrollBehavior(): ScrollBehavior {
  return prefersReducedMotion() ? 'auto' : 'smooth'
}

/** 刻度梯内容总高（含首尾留白）。 */
function railContentHeight(count: number): number {
  return RAIL_INSET_PX * 2 + Math.max(0, count - 1) * TURN_PITCH_PX
}

export function TurnRail({ messages, scrollRef, t }: TurnRailProps) {
  const anchors = useMemo(() => buildTurnAnchors(messages), [messages])
  const previewId = useId()

  const [previewTurn, setPreviewTurn] = useState<number | null>(null)
  const [activeTurn, setActiveTurn] = useState<number | null>(null)
  const [railOffset, setRailOffset] = useState(0)
  const [railView, setRailView] = useState(0)

  const viewportRef = useRef<HTMLDivElement | null>(null)
  /** 指针是否正停在轨上：此时激活轮跟进让位给用户操作 */
  const pointerInsideRef = useRef(false)
  /** 锚点实测偏移缓存（messageId → 内容坐标），滚动回调里只做算术不读 DOM */
  const topsRef = useRef(new Map<string, number>())
  /** 上一次随见的锚点数 / 视口高——结构变化时跟进动效降级为瞬时 */
  const followRef = useRef<{ count: number; height: number } | null>(null)

  /* ── 锚点实测：把锚点行换算成滚动内容坐标 ── */
  const measure = useCallback(() => {
    const container = scrollRef.current
    if (!container) return
    const containerTop = container.getBoundingClientRect().top
    const tops = new Map<string, number>()
    for (const anchor of anchors) {
      const el = document.getElementById(`turn-anchor-${anchor.messageId}`)
      if (!el) continue
      // 换算到滚动内容坐标：视口坐标 − 容器视口坐标 + 当前 scrollTop
      tops.set(
        anchor.messageId,
        el.getBoundingClientRect().top - containerTop + container.scrollTop,
      )
    }
    topsRef.current = tops
  }, [anchors, scrollRef])

  /** 同步刻度梯自身视口（渐隐判断用）。 */
  const syncRailView = useCallback(() => {
    const el = viewportRef.current
    if (!el) return
    setRailView(el.clientHeight)
    setRailOffset(el.scrollTop)
  }, [])

  /**
   * 按当前滚动位置重算激活轮。锚点集合变化（新消息）、滚动、尺寸变化都走它。
   *
   * 触底时激活最新一轮——发送后对话钉在底部，新刻度的 is-active 必须即时跟上
   * （旧实现只认判定线，新锚点还在判定线之下时会停在上一轮）。用户翻历史时
   * （未触底）仍按判定线取阅读位置，两者不会互相抢。
   */
  const recompute = useCallback(() => {
    const container = scrollRef.current
    if (!container) return
    setActiveTurn(
      pickActiveTurn(
        anchors,
        topsRef.current,
        container.scrollTop,
        container.clientHeight,
        isAtBottom(container.scrollTop, container.clientHeight, container.scrollHeight),
      ),
    )
  }, [anchors, scrollRef])

  /* 首测 + 视口同步 + 迟到的补测：放 passive effect（宿主 ref 此时已附着），
     并用 rAF 让位于本轮渲染的布局；锚点集合变化时 layout effect 仍会即时重算。 */
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      measure()
      syncRailView()
      recompute()
    })
    // 消息区渲染（markdown / 图片撑高）后布局才最终稳定，补一次迟到的实测
    const timer = window.setTimeout(() => {
      measure()
      syncRailView()
      recompute()
    }, 300)
    return () => {
      cancelAnimationFrame(frame)
      window.clearTimeout(timer)
    }
  }, [measure, syncRailView, recompute])

  /* 锚点集合变化（新消息到达）后立即重算，不等 rAF——激活轮要跟手。 */
  useLayoutEffect(() => {
    if (scrollRef.current) {
      measure()
      recompute()
    }
  }, [measure, recompute, scrollRef])

  useEffect(() => {
    // 窗口尺寸变化：锚点偏移与轨自身视口都要重算（后者喂渐隐阈值）
    const onResize = () => {
      measure()
      syncRailView()
      recompute()
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [measure, syncRailView, recompute])

  /* ── 滚动 → 激活轮（rAF 合帧，滚动回调里只做算术不读 DOM） ── */
  useEffect(() => {
    const container = scrollRef.current
    if (!container) return
    let frame = 0
    const onScroll = () => {
      if (frame) return
      frame = requestAnimationFrame(() => {
        frame = 0
        recompute()
      })
    }
    container.addEventListener('scroll', onScroll, { passive: true })
    onScroll()
    return () => {
      container.removeEventListener('scroll', onScroll)
      if (frame) cancelAnimationFrame(frame)
    }
  }, [recompute, scrollRef])

  /* ── 刻度梯内部滚动：记录偏移供两端渐隐判断 ── */
  const handleRailScroll = useCallback(() => {
    syncRailView()
  }, [syncRailView])

  /* ── 激活轮跟进：指针不在轨上时才把激活刻度滚进可见区 ── */
  useEffect(() => {
    const el = viewportRef.current
    if (!el || activeTurn === null || pointerInsideRef.current) return
    const index = anchors.findIndex(anchor => anchor.turn === activeTurn)
    if (index < 0) return
    const content = railContentHeight(anchors.length)
    const view = el.clientHeight
    if (view <= 0) return
    const target = Math.max(
      0,
      Math.min(Math.max(0, content - view), index * TURN_PITCH_PX + RAIL_INSET_PX - view / 2),
    )
    const previous = followRef.current
    const structural = previous?.count !== anchors.length || previous?.height !== view
    followRef.current = { count: anchors.length, height: view }
    if (Math.abs(el.scrollTop - target) <= 0.5) return
    el.scrollTo({ top: target, behavior: structural ? 'auto' : scrollBehavior() })
  }, [activeTurn, anchors])

  /* ── 跳转：滚到该轮首条消息（瞬移或平滑由动效偏好决定） ── */
  const jumpTo = useCallback(
    (messageId: string) => {
      const container = scrollRef.current
      const el = document.getElementById(`turn-anchor-${messageId}`)
      if (!container || !el) return
      el.scrollIntoView({ behavior: scrollBehavior(), block: 'start' })
      // scrollIntoView 之后布局可能仍在合帧中，补一次实测保持激活轮准确
      requestAnimationFrame(() => measure())
    },
    [measure, scrollRef],
  )

  if (anchors.length < 2) return null

  const content = railContentHeight(anchors.length)
  const canScroll = content > railView && railView > 0
  const previewAnchor =
    previewTurn === null ? undefined : anchors.find(anchor => anchor.turn === previewTurn)
  const previewIndex =
    previewAnchor === undefined
      ? -1
      : anchors.findIndex(anchor => anchor.turn === previewAnchor.turn)

  return (
    <nav
      className="turn-rail"
      aria-label={t('chat.turnRail.label')}
      onPointerEnter={() => {
        pointerInsideRef.current = true
      }}
      onPointerLeave={() => {
        pointerInsideRef.current = false
        setPreviewTurn(null)
      }}
    >
      <div className="turn-rail-viewport" ref={viewportRef} onScroll={handleRailScroll}>
        {/* 轨道必须带确定高度：刻度全部绝对定位，不给定高度轨道会塌成 0，
            整个模块随之高度为 0、刻度从锚定点向下溢出（模块底部跑出定位线）。 */}
        <div className="turn-rail-track" style={{ height: content }}>
          {anchors.map((anchor, index) => {
            const isActive = anchor.turn === activeTurn
            const isPreview = anchor.turn === previewTurn
            const classes = ['turn-rail-mark']
            if (isActive) classes.push('is-active')
            else if (isPreview) classes.push('is-preview')
            return (
              <button
                key={anchor.messageId}
                type="button"
                className={classes.join(' ')}
                style={{ top: RAIL_INSET_PX + index * TURN_PITCH_PX }}
                aria-label={t('chat.turnRail.jump', String(anchor.turn))}
                aria-current={isActive ? 'true' : undefined}
                aria-describedby={isPreview ? previewId : undefined}
                onPointerMove={() => setPreviewTurn(anchor.turn)}
                onFocus={() => setPreviewTurn(anchor.turn)}
                onBlur={() => setPreviewTurn(null)}
                onClick={() => jumpTo(anchor.messageId)}
              >
                <span className="turn-rail-tick" />
              </button>
            )
          })}
        </div>
        {canScroll && railOffset > 1 && <div className="turn-rail-fade-top" />}
        {canScroll && railOffset < content - railView - 1 && (
          <div className="turn-rail-fade-bottom" />
        )}
      </div>
      {previewAnchor !== undefined && (
        <div
          id={previewId}
          role="tooltip"
          className="turn-rail-preview"
          style={
            {
              '--turn-preview-center': `${previewIndex * TURN_PITCH_PX + RAIL_INSET_PX + MARK_HEIGHT_PX / 2 - railOffset}px`,
            } as CSSProperties
          }
        >
          <div className="turn-rail-preview-prompt">
            {previewAnchor.prompt || t('chat.turnRail.turn', String(previewAnchor.turn))}
          </div>
          {previewAnchor.response !== '' && (
            <div className="turn-rail-preview-response">{previewAnchor.response}</div>
          )}
        </div>
      )}
    </nav>
  )
}
