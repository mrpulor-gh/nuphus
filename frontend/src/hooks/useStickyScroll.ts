// useStickyScroll — 消息流「贴底跟随」滚动语义（ChatPanel 的 .chat-messages、执行追踪
// 面板的步骤树容器共用）
//
// 背景：旧实现在 `useEffect(..., [messages])` 里无条件 scrollTo(scrollHeight)，流式每来
// 一个 delta 都会触发一次，用户一旦上翻读历史就被反复拽回底部。本 hook 把语义收敛为：
//   1. 贴底跟随：followKey 变化（新消息 / 流式 delta / 执行步骤更新）且处于跟随态
//      → smooth 滚底；
//   2. 上翻冻结：向上滚动（delta < -2px）= 查看历史 → 冻结跟随 + 续命；
//   3. 向下滚动（delta > +2px）= 朝底部回走：不冻结、不续命 —— 用户在往回走、不是
//      读历史；且中途恢复等于把用户弹回底部（只能上滚不能下滚），只有滚回底部才恢复；
//   4. 恢复路径：
//      a. atBottom（80px 容差内）：用户自己滚回底部 / 向下或静止滚动贴底 / 近底微滚
//         （容差内不判离开，防抖动误冻结）—— 立即恢复 + 按钮隐藏；
//      b. 静默宽限满 resumeMs —— 语义是「连续无上滚操作」的上限而非读死表：冻结态
//         下的向上 scroll 都清旧建新重置计时，用户在读就永不恢复；且仅 executing=true
//         才排计时，executing=false（空闲翻看）不排 —— 永久冻结直到 followReset /
//         用户滚回底部；
//      c. 程序调 followReset（新轮次 execution_started / 任务完成瞬间补拉）。
//
// 进场自动滚底（执行追踪面板 open 专用）：enterPanel 先 followReset 立即滚底展示
// 最新执行态；在**这次自动滚底完成之前**，scroll 事件不参与判定 —— 但绝不吞用户
// 输入：用户一伸手（向上 delta）立即交还判定权并按其意图冻结；滚到（容差内）即
// 释放。没有固定时长宽限（旧实现是 3s 硬窗：窗内用户的滚动一律被丢，等于抢控制）。
// 对话窗无「进入」语义，不调用本方法。
//
// 实现要点（成败点）：hook 自己发起的 smooth 滚动会连续触发 scroll 事件，中途态 scrollTop
// 并未到底，若 onScroll 无脑判定会把「程序滚动」误判成「用户上拉」⇒ 冻结 + 按钮闪现。
// 故程序滚动前置 400ms 屏蔽窗（programScrollRef）：窗内向下 / 静止的 delta 一律忽略；
// 向上的 delta 只可能来自用户操作（抢滚动条 / 滚轮反向），立即退出屏蔽态并按用户分支
// 处理 —— 不吞用户输入。
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'

/** 距底容差：80px 内视为贴底（与 mobile MessageList 同一阈值，跨端语义一致） */
const BOTTOM_TOLERANCE_PX = 80
/** 恢复静默宽限缺省值：连续无上滚操作 15s 后恢复跟随（对话窗传 60_000、执行面板传 15_000） */
const DEFAULT_RESUME_MS = 15_000
/** 程序滚动屏蔽窗：smooth 动画通常 <400ms；超窗即使未滚完也不再拦截（见 onScroll） */
const PROGRAM_SCROLL_GUARD_MS = 400
/**
 * 进场自动滚底的安全网上限（ms）——**不是宽限，是释放兜底**：
 * smooth 动画正常 <400ms 内到底并释放；只在「浏览器不派发 scroll 事件 / 内容突变
 * 导致永远滚不到底」的异常下，到点强行交还判定权，避免 pending 态把之后所有 scroll
 * 全吞掉。选 1000ms 而非旧 3000ms：它越短，用户越早拿回控制。
 */
const ENTRY_SCROLL_MAX_WAIT_MS = 1_000
/** scrollTop 方向判定阈值：|delta| <= 2px 视为静止，抹掉亚像素 / 布局抖动 */
const SCROLL_DELTA_EPS_PX = 2

