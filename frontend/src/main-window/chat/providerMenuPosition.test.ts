/**
 * 输入框 Models 弹窗 → 模型列表浮层定位算法
 *
 * 规则：
 *  - 以「models 弹窗」为基准（不是 window）
 *  - hover 的行在弹窗**上半部** → 浮层顶部与该行顶部对齐（top）
 *  - hover 的行在弹窗**下半部** → 浮层底部与该行底部对齐（bottom）
 *  - 浮层最大高度 = 弹窗高度一半（**上限**，非定高；实际高度随内容自适应）
 *
 * 复刻 ChatPanel.openProviderModels 的纵向计算。
 */
import { describe, expect, it } from 'vitest'

type Rect = { top: number; bottom: number; height: number }
type Pos = { top?: number; bottom?: number; maxHeight: number }
type Vp = { height: number }

function computePos(rect: Rect, modal: Rect, viewport: Vp): Pos {
  const boundTop = modal.top
  const boundBottom = modal.bottom
  const boundHeight = boundBottom - boundTop
  const maxHeight = Math.max(120, Math.floor(boundHeight / 2))

  const modalMid = boundTop + boundHeight / 2
  const alignTop = rect.top + rect.height / 2 <= modalMid

  if (alignTop) return { top: rect.top, maxHeight }
  return { bottom: viewport.height - rect.bottom, maxHeight }
}

describe('models 弹窗：模型列表浮层定位', () => {
  const modal: Rect = { top: 100, bottom: 700, height: 600 }
  const viewport: Vp = { height: 900 }

  it('maxHeight 是弹窗高度的一半（上限）', () => {
    const p = computePos({ top: 150, bottom: 196, height: 46 }, modal, viewport)
    expect(p.maxHeight).toBe(300)
  })

  it('上半部 → top 对齐，且不使用 bottom', () => {
    const p = computePos({ top: 150, bottom: 196, height: 46 }, modal, viewport)
    expect(p.top).toBe(150)
    expect(p.bottom).toBeUndefined() // top/bottom 不可同时出现
  })

  it('下半部 → bottom 对齐，且不使用 top', () => {
    const row = { top: 640, bottom: 686, height: 46 }
    const p = computePos(row, modal, viewport)
    expect(p.top).toBeUndefined()
    // bottom = 视口高 - 行底；浮层底边因此恒贴行底边
    expect(p.bottom).toBe(viewport.height - row.bottom)
    expect(viewport.height - (p.bottom as number)).toBe(row.bottom)
  })

  it('下半部用 bottom 而非 top-减-上限：内容短时不会悬空', () => {
    // 若按旧写法 top = row.bottom - maxHeight：
    const row = { top: 640, bottom: 686, height: 46 }
    const oldTop = row.bottom - 300 // = 386，比行顶 640 高出 254px → 悬空
    expect(oldTop).toBeLessThan(row.top)
    // 新写法：bottom 锚在行底，浮层高度由内容决定，1 行内容就只占 1 行
    const p = computePos(row, modal, viewport)
    expect(p.bottom).toBe(viewport.height - row.bottom)
  })

  it('弹窗较矮时高度下限 120px', () => {
    const shortModal: Rect = { top: 200, bottom: 360, height: 160 }
    const p = computePos({ top: 210, bottom: 256, height: 46 }, shortModal, viewport)
    expect(p.maxHeight).toBe(120)
  })

  it('上下半部判定以行中心对弹窗中线', () => {
    const midRow = { top: 377, bottom: 423, height: 46 } // 中心 400 = 弹窗中线
    expect(computePos(midRow, modal, viewport).top).toBe(377) // 取上半部
    const justBelow = { top: 401, bottom: 447, height: 46 } // 中心 424 > 400
    expect(computePos(justBelow, modal, viewport).top).toBeUndefined()
  })
})
