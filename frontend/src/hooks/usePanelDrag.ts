// usePanelDrag.ts — 应用内面板拖拽的**单一实现点**
//
// 机制从 DesktopToolbar.tsx（桌面工具浮窗条，battle-tested）逐行抽出：
//   mousedown 记抓取偏移 → window mousemove 实时跟手（视口边界钳制）→ mouseup 落盘
// 语义与源实现同构（mousedown/mousemove/mouseup，**不用 pointer events**），
// 只对源语义做两项**可选**扩展，且默认关闭，老调用方行为零变化：
//   ① fallback 缺省 → pos 初值 null（从未拖拽过 → 调用方保持 CSS 原定位，拖过才写 inline）
//   ② clampOnResize 可选开启（窗口变小后把已拖拽面板钳回视口；DesktopToolbar 不传）
import { useCallback, useEffect, useRef, useState } from 'react'
import type { MouseEvent as ReactMouseEvent, RefObject } from 'react'

export interface PanelPos {
  x: number
  y: number
}

export interface UsePanelDragOptions {
  /** 无 localStorage 存储时的默认站位；缺省 → null = 「从未拖拽过，保持 CSS 定位」 */
  fallback?: PanelPos
  /** 是否启用（DesktopToolbar 传 visible；关闭时无监听、mousedown 空转） */
  enabled?: boolean
  /** 窗口变小后把已拖拽的面板钳回视口（DesktopToolbar 不传，保持零变化） */
  clampOnResize?: boolean
}

export interface PanelDrag {
  /** 当前站位；null = 未拖拽过（调用方不应写 inline left/top） */
  pos: PanelPos | null
  /** 挂到被拖面板根节点：mousedown 取 rect、mousemove 取 offsetWidth/offsetHeight */
  panelRef: RefObject<HTMLDivElement>
  /** 绑到拖拽把手的 onMouseDown */
  handleMouseDown: (e: ReactMouseEvent) => void
}

/** 带 fallback 的重载：站位必有值（如 DesktopToolbar 的默认 {x:669,y:60}） */
export function usePanelDrag(
  storageKey: string,
  options: UsePanelDragOptions & { fallback: PanelPos },
): PanelDrag & { pos: PanelPos }
/** 无 fallback 的重载：pos 初始可为 null（未拖拽过 → 保持 CSS 定位） */
export function usePanelDrag(storageKey: string, options?: UsePanelDragOptions): PanelDrag
export function usePanelDrag(storageKey: string, options: UsePanelDragOptions = {}): PanelDrag {
  const { fallback, enabled = true, clampOnResize = false } = options

  const [pos, setPos] = useState<PanelPos | null>(() => {
    const saved = localStorage.getItem(storageKey)
    if (!saved) return fallback ?? null
    try {
      return JSON.parse(saved) as PanelPos
    } catch {
      // 存储值损坏（手改 / 旧版残留）：退回默认站位，不让渲染期 JSON.parse 崩掉面板
      return fallback ?? null
    }
  })
  const dragging = useRef(false)
  const dragOffset = useRef({ x: 0, y: 0 })
  const panelRef = useRef<HTMLDivElement>(null)

  const savePos = useCallback(
    (x: number, y: number) => {
      localStorage.setItem(storageKey, JSON.stringify({ x, y }))
    },
    [storageKey],
  )

  const handleMouseDown = (e: ReactMouseEvent) => {
    if (!enabled || !panelRef.current) return
    const rect = panelRef.current.getBoundingClientRect()
    dragging.current = true
    dragOffset.current = { x: e.clientX - rect.left, y: e.clientY - rect.top }
  }

  // ── 拖拽监听（与源实现同构：mousemove 只跟手 + 钳制，mouseup 落盘）──
  useEffect(() => {
    if (!enabled) return
    const handleMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return
      const w = panelRef.current?.offsetWidth || 200
      const h = panelRef.current?.offsetHeight || 40
      const maxX = Math.max(0, window.innerWidth - w)
      const maxY = Math.max(0, window.innerHeight - h)
      const newX = Math.max(0, Math.min(maxX, e.clientX - dragOffset.current.x))
      const newY = Math.max(0, Math.min(maxY, e.clientY - dragOffset.current.y))
      setPos({ x: newX, y: newY })
    }
    const handleMouseUp = () => {
      if (dragging.current) {
        dragging.current = false
        // 从没拖出过位置（pos 仍 null，如点了把手但没移动）→ 不写存储，保持「未拖拽过」语义
        if (pos) savePos(pos.x, pos.y)
      }
    }
    window.addEventListener('mousemove', handleMouseMove)
    window.addEventListener('mouseup', handleMouseUp)
    return () => {
      window.removeEventListener('mousemove', handleMouseMove)
      window.removeEventListener('mouseup', handleMouseUp)
    }
  }, [enabled, pos, savePos])

  // ── 窗口变小 → 已拖拽的面板钳回视口（不落盘：坐标仍由用户下一次松手定稿）──
  useEffect(() => {
    if (!enabled || !clampOnResize || !pos) return
    const handleResize = () => {
      const w = panelRef.current?.offsetWidth || 200
      const h = panelRef.current?.offsetHeight || 40
      const maxX = Math.max(0, window.innerWidth - w)
      const maxY = Math.max(0, window.innerHeight - h)
      setPos(prev =>
        prev
          ? { x: Math.max(0, Math.min(maxX, prev.x)), y: Math.max(0, Math.min(maxY, prev.y)) }
          : prev,
      )
    }
    window.addEventListener('resize', handleResize)
    return () => window.removeEventListener('resize', handleResize)
  }, [enabled, clampOnResize, pos])

  return { pos, panelRef, handleMouseDown }
}