export interface StickyScroll {
  /** 绑到滚动容器（ChatPanel 的 .chat-messages / 执行面板的步骤树容器） */
  scrollRef: RefObject<HTMLDivElement>
  /** 是否显示「回到底部」按钮：已冻结跟随且未回底时为 true */
  showJumpButton: boolean
  /**
   * 绑到滚动容器的 onScroll：识别用户滚动，程序滚动自动豁免。
   *
   * 返回值 = 这一帧**是否按「用户操作」处理**（false = 被进场/程序滚动屏蔽窗吞掉，
   * 或只是静止 / 亚像素抖动）。调用方若在 onScroll 之外还有自己的滚动语义
   * （执行追踪面板的"上滚到折叠条续展"），**必须**用这个返回值 gate 住自己那份判定：
   * 否则程序滚动（进场自动下拉、贴底回滚、折叠锚点补偿）产生的 delta 会被当成用户
   * 意图 → 上翻冻结被悄悄解掉、面板被顶离底部（2026-10-09 实测事故）。
   */
  onScroll: () => boolean
  /** 点击回底按钮：立即恢复跟随并 smooth 滚底 */
  jumpToBottom: () => void
  /** 立即恢复跟随并 smooth 滚底：新轮次 execution_started / 任务完成瞬间补拉（程序调用方） */
  followReset: () => void
  /**
   * 进入面板（执行追踪面板 open / 可见态变化时调用）：先 followReset 立即滚底 —
   * 进场展示最新执行态是默认预期；随后只等**这次自动滚底完成**就交还判定权
   * （到底 / 用户伸手接手 / 安全网到点，见 onScroll），没有固定时长宽限。
   * 对话窗无「进入」语义，不调用本方法。
   */
  enterPanel: () => void
  /**
   * 程序性调整 scrollTop（**保持当前视野锚点**）：内容在视野上方增删时，用它把
   * 视野钉回原处。必须走本 hook 的程序滚动屏蔽窗——裸改 scrollTop 产生的 delta
   * 会被方向判定读成"用户在往底部回走"，把上翻冻结解掉并触发回底。
   */
  nudgeScrollTop: (deltaPx: number) => void
}

export interface StickyScrollOptions {
  /**
   * 无滚动操作的宽限上限（ms）：到点且仍在冻结态才恢复跟随。
   * 对话窗 60_000 / 执行追踪面板 15_000；缺省 15_000（保持原值）。
   */
  resumeMs?: number
  /**
   * 仅执行态读秒：false 时上翻冻结后不排恢复计时（空闲翻看不打扰，永久冻结直到
   * followReset / 用户滚回底部）；true（缺省，维持旧行为）才排 resumeMs 宽限。
   */
  executing?: boolean
}

