// useStickyScroll — 消息流「贴底跟随」滚动语义（ChatPanel 的 .chat-messages、执行追踪
// 面板的步骤树 / 终端容器共用）
//
// 设计原则（2026-10-09 重写）：**只有用户手势能改变跟随状态，内容增长永远不能。**
//
// 旧实现靠「读滚动条位置」反推用户意图（80px 容差 + 15s/60s 静默宽限），在极高流式
// 输出下必然失效：内容被顶高 → 滚动条离底 → 被读成「用户上翻」→ 冻结；内容又把滚动条
// 顶回容差内 → 被读成「用户到底」→ 误恢复跟随。二者互相争抢，用户上滚都按不住，
// 自动下拉也无法在流式下稳定跟住。新实现改为直接监听手势事件，跟随态只有手势一个入口：
//
//   1. 贴底跟随：followKey 变化（新消息 / 流式 delta / 执行步骤更新）且 followRef 为
//      true → 瞬移到底（直接赋值 scrollTop）。**不用 smooth**：smooth 动画逐帧也会改
//      scrollTop，在流式高频增长下必然与内容增长打架（「相互争抢」的一半根因）。
//   2. 手势 → 状态（唯一的状态改写入口）：
//      · wheel deltaY < 0（鼠标上滚）→ 立即 follow=false —— 同步改写，不等 scroll 事件、
//        不等任何计时器、不等防抖（B2）；
//      · pointerdown 落在滚动条槽上 → 立即 follow=false，拖拽期间不跟随（B3）；
//      · keydown ArrowUp / PageUp / Home → follow=false；
//      · 手势结束后（滚轮 / 键盘：末次手势后 GESTURE_END_DEBOUNCE_MS 无新手势；指针：
//        pointerup）按落点 isAtBottom(el) 决定是否回归 follow=true（B4）。
//   3. isAtBottom(el)（≤ AT_BOTTOM_EPS_PX）**只回答「手势结束时落点在哪」**，绝不作为
//      自动跟随的触发 / 保持条件（B1）。它与旧 80px 容差的本质区别：那个是滚动事件
//      驱动的状态判定，这个只是手势收尾时的落点查询。
//   4. 程序滚动按「次」精确对冲：写入 scrollTop 前置一次待吞账，紧随的这一次 scroll
//      事件被判为程序回响（B1 / B5 的另一半），而不是靠固定时长窗赌（固定窗既会漏，
//      又会误吞窗口内用户的真实滚动 = 抢控制）。
//
// 「内容增长」为何不会再被读成用户意图：容器启用 overflow-anchor: none 后，流式撑高只
// 改变 scrollHeight、不改 scrollTop（浏览器不会为内容增长派发 scroll 事件）；且即便
// 派发了 scroll，本实现的跟随态也完全由手势标志决定，不读「距底距离 / 百分比」。
//
// 与旧「静默宽限」的区别：GESTURE_END_DEBOUNCE_MS 是**手势去抖**（判定一次连续手势何时
// 收尾，只影响「何时按落点回归」，绝不主动把用户拽回底部）；旧的 resumeMs 是「静默 N 秒
// 后无条件恢复跟随」，正是「用户读着被拽回底部」的来源（B6），已整段删除。
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'

/**
 * 贴底判据容差（px）：亚像素 / 缩放抖动余量。**只**用于「用户手势结束后落点在哪」的
 * 查询 —— 绝不作为自动跟随的触发 / 保持条件（那是旧 BOTTOM_TOLERANCE_PX=80 的病根）。
 */
const AT_BOTTOM_EPS_PX = 4

/**
 * 滚轮 / 键盘手势收尾去抖（ms）：末次手势后静默满此刻 = 一次连续手势结束，此时才按
 * 落点决定是否回归跟随。注意这是**手势去抖**，与旧 resumeMs（静默 N 秒后无条件恢复
 * 跟随）性质完全不同：它不会在没有用户手势的情况下改写跟随态。
 */
