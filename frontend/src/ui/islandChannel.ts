/**
 * islandChannel.ts — 应用内 island 轻反馈通道：焦点分流 + 队列编排
 *
 * 改造前：所有轻反馈只有一条出路 —— `invoke('hud_update')`，即桌面右下角的独立
 * HUD 窗口。应用在前台时视线就在窗口内，反馈却甩到屏幕角落；而且 HUD 是
 * 「追加覆盖」式呈现，前一条还没读完就被下一条顶掉。
 *
 * 本模块是**唯一的**分流与编排实现（`useInit.showToast` 只调 showAppFeedback 一个入口）：
 *   ① 分流：应用窗口在前台 → 应用内 island；不在前台 / 最小化 → 仍走 hud_update
 *      （HUD 是独立 always-on-top 窗口，后台提醒是它不可替代的场景，保留）
 *   ② 编排：island 的提示**队列串行**（一条退场后才进下一条），绝不堆叠
 *
 * 职责边界：本文件只管「状态 + 时机 + 落点锚点解析」，呈现与动效在 AppIsland.tsx /
 * AppPill.tsx + styles/app-*.css。定时器只驱动**生命周期**（何时进退场、何时放下一条），
 * 位移/呼吸/淡出全部是 CSS 动画 —— 不出现 JS 改 inline style 做反馈。
 */
import { useCallback } from 'react'
import { invoke } from '../core/bridge'

/** 相位：与 useInit 的 `Toast['type']` 同构 */
export type AppFeedback = 'info' | 'success' | 'warning' | 'error'

export interface AppIslandToast {
  id: string
  message: string
  type: AppFeedback
}

/** 组件订阅的快照；引用保持稳定，仅在真正变化时替换（供 useSyncExternalStore） */
export interface AppIslandSnapshot {
  /** 当前展示的提示；null = 无提示 */
  toast: AppIslandToast | null
  /** true = 正在播退场动画：下一条要等它退完才进场（串行，不堆叠） */
  exiting: boolean
}

/** 停留时长：info/success 3.2s；warning/error 5s（错误留更久读完） */
export const ISLAND_DWELL_MS: Record<AppFeedback, number> = {
  info: 3200,
  success: 3200,
  warning: 5000,
  error: 5000,
}

/** 退场动画时长，与 app-island.css 的退出 keyframes 时长（--dur-med）对齐 */
export const ISLAND_EXIT_MS = 200

/**
 * 「活跃态」判定：只有 info 属于进行中/等待中的持续反馈（如「已请求优雅停止」），
 * 呼吸动效传达「这件事还在走」；success / warning / error 都是一次性结果，
 * 只做一次进场 + 停留 + 淡出，不持续脉动。
 */
export function isActiveFeedback(type: AppFeedback): boolean {
  return type === 'info'
}

// ── 焦点跟踪（分流依据）────────────────────────────────────────────
/** 跟踪器就绪前保持 false → 走 HUD，与改造前行为一致 */
let appFocused = false

export function isAppFocused(): boolean {
  return appFocused
}

/** 由焦点跟踪器写入（window focus/blur + Tauri onFocusChanged） */
export function setAppFocused(next: boolean): void {
  appFocused = next
}

/**
 * 启动焦点跟踪，返回清理函数。
 *
 * 同步初值取 `document.hasFocus()`（WebView2 里随窗口焦点同步变化）；
 * 再订阅 Tauri 的权威焦点事件——最小化、被其它应用完全盖住时，
 * `window` 的 blur 未必按预期触发，不能只靠 document。
 */
export function startAppFocusTracking(): () => void {
  let disposed = false
  const cleanups: Array<() => void> = []

  setAppFocused(document.hasFocus())

  const onFocus = () => setAppFocused(true)
  const onBlur = () => setAppFocused(false)
  window.addEventListener('focus', onFocus)
  window.addEventListener('blur', onBlur)
  cleanups.push(() => {
    window.removeEventListener('focus', onFocus)
    window.removeEventListener('blur', onBlur)
  })

  // 浏览器调试环境（vite dev / vitest）没有 Tauri 运行时：document 焦点兜底
  if ('__TAURI_INTERNALS__' in window) {
    void (async () => {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window')
        const win = getCurrentWindow()
        const unlisten = await win.onFocusChanged(({ payload }) => setAppFocused(payload))
        if (disposed) {
          unlisten()
          return
        }
        cleanups.push(unlisten)
        setAppFocused(await win.isFocused())
      } catch {
        // 焦点 API 不可用：保留 document 焦点兜底
      }
    })()
  }

  return () => {
    disposed = true
    cleanups.forEach(fn => fn())
  }
}

