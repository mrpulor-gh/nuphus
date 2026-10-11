/**
 * turnRailModel 单测 —— 钉死切轮规则与激活轮推断语义。
 */
import { describe, expect, it } from 'vitest'
import {
  ACTIVE_LINE_RATIO,
  buildTurnAnchors,
  boundPreview,
  isAtBottom,
  pickActiveTurn,
  type TurnSourceMessage,
} from './turnRailModel'

const msg = (id: string, role: string, content = ''): TurnSourceMessage => ({
  id,
  role,
  content,
})

describe('buildTurnAnchors', () => {
  it('每条 user 开启一轮，assistant 归入当前轮', () => {
    const anchors = buildTurnAnchors([
      msg('u1', 'user', '问题一'),
      msg('a1', 'assistant', '答复一'),
      msg('u2', 'user', '问题二'),
      msg('a2', 'assistant', '答复二'),
    ])
    expect(anchors).toEqual([
      { turn: 1, messageId: 'u1', prompt: '问题一', response: '答复一' },
      { turn: 2, messageId: 'u2', prompt: '问题二', response: '答复二' },
    ])
  })

  it('多段 assistant 内容拼接进同一轮', () => {
    const anchors = buildTurnAnchors([
      msg('u1', 'user', '问'),
      msg('a1', 'assistant', '第一段'),
      msg('a2', 'assistant', '第二段'),
      msg('a3', 'assistant', '   '), // 空段忽略
    ])
    expect(anchors).toHaveLength(1)
    expect(anchors[0].response).toBe('第一段\n\n第二段')
  })

  it('refine 段分隔与 system 不计轮、不归答复', () => {
    const anchors = buildTurnAnchors([
      msg('s0', 'system', '系统提示'),
      msg('r1', 'refine', '提炼'),
      msg('u1', 'user', '问'),
      msg('s1', 'system', '又一条系统消息'),
      msg('a1', 'assistant', '答'),
    ])
    expect(anchors).toEqual([{ turn: 1, messageId: 'u1', prompt: '问', response: '答' }])
  })

  it('开头的 assistant 不造空轮', () => {
    const anchors = buildTurnAnchors([msg('a0', 'assistant', '欢迎语')])
    expect(anchors).toEqual([])
  })

  it('空 user 消息仍占一轮（可能只发了附件）', () => {
    const anchors = buildTurnAnchors([msg('u1', 'user', '')])
    expect(anchors).toHaveLength(1)
    expect(anchors[0].prompt).toBe('')
  })

  it('预览文本按上限截断', () => {
    expect(boundPreview('  短文本  ', 120)).toBe('短文本')
    expect(boundPreview('x'.repeat(200), 120)).toBe(`${'x'.repeat(120)}…`)
    const anchors = buildTurnAnchors([
      msg('u1', 'user', 'y'.repeat(300)),
      msg('a1', 'assistant', 'z'.repeat(500)),
    ])
    expect(anchors[0].prompt).toHaveLength(121) // 120 + 省略号
    expect(anchors[0].response).toHaveLength(241)
  })
})

describe('pickActiveTurn', () => {
  const anchors = buildTurnAnchors([
    msg('u1', 'user', '一'),
    msg('u2', 'user', '二'),
    msg('u3', 'user', '三'),
  ])
  const tops = new Map([
    ['u1', 0],
    ['u2', 1000],
    ['u3', 2000],
  ])
  const viewport = 600

  it('判定线以上最后一轮为激活轮', () => {
    // line = 100 + 600*0.35 = 310 → u1 在上 → 第 1 轮
    expect(pickActiveTurn(anchors, tops, 100, viewport)).toBe(1)
    // line = 900 + 210 = 1110 → u2 在上 → 第 2 轮
    expect(pickActiveTurn(anchors, tops, 900, viewport)).toBe(2)
    // line = 5000 + 210 → u3 在上 → 第 3 轮
    expect(pickActiveTurn(anchors, tops, 5000, viewport)).toBe(3)
  })

  it('判定线还未到第一轮时回退第 1 轮', () => {
    expect(pickActiveTurn(anchors, tops, 0, viewport)).toBe(1)
  })

  it('缺实测偏移的锚点不参与判定', () => {
    const partial = new Map([
      ['u1', 0],
      ['u3', 2000],
    ])
    expect(pickActiveTurn(anchors, partial, 2500, viewport)).toBe(3)
  })

  it('无锚点返回 null', () => {
    expect(pickActiveTurn([], tops, 100, viewport)).toBeNull()
  })

  it('触底时激活最新一轮（发送新消息后 is-active 必须跟上新刻度）', () => {
    // 判定线口径在 scrollTop=1900 时 line=2110，u3（2000）在上 → 本来也是 3；
    // 但发送瞬间 scrollTop 尚未追赶（u3 在判定线之下）时，判定线口径会停在 2。
    expect(pickActiveTurn(anchors, tops, 900, viewport, true)).toBe(3)
    expect(pickActiveTurn(anchors, tops, 0, viewport, true)).toBe(3)
  })

  it('触底但翻历史（未触底）不被最新轮抢占', () => {
    expect(pickActiveTurn(anchors, tops, 900, viewport, false)).toBe(2)
  })

  it('判定线比例导出为常量（UI 同源消费）', () => {
    expect(ACTIVE_LINE_RATIO).toBe(0.35)
  })
})

describe('isAtBottom', () => {
  it('贴底（含流式增长的帧差容差）为 true', () => {
    expect(isAtBottom(400, 600, 1000)).toBe(true)
    // 差 3px（< BOTTOM_EPSILON_PX）仍算贴底：流式输出钉底追赶有帧差
    expect(isAtBottom(397, 600, 1000)).toBe(true)
  })

  it('离开底部超过容差为 false', () => {
    expect(isAtBottom(390, 600, 1000)).toBe(false)
    expect(isAtBottom(0, 600, 5000)).toBe(false)
  })

  it('内容短于视口（无需滚动）也算贴底', () => {
    expect(isAtBottom(0, 600, 400)).toBe(true)
  })
})
