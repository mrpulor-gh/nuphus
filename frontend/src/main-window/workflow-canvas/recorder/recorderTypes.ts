/**
 * recorderTypes.ts — 工作流录制（RecorderBar / 意图面板 / step_draft）共享类型
 *
 * 对应设计意图 §四数据模型与 rec 后端契约：
 * - step_draft 结构对齐 §4.1（action/intent/params/exception_note/evidence）
 * - rec_complete 仅序列化 steps 数组落盘 record-draft.{ts}.json
 */

import type { WorkflowStep } from '../../../core/types'

/** 录制状态机：off=无会话；idle=会话 active 待命；capturing=捕获中；pending=意图面板 */
export type RecStatus = 'off' | 'idle' | 'capturing' | 'pending'

/** 10 种录制动作（设计意图 §三 + browser 动作组） */
export type RecAction =
  | 'click'
  | 'scroll'
  | 'text'
  | 'hotkey'
  | 'sleep'
  | 'region'
  | 'find_image'
  // ── browser 动作组：网页流程（目标 = Nuphus managed 浏览器窗口）──
  | 'browser_navigate'
  | 'browser_click'
  | 'browser_extract'

/** 动作捕获通道：hook=桌面低层 hook（真实桌面事件）；overlay=全屏框选；
 *  cdp=CDP 注入捕获（浏览器页面真实点击）；none=无捕获（纯填写意图/参数） */
export type RecCaptureChannel = 'hook' | 'overlay' | 'cdp' | 'none'

/** 动作 → 捕获通道（pick 路由依据；动作自带通道，用户无感） */
export const ACTION_CHANNEL: Record<RecAction, RecCaptureChannel> = {
  click: 'hook',
  scroll: 'hook',
  text: 'hook',
  hotkey: 'hook',
  sleep: 'none',
  region: 'overlay',
  find_image: 'overlay',
  browser_navigate: 'none',
  browser_click: 'cdp',
  browser_extract: 'none',
}

export const ACTION_LABEL: Record<RecAction, string> = {
  click: '桌面点击',
  scroll: '滚动',
  text: '输入文本',
  hotkey: '按键组合',
  sleep: '等待延时',
  region: '区域框选坐标',
  find_image: '模板锚点',
  browser_navigate: '打开网址',
  browser_click: '网页点击',
  browser_extract: '网页内容获取',
}

/** rec_start 返回的 CaptureEvent（serde 直序列化，坐标 = 屏幕绝对物理像素） */
export interface CaptureEvent {
  kind: 'click' | 'double_click' | 'scroll' | 'hotkey'
  button: 'left' | 'right' | 'middle' | null
  x: number
  y: number
  wheel_delta: number | null
  keys: string[]
  window_title: string
  hwnd: number
  /** 前台窗口所属进程 PID（0 = 无前台/不可用；标题不可靠时记录真实归属） */
  pid: number
  /** 进程可执行文件名（如 "explorer.exe"）；获取失败或无前台为 null */
  process_name: string | null
  ts_ms: number
}

/** rec_set_workflow / rec_session_status 返回 */
export interface RecSessionInfo {
  status: 'idle' | 'active'
  workflow_id?: string | null
  workflow_dir?: string | null
  screenshots_dir?: string | null
  started_at_ms?: number | null
}

/** overlay_capture_confirm / take_capture_result 的录制 ROI 结果 */
export interface RecOverlayCapture {
  path: string
  rect: RecRect
  base64: string | null
}

export interface RecRect {
  x: number
  y: number
  width: number
  height: number
}

/** 待确认步骤（进入 pending 的数据） */
export interface RecPending {
  action: RecAction
  /** 显示用步骤序号（1-based，便于面板标题） */
  stepNo: number
  /** hook 类动作捕获事件（click/scroll/text/hotkey） */
  event?: CaptureEvent
  /** overlay 类动作 ROI 结果（region/find_image） */
  overlay?: RecOverlayCapture
  /** browser_click 捕获到的网页元素（CDP 注入捕获；selector 为当前页唯一 CSS selector） */
  browserCapture?: {
    selector: string
    tag: string
    text: string
    href?: string | null
    /** 捕获时页面 URL（draft 上下文；导航后 selector 语义可能变化，WorkflowAgent 据此判断） */
    url?: string
  }
}

/** 意图面板确认产物（§4.1 step_draft 结构 + 画布可执行体） */
export interface RecDraft {
  action: string
  intent: string
  params: Record<string, unknown>
  exception_note?: string | null
  evidence: {
    screenshot?: string | null
    rect?: RecRect | null
  }
  /** 绑定画布节点 id（confirmPending 入画布后记录；删除草稿时联动 remove_step；恢复进度时保留） */
  canvas_step_id?: string
  /** 追加：画布可执行体（供 WorkflowAgent 反查 IR 映射，不破坏 §4.1 兼容） */
  canvas?: {
    kind: 'tool' | 'sleep' | 'chat'
    tool?: string
    with?: Record<string, unknown>
    seconds?: number
    /** 节点 capture 变量名（find_image 后点击场景：find 节点捕获输出供后续点击引用） */
    capture?: string
  }
}

/** 输入文本步骤的目标：窗口输入（默认，基础参数生效）/ chatagent（生成 chat 步骤，基础参数可空） */
export type TextTarget = 'window' | 'chatagent'

/** 模板锚点/区域步骤「找图后」动作：无动作 / 单击 / 双击 */
export type FindClickAfter = 'none' | 'click' | 'double_click'

/** 意图面板表单 payload */
export interface IntentPayload {
  intent: string
  exceptionNote: string
  variable: boolean
  text?: string
  send?: string
  seconds?: number
  /** text 动作输入目标（仅 text 生效；默认 window） */
  target?: TextTarget
  /** find_image/region 找图后点击（默认 none = 不额外生成点击） */
  clickAfter?: FindClickAfter
  /** browser_navigate 目标 URL（必填，面板校验） */
  url?: string
  /** browser_extract 获取内容说明（默认「获取当前页面主要内容」） */
  contentNote?: string
}

/** 生成画布节点的最小可执行体（一个 pending 动作可展开为多条；每条对应一个画布节点 + 一条 draft） */
export interface BuiltStep {
  kind: 'tool' | 'sleep' | 'chat'
  /** 画布节点名 = 用户填写的意图（截断在 CanvasPage 侧统一处理） */
  name: string
  do: WorkflowStep['do']
  /** 节点 capture 变量名（find_image 步骤捕获匹配输出，供后续点击引用） */
  capture?: string
  draft: RecDraft
}