// ── 队列与生命周期 ───────────────────────────────────────────────
let current: AppIslandToast | null = null
let exiting = false
let pending: AppIslandToast[] = []
let dwellTimer: ReturnType<typeof setTimeout> | null = null
/** 剩余停顿时长（hover 暂停时按已耗时扣减，供恢复用） */
let dwellRemainingMs = 0
let dwellStartedAt = 0
let dwellPaused = false

let snapshot: AppIslandSnapshot = { toast: null, exiting: false }
const listeners = new Set<() => void>()

function publish(): void {
  if (snapshot.toast === current && snapshot.exiting === exiting) return
  snapshot = { toast: current, exiting }
  listeners.forEach(listener => listener())
}

export function subscribeAppIsland(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function getAppIslandSnapshot(): AppIslandSnapshot {
  return snapshot
}

function clearDwellTimer(): void {
  if (dwellTimer !== null) {
    clearTimeout(dwellTimer)
    dwellTimer = null
  }
}

function startDwell(ms: number): void {
  clearDwellTimer()
  dwellPaused = false
  dwellRemainingMs = ms
  dwellStartedAt = Date.now()
  dwellTimer = setTimeout(beginExit, ms)
}

/** 停留结束 / 用户点击关闭 → 进退场阶段（退完才轮到下一条） */
function beginExit(): void {
  clearDwellTimer()
  if (current === null || exiting) return
  exiting = true
  publish()
  setTimeout(advance, ISLAND_EXIT_MS)
}

/** 退场结束 → 出队下一条（串行的收敛点：这里一次只放一条进来） */
function advance(): void {
  const next = pending.shift() ?? null
  current = next
  exiting = false
  publish()
  if (next) startDwell(ISLAND_DWELL_MS[next.type])
}

function enqueue(message: string, type: AppFeedback): void {
  const toast: AppIslandToast = { id: crypto.randomUUID(), message, type }
  if (current !== null) {
    pending.push(toast)
    return
  }
  current = toast
  publish()
  startDwell(ISLAND_DWELL_MS[type])
}

/** hover 暂停自动消失计时（让用户把话读完） */
export function pauseIslandDwell(): void {
  if (current === null || exiting || dwellTimer === null) return
  dwellPaused = true
  dwellRemainingMs = Math.max(0, dwellRemainingMs - (Date.now() - dwellStartedAt))
  clearDwellTimer()
}

/** 移开 → 按剩余时长继续计时 */
export function resumeIslandDwell(): void {
  if (current === null || exiting || !dwellPaused) return
  startDwell(dwellRemainingMs)
}

/** 点击即刻关闭（进退场，退完进下一条） */
export function dismissIslandToast(): void {
  beginExit()
}

// ── 分流入口 ─────────────────────────────────────────────────────
/** 相位 → HUD phase（与改造前 useInit 内的 phaseMap 一致） */
const HUD_PHASE: Record<AppFeedback, string> = {
  info: 'info',
  success: 'success',
  warning: 'warning',
  error: 'error',
}

/**
 * 轻反馈唯一分流入口（`useInit.showToast` 的唯一实现）。
 *
 * 应用窗口在前台 → 应用内 island；不在前台 / 最小化 → HUD 独立窗口
 * （那种场景下窗口内的 island 用户根本看不见）。
 */
export function showAppFeedback(message: string, type: AppFeedback = 'info'): void {
  if (isAppFocused()) {
    enqueue(message, type)
    return
  }
  sendToHud(message, HUD_PHASE[type])
}

/** 后台/最小化时的出路：HUD 是独立 always-on-top 窗口，岛在那种场景下看不见 */
function sendToHud(message: string, phase: string): void {
  invoke('hud_update', { text: message, phase }).catch(e =>
    console.warn('[Toast] hud_update failed:', e),
  )
}

// ── 收编入口：兼收 HUD 相位词汇 ─────────────────────────────────────
/**
 * HUD 相位 → island 相位。
 *
 * 历史直调点是「直接 invoke('hud_update')」，带的是 HUD 的相位字面量；其中
 * `done`（执行完成）island 没有 —— 岛只表达 info/success/warning/error 四种语义。
 * 翻译**只此一份**：收编点保留原相位字面量（可回溯改造前语义），歧义在这里收敛。
 */
const HUD_PHASE_TO_FEEDBACK: Record<string, AppFeedback> = {
  info: 'info',
  success: 'success',
  // HUD 的「执行完成」是一次性结果 → 成功态
  done: 'success',
  warning: 'warning',
  error: 'error',
}

/**
 * 收编入口：以 HUD 相位词汇调轻反馈。
 *
 * 调用方：`main-window/lib/api.ts` 的 `hudUpdate` 封装、useEvents 的「执行完成」。
 * 前台 → island（相位按映射表翻译成岛的四种语义）；后台/最小化 → 仍走 HUD，
 * 且**相位原样透传**（HUD 认识自己的全部相位：done 在 HUD 里停 15s，比 success
 * 更久，不能顺手抹平）。未知相位 → island 侧落 info：不猜语义，也不静默丢弃。
 */
export function showAppFeedbackByHudPhase(message: string, hudPhase: string): void {
  if (!isAppFocused()) {
    sendToHud(message, hudPhase)
    return
  }
  enqueue(message, HUD_PHASE_TO_FEEDBACK[hudPhase] ?? 'info')
}

// ── 落点锚点：宿主标题栏 > 聊天区 header > 全局回落 ───────────────────
/**
 * island 的落点锚点解析。
 *
 * 候选按**可见性**排序（不是简单的"第一个存在的"）：
 *   ① 全屏宿主锚点（设置中心 / 模型页标题栏的 `.island-slot`，随宿主打开/关闭注册）
 *      —— 宿主是 `fixed inset:0 + z-index 2500`，会把聊天区整个盖住；此时聊天区锚点
 *      虽在 DOM 里却不可见，必须让位给宿主锚点，否则提示会被静默压住。
 *      同时打开多个宿主时取**最后注册**（= 最后打开）的那个，与 App 层宿主 DOM 顺序一致。
 *   ② 聊天区 `.chat-header` 中央锚点（随聊天视图挂载）
 *   ③ 都没有 → null：AppIsland 原地渲染，回落窗口级顶部居中
 *      —— 任何时刻都有落点，不会出现「入队了但没人渲染」。
 *
 * 为什么用模块级共享状态而不是 Context：AppIsland 与 ChatPanel / 各宿主是互不嵌套的
 * 子树（岛常驻 App 根部），锚点必须是「谁先挂载都成立」的共享状态；用
 * useSyncExternalStore 订阅，锚点出现/消失时岛原地改挂载点。
 */
let chatAnchor: HTMLElement | null = null
/** 全屏宿主锚点：按打开顺序入栈（后打开的在最上层） */
const hostAnchors: Array<{ id: string; el: HTMLElement }> = []
const anchorListeners = new Set<() => void>()

function publishAnchors(): void {
  anchorListeners.forEach(listener => listener())
}

export function subscribeIslandAnchor(listener: () => void): () => void {
  anchorListeners.add(listener)
  return () => {
    anchorListeners.delete(listener)
  }
}

/** 当前应挂载到的锚点（宿主 > 聊天 header > null 表示回落全局定位） */
export function getIslandAnchor(): HTMLElement | null {
  return hostAnchors[hostAnchors.length - 1]?.el ?? chatAnchor
}

/** 聊天区 header 中央锚点（ChatPanel 的 `.island-slot`，随聊天视图挂载/卸载） */
export function setIslandAnchor(next: HTMLElement | null): void {
  if (chatAnchor === next) return
  chatAnchor = next
  publishAnchors()
}

/**
 * 全屏宿主锚点注册（宿主标题栏的 `.island-slot`）。
 * 宿主打开时传元素，关闭时传 null（React 的 ref 回调会以 null 通知卸载）。
 */
export function setIslandHostAnchor(id: string, next: HTMLElement | null): void {
  const idx = hostAnchors.findIndex(entry => entry.id === id)
  if (next === null) {
    if (idx < 0) return
    hostAnchors.splice(idx, 1)
  } else {
    // 已存在（同一宿主重渲染）时先摘再压栈：栈顶恒为最后打开/最后挂载的宿主
    if (idx >= 0) hostAnchors.splice(idx, 1)
    hostAnchors.push({ id, el: next })
  }
  publishAnchors()
}

/** 宿主锚点 ref 的稳定身份：内联箭头函数会让 ref 每次渲染摘挂，锚点栈反复抖动 */
export function useIslandHostAnchor(id: string): (el: HTMLElement | null) => void {
  return useCallback((el: HTMLElement | null) => setIslandHostAnchor(id, el), [id])
}
