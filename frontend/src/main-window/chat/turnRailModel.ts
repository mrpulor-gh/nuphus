/**
 * turnRailModel —— 对话轮次轨的纯逻辑层（无 DOM、无副作用）。
 *
 * 职责：
 * - 把消息流切成「轮次」：每条 user 消息开启一轮，后续 assistant 内容归入该轮；
 *   refine 段分隔与 system 不计轮次（不产生锚点）。
 * - 从滚动位置推断「当前激活轮」：**触底时激活最新一轮**（发送后的即时反馈），
 *   否则以视口上 1/3 处为判定线，取最后一条起点在判定线之上的锚点。
 *
 * 为什么单独成文件：切轮与激活判定都是可单测的纯函数，UI（TurnRail.tsx）
 * 只消费这里的结果，便于回归钉死语义。
 */

/** 轮次锚点：轨上一枚刻度对应一轮对话。 */
export interface TurnAnchor {
  /** 轮次序号，从 1 起（仅展示与跳转用） */
  turn: number
  /** 该轮首条 user 消息的 id —— DOM 滚动定位用 */
  messageId: string
  /** 轮次提问预览（截断） */
  prompt: string
  /** 轮次答复预览（截断） */
  response: string
}

/** 模型只依赖消息的三个字段，不耦合完整 ChatMessage。 */
export interface TurnSourceMessage {
  id: string
  role: string
  content: string
}

/** 提问预览上限（字符）—— 气泡一行放得下的长度。 */
const PROMPT_PREVIEW_MAX = 120

/** 答复预览上限（字符）—— 气泡三行放得下的长度。 */
const RESPONSE_PREVIEW_MAX = 240

/** 判定线在视口中的相对位置：从上往下 1/3 处。 */
export const ACTIVE_LINE_RATIO = 0.35

/** 触底判定的容差（px）：流式输出让内容持续增高，钉底时 scrollTop 追赶有帧差。 */
export const BOTTOM_EPSILON_PX = 4

/**
 * 依滚动位置推断激活轮次号。
 *
 * 两种口径，按滚动位置二选一：
 * - **触底**（`atBottom`）：激活**最新一轮**。发送新消息后对话钉在底部，
 *   此时判定线口径会停在上一轮（新锚点还在判定线之下），而用户眼里
 *   「刚发出去的那轮」就是当前轮——轨上的 is-active 必须跟上；
 * - 否则（用户在翻历史）：以视口上 1/3 处为判定线，取最后一条起点在判定线
 *   之上的锚点；无线索时回退到第一轮。
 *
 * @param anchors   轮次锚点（升序）
 * @param tops      锚点实测偏移；缺实测的锚点按「不可见」跳过
 * @param scrollTop 滚动容器当前 scrollTop
 * @param viewportHeight 滚动容器视口高
 * @param atBottom 滚动容器是否已（近似）触底
 * @returns 激活轮次号；无锚点时 null
 */
export function pickActiveTurn(
  anchors: readonly TurnAnchor[],
  tops: AnchorTops,
  scrollTop: number,
  viewportHeight: number,
  atBottom?: boolean,
): number | null {
  if (anchors.length === 0) return null
  if (atBottom) return anchors[anchors.length - 1].turn
  const line = scrollTop + viewportHeight * ACTIVE_LINE_RATIO
  let active = anchors[0].turn
  for (const anchor of anchors) {
    const top = tops.get(anchor.messageId)
    if (top === undefined) continue
    if (top <= line) active = anchor.turn
  }
  return active
}

/** 滚动容器是否（近似）触底。 */
export function isAtBottom(
  scrollTop: number,
  viewportHeight: number,
  scrollHeight: number,
): boolean {
  return scrollTop + viewportHeight >= scrollHeight - BOTTOM_EPSILON_PX
}

/** 相邻刻度的固定节距（px）—— 与 TurnRail.module.css 的同名常量保持一致。 */
export const TURN_PITCH_PX = 10

/** 轨首尾留白（px）—— 与 TurnRail.module.css 的同名常量保持一致。 */
export const RAIL_INSET_PX = 6

/** 两端渐隐带宽度（px）—— 与 TurnRail.module.css 的同名常量保持一致。 */
export const FADE_BAND_PX = 20

/** 截断预览文本：去首尾空白后按字符数封顶。 */
export function boundPreview(text: string, max: number): string {
  const trimmed = text.trim()
  if (trimmed.length <= max) return trimmed
  return `${trimmed.slice(0, max)}…`
}

/**
 * 把消息流切成轮次锚点（按出现序，turn 从 1 递增）。
 *
 * 规则：
 * - `user` 开启新轮（锚点 messageId = 该消息 id）；
 * - `assistant` 内容追加到当前轮的答复预览（无当前轮时丢弃——
 *   会话以助手消息开头属于异常态，不造空轮）；
 * - `refine` / `system` 不产生锚点、也不归入答复；
 * - 空 user 消息仍占一轮（用户可能只发了附件）。
 */
export function buildTurnAnchors(messages: readonly TurnSourceMessage[]): TurnAnchor[] {
  const anchors: TurnAnchor[] = []
  for (const message of messages) {
    if (message.role === 'user') {
      anchors.push({
        turn: anchors.length + 1,
        messageId: message.id,
        prompt: boundPreview(message.content, PROMPT_PREVIEW_MAX),
        response: '',
      })
      continue
    }
    if (message.role !== 'assistant') continue
    const current = anchors[anchors.length - 1]
    if (current === undefined) continue
    const text = message.content.trim()
    if (!text) continue
    current.response = current.response ? `${current.response}\n\n${text}` : text
  }
  for (const anchor of anchors) {
    anchor.response = boundPreview(anchor.response, RESPONSE_PREVIEW_MAX)
  }
  return anchors
}

/** 锚点元素相对滚动内容顶部的偏移（messageId → px）。 */
export type AnchorTops = ReadonlyMap<string, number>