const GESTURE_END_DEBOUNCE_MS = 120

/**
 * 「未经对冲的 scrollTop 下降」判定阈值（px）：作为方向噪声地板，抹掉亚像素抖动，
 * 避免把一次浮点舍入读成用户上滚。它是**方向**阈值，不是位置容差。
 */
const UNGUARDED_UNLOCK_EPS_PX = 1

/**
 * 内容收缩钳位判定阈值（px）：气泡变矮时（草稿被 progress 替换 / think 块被剥离 / 折叠）
 * 浏览器会把越界的 scrollTop 钳回底部 —— 这个「恰好落回底部」的下降**不是**用户上滚。
 * 误判会让自动下拉在流中途静默失锁且再也回不来（0.2.27 实机复现）。
 */
const CLAMP_TO_BOTTOM_EPS_PX = 2

/**
 * 贴底区宽容语义（对话窗启用；不传 = 严格语义，执行追踪面板沿用）。
 */
export interface StickyScrollOptions {
  /**
   * 贴底区半径（px）。> 0 时启用对话窗语义：
   *   ① 用户**向下**滚回区内（滚轮 / 键盘 / 拖拽滚动条，含松手瞬间）→ 立即恢复跟随；
   *   ② 「恰好落回底部」的 scrollTop 下降识别为内容收缩钳位，不再误判成用户上滚；
   *   ③ 手势收尾只在「方向向下且落在区内」时恢复，绝不把正在阅读的用户拽回底部。
   *
   * 为什么需要：严格版用 4px + 手势收尾双条件判定恢复，在流式增长下几乎不可达 ——
   * 用户滚回底部松手时内容已又长高一截，判定落空后跟随永久丢失（0.2.27 实机复现）。
   * 内容增长本身依旧不恢复跟随（向下滚动是唯一恢复入口），上滚依旧立即解锁。
   */
  bottomZonePx?: number
}

export interface StickyScroll {
  /** 绑到滚动容器（ChatPanel 的 .chat-messages / 执行面板的步骤树 / 终端容器） */
  scrollRef: RefObject<HTMLDivElement>
  /** 是否显示「回到底部」按钮：跟随态被用户手势解锁时（未贴底）为 true */
  showJumpButton: boolean
  /**
   * 绑到滚动容器的 onScroll：识别帧来源，程序滚动自动豁免。
   *
   * 返回值 = 这一帧**是否按「用户操作」处理**（false = 被程序回响对冲吞掉，或只是静止 /
   * 亚像素抖动）。调用方若在 onScroll 之外还有自己的滚动语义（执行追踪面板的"上滚到
   * 折叠条续展"），**必须**用这个返回值 gate 住自己那份判定：否则程序滚动（进场自动
   * 下拉、贴底回滚、折叠锚点补偿）产生的 delta 会被当成用户意图 → 上翻冻结被悄悄解掉、
   * 面板被顶离底部（2026-10-09 实测事故）。
   */
  onScroll: () => boolean
  /** 点击回底按钮：立即恢复跟随并瞬移到底 */
  jumpToBottom: () => void
  /** 立即恢复跟随并瞬移到底：新轮次 execution_started / 任务完成瞬间补拉 / 模式切换（程序调用方） */
  followReset: () => void
  /**
   * 进入面板（执行追踪面板 open / 可见态变化时调用）：语义已简化为「follow=true +
   * 立即贴底」——进场展示最新执行态是默认预期。旧版「进场自动下拉等待态」状态机已删：
   * 它靠吞 scroll 事件争取时间，等于短期抢控制；新语义下用户任何手势都能当场接管，
   * 无需等待态。与 followReset 同一实现，保留独立名字是为了调用方语义可读。
   */
  enterPanel: () => void
  /**
   * 程序性调整 scrollTop（**保持当前视野锚点**）：内容在视野上方增删时，用它把视野钉回
   * 原处。必须走本 hook 的程序滚动对冲 —— 裸改 scrollTop 产生的 delta 会被读成用户的
   * 方向操作，把上翻解锁后的冻结态解掉并触发回底。
   */
  nudgeScrollTop: (deltaPx: number) => void
}