export function useStickyScroll(followKey: unknown, opts?: StickyScrollOptions): StickyScroll {
  const { resumeMs = DEFAULT_RESUME_MS, executing = true } = opts ?? {}
  const scrollRef = useRef<HTMLDivElement>(null)
  /** true = 贴底跟随中；false = 用户上翻后冻结（按钮显示） */
  const [following, setFollowing] = useState(true)
  /** following 的 ref 镜像：effect / handler 闭包里读最新值，又不想把 state 拖进 deps */
  const followingRef = useRef(true)
  const showJumpButton = !following
  /** 最近一次 scrollTop：屏蔽窗内做方向判定用 */
  const lastScrollTopRef = useRef<number | null>(null)
  /** true = 正处于 hook 自己发起的 smooth 滚动中 */
  const programScrollRef = useRef(false)
  const programTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** true = enterPanel 发起的自动滚底尚未完成：期间 scroll 不参与判定（见 onScroll） */
  const entryScrollPendingRef = useRef(false)
  /** 上一项的安全网截止时间戳（ms）：到点强行交还判定权，防异常卡死（见 enterPanel） */
  const entryMaxWaitUntilRef = useRef(0)
  const resumeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  /** 切换跟随态：state 与 ref 同步置位（单一入口，防两处漂移） */
  const setFollowingMode = useCallback((next: boolean) => {
    followingRef.current = next
    setFollowing(next)
  }, [])

  const clearProgramGuard = useCallback(() => {
    if (programTimerRef.current) {
      clearTimeout(programTimerRef.current)
      programTimerRef.current = null
    }
    programScrollRef.current = false
  }, [])

  const clearResumeTimer = useCallback(() => {
    if (resumeTimerRef.current) {
      clearTimeout(resumeTimerRef.current)
      resumeTimerRef.current = null
    }
  }, [])

  /** 装甲程序滚动屏蔽窗（原理见文件头）：窗内向下 / 静止 delta 一律忽略 */
  const armProgramGuard = useCallback(() => {
    programScrollRef.current = true
    if (programTimerRef.current) clearTimeout(programTimerRef.current)
    programTimerRef.current = setTimeout(() => {
      programTimerRef.current = null
      programScrollRef.current = false
    }, PROGRAM_SCROLL_GUARD_MS)
  }, [])

  /** 程序滚动到底：smooth + 前置屏蔽窗（原理见文件头） */
  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    armProgramGuard()
    lastScrollTopRef.current = el.scrollTop
    // rAF 不能丢：同步调用会量到尚未布局的高度（沿用旧实现的取舍）
    requestAnimationFrame(() => {
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' })
    })
  }, [armProgramGuard])

  /**
   * 冻结 + 排恢复计时（滚动驱动的宽限，主判定，不是读死表）：
   * - executing=true：清旧建新排 resumeMs —— 冻结态下的向上 scroll（离开底部后的
   *   上滚，即 onScroll 判为「查看历史」的滚动）都走这里，计时随之清零；用户持续
   *   读就永不恢复，停手满 resumeMs 才 setFollowingMode(true) + 滚底（恢复路径之一，
   *   见文件头 4b；向下 / 静止滚动不冻结不续命，不经过这里）；
   * - executing=false：只清旧、不排新 —— 空闲期翻看历史不排恢复计时（避免把正在读
   *   历史的用户闪回底部），永久冻结直到 followReset / 用户自己滚回底部。
   */
  const freezeWithResumeTimer = useCallback(() => {
    if (resumeTimerRef.current) clearTimeout(resumeTimerRef.current)
    resumeTimerRef.current = null
    if (!executing) return
    resumeTimerRef.current = setTimeout(() => {
      resumeTimerRef.current = null
      // 静默宽限满：视为用户已离开阅读，主动滚回底部（与「默认自动下拉」语义一致）
      setFollowingMode(true)
      scrollToBottom()
    }, resumeMs)
  }, [executing, resumeMs, scrollToBottom, setFollowingMode])

  // followKey 变化（新消息 / 流式 delta / 执行步骤更新）：仅跟随态滚底，冻结态不拽回
  useEffect(() => {
    if (!followingRef.current) return
    scrollToBottom()
  }, [followKey, scrollToBottom])

  // 卸载清理：恢复计时与屏蔽窗都由 hook 自管，消费方无感
  useEffect(() => {
    return () => {
      if (resumeTimerRef.current) clearTimeout(resumeTimerRef.current)
      if (programTimerRef.current) clearTimeout(programTimerRef.current)
    }
  }, [])

  /** 返回值语义见 StickyScroll.onScroll：true = 这一帧按「用户操作」处理 */
  const onScroll = useCallback((): boolean => {
    const el = scrollRef.current
    if (!el) return false
    const current = el.scrollTop
    const last = lastScrollTopRef.current
    lastScrollTopRef.current = current

    // 进场自动滚底进行中（enterPanel 发起的那次 smooth 滚底）：判定权暂不参与，
    // 但**不吞用户输入**。三条出路，谁先来算谁：
    //   · 已到底（80px 容差）= 自动下拉完成 → 释放，回归正常判定（此刻本就是贴底态）；
    //   · 向上 delta = 用户伸手抢控制 → 立即释放并照常走用户分支（冻结 + 宽限重排），
    //     绝不等任何固定时长；
    //   · 向下 / 静止 = smooth 动画中间态 → 忽略；安全网到点（异常不派发 scroll 事件 /
    //     永远滚不到底）也释放，之后的所有 scroll 恢复判定。
    // lastScrollTopRef 仍随每个事件持续刷新：释放后的首个事件才能拿到干净的方向增量。
    if (entryScrollPendingRef.current) {
      const arrived = el.scrollHeight - el.scrollTop - el.clientHeight <= BOTTOM_TOLERANCE_PX
      const userPulledUp = last !== null && current - last < -SCROLL_DELTA_EPS_PX
      if (arrived || userPulledUp || Date.now() >= entryMaxWaitUntilRef.current) {
        entryScrollPendingRef.current = false
        // 自动下拉自己滚到底、且用户没伸手抢：这一帧属程序滚动，不算用户操作
        if (arrived && !userPulledUp) return false
      } else {
        return false
      }
    }

    if (programScrollRef.current) {
      // 程序滚动中：向下 / 静止都是 smooth 动画的中间态，一律忽略；向上 delta 只可能
      // 来自用户操作，立即退出屏蔽态改走用户分支（不吞用户输入）
      if (last !== null && current - last < -SCROLL_DELTA_EPS_PX) {
        clearProgramGuard()
      } else {
        return false
      }
    }

    // atBottom 兜底（先于方向判定）：贴底（80px 容差内）即恢复跟随、按钮隐藏 —— 覆盖
    // 用户自己滚回底部、向下 / 静止滚动贴底，以及近底微滚（容差内不判离开，防布局
    // 抖动把贴底态误冻结）
    if (el.scrollHeight - el.scrollTop - el.clientHeight <= BOTTOM_TOLERANCE_PX) {
      if (!followingRef.current) {
        clearResumeTimer()
        setFollowingMode(true)
      }
      // 只有"用户往下滚到底"算用户操作；静止 / 被新内容顶到底（delta≈0）不算
      return current - (last ?? current) > SCROLL_DELTA_EPS_PX
    }

    // 离开底部后再判方向，方向即意图：
    // 向上（< -2px）= 查看历史 → 冻结 + 重置宽限计时（用户在读就永不恢复）；
    // 向下（> +2px）= 朝底部回走 → 不冻结、不续命：用户在往回走、不是读历史，且
    //   中途恢复等于把用户弹回底部（只能上滚不能下滚）—— 回到上面的 atBottom 才算回底；
    // |delta| <= 2px = 静止 / 亚像素抖动 → 忽略（atBottom 已兜底贴底态）。
    const delta = current - (last ?? current)
    if (delta < -SCROLL_DELTA_EPS_PX) {
      if (followingRef.current) {
        setFollowingMode(false)
      }
      freezeWithResumeTimer()
      return true
    }
    // 向下 = 用户主动朝底部回走（是用户操作，但按语义不冻结、不续命）
    return delta > SCROLL_DELTA_EPS_PX
  }, [clearProgramGuard, clearResumeTimer, freezeWithResumeTimer, setFollowingMode])

  /**
   * 立即恢复跟随并 smooth 滚底 —— 恢复路径之三（程序调用方）：
   * 新轮次 execution_started（恢复跟随后续流式）/ 任务完成瞬间补拉（下拉展示成果）/
   * 面板 terminal/card 模式切换。与回底按钮 jumpToBottom 同语义，同一实现两个名字：
   * 前者是事件驱动，后者是点击驱动，谁也别复制谁的逻辑。
   */
  const followReset = useCallback(() => {
    clearResumeTimer()
    setFollowingMode(true)
    scrollToBottom()
  }, [clearResumeTimer, scrollToBottom, setFollowingMode])

  /**
   * 程序性调整 scrollTop：把当前视野锚回原处（内容在视野**上方**增删时）。
   *
   * 必须走屏蔽窗：裸改 scrollTop 产生的「向下 delta」会被方向判定读成"用户在往底部
   * 回走"→ 上翻冻结被解掉、followKey 一来即回底，用户正在读的位置被顶走
   * （2026-10-09 实测事故：折叠续展与自动下拉互相打架）。
   */
  const nudgeScrollTop = useCallback(
    (deltaPx: number) => {
      const el = scrollRef.current
      if (!el || !deltaPx) return
      // 先装甲再改 scrollTop：这一帧产生的 scroll 事件必须被判为程序滚动
      armProgramGuard()
      lastScrollTopRef.current = el.scrollTop + deltaPx
      // 同步落地：调用方（折叠锚点补偿）已在 layoutEffect 里量到高度差，此时布局已定；
      // 多套一层 rAF 只会让用户看到一次额外的跳动。
      el.scrollTop = el.scrollTop + deltaPx
    },
    [armProgramGuard],
  )

  /**
   * 进入面板（执行追踪面板 open / 可见态变化时调用）：先 followReset 立即滚底 —
   * 进场展示最新执行态是默认预期；随后只等**这次自动滚底完成**（到底 / 用户接手 /
   * 安全网到点，见 onScroll 的三条出路），没有固定时长宽限。
   * 无参：宽限不该由调用方给时长——给了就一定会被当成「抢控制的许可」。
   */
  const enterPanel = useCallback(() => {
    const el = scrollRef.current
    // 已在底部（内容不足一屏 / 本就贴底）= 没有「下拉」可等：不置等待态，
    // 否则永不释放，之后所有 scroll 都被吞。
    const atBottom = !el || el.scrollHeight - el.scrollTop - el.clientHeight <= BOTTOM_TOLERANCE_PX
    entryScrollPendingRef.current = !atBottom
    entryMaxWaitUntilRef.current = Date.now() + ENTRY_SCROLL_MAX_WAIT_MS
    followReset()
  }, [followReset])

  return {
    scrollRef,
    showJumpButton,
    onScroll,
    jumpToBottom: followReset,
    followReset,
    enterPanel,
    nudgeScrollTop,
  }
}