export function useStickyScroll(followKey: unknown, options?: StickyScrollOptions): StickyScroll {
  const scrollRef = useRef<HTMLDivElement>(null)
  /** true = 贴底跟随中；false = 被用户手势解锁（按钮显示） */
  const [following, setFollowing] = useState(true)
  /** following 的 ref 镜像：效果 / 手势回调里读最新值，又不想把 state 拖进 deps */
  const followingRef = useRef(true)
  const showJumpButton = !following

  /** 最近一次已知 scrollTop：方向判定用（程序写入时同步记下目标值） */
  const lastScrollTopRef = useRef<number | null>(null)
  /**
   * 程序写入 scrollTop 后「待吞」的 scroll 事件数：写入前置 1，紧随的这一次 scroll 事件
   * 被判为程序回响并清账。为什么按「次」而不是按固定时长窗：写入与它自己的回响事件是
   * 一对一（同帧内多次写入会被浏览器合并成一次 scroll 事件，故置 1 即可）；固定时长窗
   * 要么太短（窗后才到的中间态被误判为用户上翻）要么太长（窗内用户的真实滚动被吞掉）。
   */
  const programGuardRef = useRef(0)
  /** true = 滚动条槽拖拽中（pointerdown 落在槽上，pointerup / pointercancel 结束） */
  const draggingRef = useRef(false)
  /** true = 指针在容器内容区按下：期间出现的非程序 scroll 事件归因用户手势 */
  const pointerActiveRef = useRef(false)
  /** true = 滚轮 / 键盘手势去抖窗内：期间出现的 scroll 事件归因用户手势 */
  const gestureActiveRef = useRef(false)
  const gestureEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** 最近一次用户滚动方向：手势收尾只在「向下」时恢复跟随（不会把阅读中的用户拽回底部） */
  const lastUserDirRef = useRef<'up' | 'down' | null>(null)
  /** 贴底区半径：0 = 严格语义（默认）；> 0 = 对话窗宽容语义（见 StickyScrollOptions） */
  const zonePx = options?.bottomZonePx ?? 0

  /** 切换跟随态：state 与 ref 同步置位（单一入口，防两处漂移） */
  const setFollowingMode = useCallback((next: boolean) => {
    followingRef.current = next
    setFollowing(next)
  }, [])

  /** 贴底落点查询：只回答「此刻是否贴底」，绝不触发任何跟随状态变更 */
  const isAtBottom = useCallback((el: HTMLDivElement): boolean => {
    return el.scrollHeight - el.scrollTop - el.clientHeight <= AT_BOTTOM_EPS_PX
  }, [])

  /**
   * 「在贴底区内」查询：严格版 = 4px 贴底；宽容版（对话窗）= 落进 bottomZonePx 半径。
   * 宽容版只扩大**恢复跟随**的落点判定，不新增解锁入口（解锁仍只由明确的上滚手势负责）。
   */
  const inBottomZone = useCallback(
    (el: HTMLDivElement): boolean =>
      zonePx > 0 ? el.scrollHeight - el.scrollTop - el.clientHeight <= zonePx : isAtBottom(el),
    [isAtBottom, zonePx],
  )

  /** 用户手势优先：清掉可能残留的程序待吞账，绝不让它吞掉用户的 scroll 事件 */
  const clearProgramGuard = useCallback(() => {
    programGuardRef.current = 0
  }, [])

  const clearGestureEndTimer = useCallback(() => {
    if (gestureEndTimerRef.current) {
      clearTimeout(gestureEndTimerRef.current)
      gestureEndTimerRef.current = null
    }
  }, [])

  /**
   * 手势收尾去抖：排定「手势结束」时刻，到点**只**按当前落点决定是否回归跟随
   * （贴底 → 跟随；离开底部 → 保持解锁）。拖拽由 pointerup 收尾，不排此计时。
   */
  const scheduleGestureEnd = useCallback(() => {
    gestureActiveRef.current = true
    clearGestureEndTimer()
    gestureEndTimerRef.current = setTimeout(() => {
      gestureEndTimerRef.current = null
      gestureActiveRef.current = false
      if (draggingRef.current) return
      const el = scrollRef.current
      if (!el) return
      if (zonePx > 0) {
        // 对话窗：只在「这一轮手势是往下滚、且落点在贴底区内」时恢复。**绝不在收尾时解锁** ——
        // 严格版在此无条件按 4px 改写跟随态，向下滚 + 流式增长会把它误锁死（自动下拉丢失）。
        if (lastUserDirRef.current === 'down' && inBottomZone(el)) setFollowingMode(true)
        return
      }
      setFollowingMode(isAtBottom(el))
    }, GESTURE_END_DEBOUNCE_MS)
  }, [clearGestureEndTimer, inBottomZone, isAtBottom, setFollowingMode, zonePx])

  /**
   * 程序写入 scrollTop 的唯一出口：先记目标值（供方向判定拿到干净基准）、再置 1 次
   * 「待吞」，最后直接赋值。目标与现值相同时完全不动 —— 避免装甲后没有回响事件、
   * 把下一次用户的 scroll 误吞。
   */
  const writeScrollTop = useCallback((el: HTMLDivElement, next: number) => {
    if (el.scrollTop === next) return
    programGuardRef.current = 1
    lastScrollTopRef.current = next
    el.scrollTop = next
  }, [])

  /**
   * 瞬移贴底：同步落到本帧，并在下一帧再量一次 scrollHeight 补一次 —— 流式仍在增长时
   * 本帧量到的高度可能已过时，补一次能抹掉「增长 → 贴底 → 又增长」之间的空隙残留。
   * 两次写入都在跟随态下才做（用户中途抢控制则放弃补写）。
   */
  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    writeScrollTop(el, Math.max(0, el.scrollHeight - el.clientHeight))
    requestAnimationFrame(() => {
      if (!followingRef.current) return
      const next = scrollRef.current
      if (!next) return
      writeScrollTop(next, Math.max(0, next.scrollHeight - next.clientHeight))
    })
  }, [writeScrollTop])

  // ── 手势监听：挂在 window 捕获阶段 + 按 target 归属过滤 ──
  // 不挂在元素上：消费方的 scrollRef 是稳定对象、容器元素可能延迟出现（执行面板先
  // return null 后再打开），挂元素需要「渲染后探测元素是否换了」；window 捕获 +
  // el.contains(target) 只在挂载时注册一次，容器何时出现都能命中，且多实例互不干扰
  // （各自只处理自己容器内的手势）。
  const handleWheel = useCallback(
    (e: WheelEvent) => {
      const el = scrollRef.current
      if (!el || !el.contains(e.target as Node)) return
      clearProgramGuard()
      if (e.deltaY < 0) {
        // B2：鼠标上滚 → 立即解锁（同步改写，不等 scroll 事件、不等去抖、不等计时器）
        lastUserDirRef.current = 'up'
        setFollowingMode(false)
        scheduleGestureEnd()
      } else if (e.deltaY > 0) {
        lastUserDirRef.current = 'down'
        // 向下滚不当场恢复：中途恢复等于把用户弹回底部。等手势收尾后按落点决定（B4）。
        // 例外（对话窗）：已在贴底区内继续往下滚 —— 其实滚无可滚、不会派发 scroll 帧，
        // 这个手势的本意只能是「回到跟随」，当场恢复。
        if (zonePx > 0 && !followingRef.current && inBottomZone(el)) setFollowingMode(true)
        scheduleGestureEnd()
      }
    },
    [clearProgramGuard, inBottomZone, scheduleGestureEnd, setFollowingMode, zonePx],
  )

  const handlePointerDown = useCallback(
    (e: PointerEvent) => {
      const el = scrollRef.current
      if (!el || !el.contains(e.target as Node)) return
      clearProgramGuard()
      pointerActiveRef.current = true
      // 原生滚动条槽在内容盒之外（clientWidth 不含滚动条宽度）：按下点越过它即判拖拽。
      // 用视口坐标差而非 offsetX —— offsetX 相对事件 target，可能落在子元素上。
      const rect = el.getBoundingClientRect()
      const overGutter = e.clientX - rect.left >= el.clientWidth
      if (overGutter) {
        draggingRef.current = true
        // B3：拖拽开始即解锁（拖拽期间不跟随）
        setFollowingMode(false)
      }
    },
    [clearProgramGuard, setFollowingMode],
  )

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const el = scrollRef.current
      if (!el || !el.contains(e.target as Node)) return
      clearProgramGuard()
      if (e.key === 'ArrowUp' || e.key === 'PageUp' || e.key === 'Home') {
        lastUserDirRef.current = 'up'
        setFollowingMode(false)
        scheduleGestureEnd()
      } else if (e.key === 'ArrowDown' || e.key === 'PageDown' || e.key === 'End') {
        lastUserDirRef.current = 'down'
        // 同滚轮向下：手势收尾后按落点决定（End 落到底 → 回归）；已在贴底区内继续下按
        // （滚无可滚、无 scroll 帧）→ 对话窗当场恢复。
        if (zonePx > 0 && !followingRef.current && inBottomZone(el)) setFollowingMode(true)
        scheduleGestureEnd()
      }
    },
    [clearProgramGuard, inBottomZone, scheduleGestureEnd, setFollowingMode, zonePx],
  )

  const handlePointerUp = useCallback(() => {
    const el = scrollRef.current
    draggingRef.current = false
    pointerActiveRef.current = false
    // B4：用户操作后释放指针且停在底部 → 回归自动下拉。注意这里**只回归、不解锁**
    // （解锁只由明确的上滚手势触发），所以它不会把正在阅读的用户拽回底部。
    if (!el || followingRef.current) return
    // 对话窗：松手时落点在贴底区内、且本轮指针移动不是「向上」（自己拖回底部 / 拖到底
    // 松手）→ 恢复。严格版仍是 4px 贴底判定。
    if (zonePx > 0 ? lastUserDirRef.current === 'down' && inBottomZone(el) : isAtBottom(el)) {
      setFollowingMode(true)
    }
  }, [inBottomZone, isAtBottom, setFollowingMode, zonePx])

  useEffect(() => {
    const capture = { capture: true } as const
    const capturePassive = { capture: true, passive: true } as const
    window.addEventListener('wheel', handleWheel, capturePassive)
    window.addEventListener('pointerdown', handlePointerDown, capture)
    window.addEventListener('keydown', handleKeyDown, capture)
    window.addEventListener('pointerup', handlePointerUp, capture)
    window.addEventListener('pointercancel', handlePointerUp, capture)
    return () => {
      window.removeEventListener('wheel', handleWheel, capturePassive)
      window.removeEventListener('pointerdown', handlePointerDown, capture)
      window.removeEventListener('keydown', handleKeyDown, capture)
      window.removeEventListener('pointerup', handlePointerUp, capture)
      window.removeEventListener('pointercancel', handlePointerUp, capture)
      clearGestureEndTimer()
    }
  }, [handleWheel, handlePointerDown, handleKeyDown, handlePointerUp, clearGestureEndTimer])

  // followKey 变化（新消息 / 流式 delta / 执行步骤更新）：仅跟随态贴底，解锁态不拽回
  useEffect(() => {
    if (!followingRef.current) return
    scrollToBottom()
  }, [followKey, scrollToBottom])

  /** 返回值语义见 StickyScroll.onScroll：true = 这一帧按「用户操作」处理 */
  const onScroll = useCallback((): boolean => {
    const el = scrollRef.current
    if (!el) return false
    const current = el.scrollTop
    const last = lastScrollTopRef.current
    lastScrollTopRef.current = current

    // 程序写入自己的回响：吞掉紧随的这一次 scroll 事件，不参与任何判定
    if (programGuardRef.current > 0) {
      programGuardRef.current -= 1
      return false
    }

    const delta = current - (last ?? current)
    const fromUserGesture =
      draggingRef.current || pointerActiveRef.current || gestureActiveRef.current

    // 收缩钳位：内容变矮时浏览器把越界的 scrollTop 钳回底部 —— 落点**恰好**在底
    // （≤2px）、且没有任何用户手势。它不是「用户上滚」（严格版 / 用户拖回区内不误判：
    // 拖回只会在区内、不会精准停在 2px 内；对话窗识别并放行，防止自动下拉静默失锁）。
    const clampedByShrink =
      zonePx > 0 &&
      !fromUserGesture &&
      el.scrollHeight - el.scrollTop - el.clientHeight <= CLAMP_TO_BOTTOM_EPS_PX

    if (delta > UNGUARDED_UNLOCK_EPS_PX) {
      lastUserDirRef.current = 'down'
    } else if (delta < -UNGUARDED_UNLOCK_EPS_PX && !clampedByShrink) {
      lastUserDirRef.current = 'up'
    }

    // 未经对冲的 scrollTop 下降只可能来自用户：内容增长只改 scrollHeight、不改
    // scrollTop（overflow-anchor: none 已关掉浏览器的反向锚定），程序写入已在上一步
    // 对冲。这也是「原生滚动条拖拽不派发 pointer 事件」时的兜底识别手段 —— 拖拽期间
    // 到达的 scroll 事件一律归因用户意图（B3），绝不因此回退成位置百分比判定。
    if (delta < -UNGUARDED_UNLOCK_EPS_PX && followingRef.current && !clampedByShrink) {
      setFollowingMode(false)
    }

    // 对话窗恢复入口：用户**向下**滚回贴底区 → 立即恢复跟随。落点进区即恢复，
    // 不必压线到 4px、不必等手势收尾 —— 后两个条件在流式增长下几乎不可达，是
    // 「自动下拉没了」的主因。内容增长本身不产生向下 delta，不会误开（B1/B5 不变）。
    if (
      zonePx > 0 &&
      !followingRef.current &&
      delta > UNGUARDED_UNLOCK_EPS_PX &&
      inBottomZone(el)
    ) {
      setFollowingMode(true)
    }

    return fromUserGesture || delta !== 0
  }, [inBottomZone, setFollowingMode, zonePx])

  /**
   * 立即恢复跟随并瞬移到底 —— 程序调用方出口（新轮次 / 任务完成补拉 / 模式切换 /
   * 回底按钮 / 进场）。同一实现三个名字：jumpToBottom（点击）、followReset（事件）、
   * enterPanel（进场），谁也别复制谁的逻辑。
   */
  const followReset = useCallback(() => {
    setFollowingMode(true)
    scrollToBottom()
  }, [scrollToBottom, setFollowingMode])

  /**
   * 程序性调整 scrollTop：把当前视野锚回原处（内容在视野**上方**增删时）。
   *
   * 必须走 writeScrollTop：裸改 scrollTop 产生的「向下 delta」会被读成用户在往底部
   * 回走，把上翻解锁后的冻结态解掉、followKey 一来即回底，用户正在读的位置被顶走
   * （2026-10-09 实测事故：折叠续展与自动下拉互相打架）。
   * 同步落地：调用方（折叠锚点补偿）已在 layoutEffect 里量到高度差，此时布局已定。
   */
  const nudgeScrollTop = useCallback(
    (deltaPx: number) => {
      const el = scrollRef.current
      if (!el || !deltaPx) return
      writeScrollTop(el, el.scrollTop + deltaPx)
    },
    [writeScrollTop],
  )

  return {
    scrollRef,
    showJumpButton,
    onScroll,
    jumpToBottom: followReset,
    followReset,
    enterPanel: followReset,
    nudgeScrollTop,
  }
}
