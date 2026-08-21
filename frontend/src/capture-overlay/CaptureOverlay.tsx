// CaptureOverlay.tsx — 全屏遮罩覆盖窗
// v5 — 放大镜：稳定显示鼠标附近区域 + 选区边界标线覆盖，精确裁剪

import { useState, useRef, useCallback, useEffect } from 'react'
import { invoke } from '../core/bridge'
import { Button } from '../ui/Button'

const MAGNIFIER_DIAMETER = 160 // 放大镜直径 px
const CAPTURE_SIZE = 40 // 后端采样 40×40 像素（160/40=4x 放大）

// Default mode — will be updated by Rust via Tauri event on each show
let _pendingMode: string | null = null
;(window as any).__setOverlayMode__ = (mode: string) => {
  _pendingMode = mode
  window.dispatchEvent(new CustomEvent('overlay-mode-changed', { detail: mode }))
}

export function CaptureOverlay() {
  // Dynamic mode: initial from URL, then updated by Rust on each show
  const initialMode = new URLSearchParams(window.location.search).get('mode') || 'screenshot'
  const [overlayMode, setOverlayMode] = useState<string>(initialMode)
  const SHOW_PREVIEW = overlayMode === 'screenshot'
  const IS_COLOR_PICKER = overlayMode === 'color_picker'
  const IS_MOUSE_POS = overlayMode === 'mouse_pos'

  // Listen for mode changes from Rust (when overlay is reshown with different mode)
  useEffect(() => {
    const applyMode = () => {
      if (!_pendingMode) return
      setOverlayMode(_pendingMode)
      // Reset all interaction state for fresh session
      setIsDragging(false)
      isDraggingRef.current = false
      setRegion(null)
      dragStart.current = null
      dragRegionRef.current = null
      setCursor(null)
      cursorPosRef.current = null
      overlayRegionRef.current = null
      setLiveColor(null)
      liveColorRef.current = null
      setCaptureResult(null)
      captureResultRef.current = null
      setMask({ top: 0, bottom: 0, left: 0, right: 0 })
      _pendingMode = null
    }
    if (_pendingMode) applyMode()
    const handler = () => applyMode()
    window.addEventListener('overlay-mode-changed', handler)
    return () => window.removeEventListener('overlay-mode-changed', handler)
  }, [])
  const [isDragging, setIsDragging] = useState(false)
  const isDraggingRef = useRef(false)
  const [region, setRegion] = useState<{ x: number; y: number; w: number; h: number } | null>(null)
  const dragStart = useRef<{ x: number; y: number } | null>(null)
  const dragRegionRef = useRef<{ x: number; y: number; w: number; h: number } | null>(null)

  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null)
  const magnifierElRef = useRef<HTMLImageElement | null>(null)
  // RAF 循环通过 ref 读取当前光标+选区，绘制边界标线（避免闭包捕获过时值）
  const cursorPosRef = useRef<{ x: number; y: number } | null>(null)
  const overlayRegionRef = useRef<{ x: number; y: number; w: number; h: number } | null>(null)

  // 取色模式：实时预览颜色
  const [liveColor, setLiveColor] = useState<{
    hex: string
    r: number
    g: number
    b: number
  } | null>(null)
  const liveColorRef = useRef<typeof liveColor>(null)

  const [captureResult, setCaptureResult] = useState<{
    path: string
    base64: string
    x: number
    y: number
    width: number
    height: number
  } | null>(null)
  const captureResultRef = useRef<typeof captureResult>(null)

  const [mask, setMask] = useState({ top: 0, bottom: 0, left: 0, right: 0 })
  const canvasRef = useRef<HTMLCanvasElement>(null)

  // ── 持久 Image + RAF 循环：放大图像 + 十字线 + 选区边界标线 ──
  useEffect(() => {
    const img = new Image()
    magnifierElRef.current = img
    let running = true
    const S = CAPTURE_SIZE
    const D = MAGNIFIER_DIAMETER

    const draw = () => {
      if (!running) return
      const canvas = canvasRef.current
      if (canvas && img.complete && img.naturalWidth > 0) {
        const ctx = canvas.getContext('2d')
        if (!ctx) {
          requestAnimationFrame(draw)
          return
        }

        // 清空
        ctx.fillStyle = '#000'
        ctx.fillRect(0, 0, D, D)
        ctx.imageSmoothingEnabled = false

        // 绘制放大图像（S×S 源像素 → D×D 画布）
        ctx.drawImage(img, 0, 0, S, S, 0, 0, D, D)

        // ── 选区边界标线（叠加在放大图上） ──
        const sel = overlayRegionRef.current
        const cur = cursorPosRef.current
        if (sel && sel.w >= 5 && sel.h >= 5 && cur) {
          // 放大镜视口：以 cursor 为中心，S×S 像素
          const vpLeft = cur.x - S / 2
          const vpTop = cur.y - S / 2
          const scale = D / S // 屏幕坐标 → 画布坐标 的缩放

          // 计算选区四边在画布上的位置（钳制到画布边界）
          const left = Math.max(0, (sel.x - vpLeft) * scale)
          const top = Math.max(0, (sel.y - vpTop) * scale)
          const right = Math.min(D, (sel.x + sel.w - vpLeft) * scale)
          const bottom = Math.min(D, (sel.y + sel.h - vpTop) * scale)

          if (right > left && bottom > top) {
            // ── 选区外半透明遮罩 ──
            ctx.fillStyle = 'rgba(0,0,0,0.35)'
            // 上
            ctx.fillRect(0, 0, D, top)
            // 下
            ctx.fillRect(0, bottom, D, D - bottom)
            // 左（中间段）
            ctx.fillRect(0, top, left, bottom - top)
            // 右（中间段）
            ctx.fillRect(right, top, D - right, bottom - top)

            // ── 选区边框（亮蓝色，2px） ──
            ctx.strokeStyle = '#00aaff'
            ctx.lineWidth = 2
            ctx.strokeRect(left, top, right - left, bottom - top)

            // ── 四角标记（增强可识别性） ──
            const cornerLen = 6
            ctx.strokeStyle = '#00ddff'
            ctx.lineWidth = 1.5
            ctx.beginPath()
            // 左上
            ctx.moveTo(left, top + cornerLen)
            ctx.lineTo(left, top)
            ctx.lineTo(left + cornerLen, top)
            // 右上
            ctx.moveTo(right - cornerLen, top)
            ctx.lineTo(right, top)
            ctx.lineTo(right, top + cornerLen)
            // 左下
            ctx.moveTo(left, bottom - cornerLen)
            ctx.lineTo(left, bottom)
            ctx.lineTo(left + cornerLen, bottom)
            // 右下
            ctx.moveTo(right - cornerLen, bottom)
            ctx.lineTo(right, bottom)
            ctx.lineTo(right, bottom - cornerLen)
            ctx.stroke()

            // ── 尺寸标注（右上角） ──
            const label = `${Math.round(sel.w)} × ${Math.round(sel.h)}`
            ctx.font = '10px sans-serif'
            const tw = ctx.measureText(label).width
            const lx = Math.min(right - tw - 4, D - tw - 2)
            const ly = Math.max(top + 2, 2)
            ctx.fillStyle = 'rgba(0,0,0,0.6)'
            ctx.fillRect(lx - 2, ly, tw + 4, 14)
            ctx.fillStyle = '#00ddff'
            ctx.textAlign = 'left'
            ctx.fillText(label, lx, ly + 11)
          }
        }

        // ── 十字标线 ──
        const cx = D / 2
        const cy = D / 2
        ctx.strokeStyle = 'rgba(255,255,255,0.75)'
        ctx.lineWidth = 0.5
        ctx.beginPath()
        ctx.moveTo(0, cy)
        ctx.lineTo(D, cy)
        ctx.moveTo(cx, 0)
        ctx.lineTo(cx, D)
        ctx.stroke()

        // ── 中心点 ──
        ctx.fillStyle = 'rgba(255,0,0,0.9)'
        ctx.beginPath()
        ctx.arc(cx, cy, 2, 0, Math.PI * 2)
        ctx.fill()

        // ── 取色模式：读取中心像素颜色 + 显示信息 ──
        if (IS_COLOR_PICKER) {
          // 中心像素 = 放大图像中 D/2, D/2，对应源图像中心
          const pixelData = ctx.getImageData(D / 2, D / 2, 1, 1).data
          const r = pixelData[0],
            g = pixelData[1],
            b = pixelData[2]
          const hexStr = `#${r.toString(16).padStart(2, '0').toUpperCase()}${g.toString(16).padStart(2, '0').toUpperCase()}${b.toString(16).padStart(2, '0').toUpperCase()}`
          liveColorRef.current = { hex: hexStr, r, g, b }
          setLiveColor({ hex: hexStr, r, g, b })

          // 在放大镜下方绘制颜色信息条
          const infoY = D + 8
          const swatchSize = 14
          // 背景
          ctx.fillStyle = 'rgba(0,0,0,0.75)'
          const iw = 180
          const ih = 22
          ctx.beginPath()
          ctx.roundRect((D - iw) / 2, infoY, iw, ih, 4)
          ctx.fill()
          // 色块
          ctx.fillStyle = hexStr
          ctx.fillRect((D - iw) / 2 + 4, infoY + 4, swatchSize, swatchSize)
          // 文字
          ctx.fillStyle = '#fff'
          ctx.font = '11px monospace'
          ctx.textAlign = 'left'
          ctx.textBaseline = 'middle'
          const textX = (D - iw) / 2 + 4 + swatchSize + 6
          ctx.fillText(`${hexStr}  RGB(${r},${g},${b})`, textX, infoY + ih / 2)
        }
      }
      requestAnimationFrame(draw)
    }
    requestAnimationFrame(draw)
    return () => {
      running = false
    }
  }, [])

  // ── 放大镜更新：RAF 节流，始终以鼠标为中心的放大区域 ──
  const magPosRef = useRef<{ x: number; y: number } | null>(null)
  const magPendingRef = useRef(false)
  const requestMagnifier = useCallback((cx: number, cy: number) => {
    cursorPosRef.current = { x: cx, y: cy }
    // 同步选区 ref 供 RAF 循环绘制边界标线
    overlayRegionRef.current = dragRegionRef.current

    magPosRef.current = { x: cx, y: cy }
    if (magPendingRef.current) return
    magPendingRef.current = true
    requestAnimationFrame(async () => {
      magPendingRef.current = false
      const pos = magPosRef.current
      if (!pos) return
      try {
        const b64 = await invoke<string | null>('overlay_magnifier_region', {
          x: Math.round(pos.x),
          y: Math.round(pos.y),
          size: CAPTURE_SIZE,
        })
        const img = magnifierElRef.current
        if (img && b64) img.src = b64
      } catch {
        /* ignore */
      }
    })
  }, [])

  // ── 键盘事件（Esc 退出） ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        // 预览模式下按 Esc = 取消整个截图
        // 拖拽/选区模式下按 Esc = 取消
        invoke('overlay_capture_cancel').catch(console.error)
      }
    }
    window.addEventListener('keydown', onKey, true) // capture phase
    document.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [])

  // ── 鼠标右键取消（仅选区阶段，预览时不触发） ──
  useEffect(() => {
    const onContext = (e: MouseEvent) => {
      if (captureResultRef.current) return // 预览阶段不禁用右键（用户可能想复制等）
      e.preventDefault()
      invoke('overlay_capture_cancel').catch(console.error)
    }
    window.addEventListener('contextmenu', onContext)
    return () => window.removeEventListener('contextmenu', onContext)
  }, [])

  // ── 鼠标事件 ──
  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (captureResultRef.current) return
      dragStart.current = { x: e.clientX, y: e.clientY }
      dragRegionRef.current = null
      overlayRegionRef.current = null
      setIsDragging(true)
      isDraggingRef.current = true
      setRegion(null)
      setMask({ top: 0, bottom: 0, left: 0, right: 0 })
      setCursor({ x: e.clientX, y: e.clientY })
      requestMagnifier(e.clientX, e.clientY)
    },
    [requestMagnifier],
  )

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      const cx = e.clientX,
        cy = e.clientY
      setCursor({ x: cx, y: cy })
      if (!isDraggingRef.current || !dragStart.current || captureResultRef.current) {
        requestMagnifier(cx, cy)
        return
      }
      const start = dragStart.current
      const x = Math.min(start.x, cx),
        y = Math.min(start.y, cy)
      const w = Math.abs(cx - start.x),
        h = Math.abs(cy - start.y)
      const sel = { x, y, w, h }
      dragRegionRef.current = sel
      overlayRegionRef.current = sel
      setRegion(sel)
      setMask({
        top: y,
        bottom: window.innerHeight - y - h,
        left: x,
        right: window.innerWidth - x - w,
      })
      requestMagnifier(cx, cy)
    },
    [requestMagnifier],
  )

  const handleMouseUp = useCallback(async () => {
    if (!isDraggingRef.current || captureResultRef.current) return
    setIsDragging(false)
    isDraggingRef.current = false

    // 取色模式：直接用光标位置取色，不依赖拖拽区域
    if (IS_COLOR_PICKER) {
      const cur = cursorPosRef.current
      if (!cur) return
      try {
        await invoke('overlay_pick_color', { x: Math.round(cur.x), y: Math.round(cur.y) })
      } catch (e) {
        console.error('取色失败:', e)
      }
      // overlay_pick_color 内部已恢复主窗、关闭覆盖窗、写入 CAPTURE_RESULT
      return
    }

    // 鼠标坐标模式：单击捕获点坐标，直接调用 overlay_capture_done
    if (IS_MOUSE_POS) {
      const cur = cursorPosRef.current
      if (!cur) return
      const cx = Math.round(cur.x)
      const cy = Math.round(cur.y)
      // 以 1×1 虚拟区域存储坐标，调用方从 region 提取 (x, y)
      try {
        await invoke('overlay_capture_done', {
          path: '',
          x: cx,
          y: cy,
          width: 1,
          height: 1,
        })
      } catch (e) {
        console.error('坐标捕获失败:', e)
      }
      return
    }

    const finalRegion = dragRegionRef.current
    if (!finalRegion || finalRegion.w < 5 || finalRegion.h < 5) return
    try {
      const result = await invoke<any>('overlay_capture_confirm', {
        x: Math.round(finalRegion.x),
        y: Math.round(finalRegion.y),
        width: Math.round(finalRegion.w),
        height: Math.round(finalRegion.h),
        mode: overlayMode,
      })
      const cr = {
        path: result.path,
        base64: result.base64,
        x: result.x,
        y: result.y,
        width: result.width,
        height: result.height,
      }
      captureResultRef.current = cr
      setCaptureResult(cr)
    } catch (e: any) {
      console.error('截图确认失败:', e)
      setRegion(null)
      dragRegionRef.current = null
      setMask({ top: 0, bottom: 0, left: 0, right: 0 })
    }
  }, [])

  const handleDone = useCallback(() => {
    const cr = captureResultRef.current
    if (!cr) return
    invoke('overlay_capture_done', {
      path: cr.path,
      x: cr.x,
      y: cr.y,
      width: cr.width,
      height: cr.height,
      base64: cr.base64 || null,
    }).catch(console.error)
  }, [])

  // ── 非截图模式（选区/模板/OCR）：确认后自动完成，跳过预览 ──
  useEffect(() => {
    if (captureResult && !SHOW_PREVIEW) {
      handleDone()
    }
  }, [captureResult, handleDone])

  // ── 预览态快捷键：Enter 确认（预览仅截图模式出现）──
  useEffect(() => {
    if (!SHOW_PREVIEW) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Enter' && captureResultRef.current) {
        e.preventDefault()
        handleDone()
      }
    }
    window.addEventListener('keydown', onKey, true)
    document.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [SHOW_PREVIEW, handleDone])

  const handleReselect = useCallback(() => {
    captureResultRef.current = null
    setCaptureResult(null)
    setRegion(null)
    dragRegionRef.current = null
    setMask({ top: 0, bottom: 0, left: 0, right: 0 })
  }, [])

  const hasSelection = region !== null && region.w >= 5 && region.h >= 5
  const magX = cursor ? Math.min(cursor.x + 20, window.innerWidth - MAGNIFIER_DIAMETER - 10) : 10
  const magY = cursor ? Math.max(10, cursor.y - MAGNIFIER_DIAMETER - 10) : 10

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        overflow: 'hidden',
        cursor: captureResult ? 'default' : 'crosshair',
        userSelect: 'none',
        background: 'transparent',
      }}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
    >
      {/* ── CSS 遮罩 ── */}
      {!captureResult && (
        <div style={{ position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 1 }}>
          <div
            style={{
              position: 'absolute',
              top: 0,
              left: 0,
              width: '100vw',
              height: mask.top,
              background: 'rgba(0,0,0,0.45)',
            }}
          />
          <div
            style={{
              position: 'absolute',
              bottom: 0,
              left: 0,
              width: '100vw',
              height: mask.bottom,
              background: 'rgba(0,0,0,0.45)',
            }}
          />
          <div
            style={{
              position: 'absolute',
              top: mask.top,
              left: 0,
              width: mask.left,
              height: Math.max(0, window.innerHeight - mask.top - mask.bottom),
              background: 'rgba(0,0,0,0.45)',
            }}
          />
          <div
            style={{
              position: 'absolute',
              top: mask.top,
              right: 0,
              width: mask.right,
              height: Math.max(0, window.innerHeight - mask.top - mask.bottom),
              background: 'rgba(0,0,0,0.45)',
            }}
          />
        </div>
      )}

      {/* ── 选区边框 ── */}
      {hasSelection && !captureResult && region && (
        <>
          <div
            style={{
              position: 'absolute',
              left: region.x - 1,
              top: region.y - 1,
              width: region.w + 2,
              height: region.h + 2,
              border: '2px solid #00aaff',
              boxSizing: 'border-box',
              pointerEvents: 'none',
              zIndex: 2,
            }}
          />
          <div
            style={{
              position: 'absolute',
              left: region.x,
              top: region.y - 22,
              background: '#00aaff',
              color: '#fff',
              fontSize: 11,
              padding: '2px 8px',
              borderRadius: 3,
              pointerEvents: 'none',
              whiteSpace: 'nowrap',
              zIndex: 2,
            }}
          >
            {region.w} × {region.h}
          </div>
        </>
      )}

      {/* ── 放大镜（canvas） — 始终在 DOM 中，cursor 控制显隐 ── */}
      <div
        style={{
          position: 'fixed',
          left: magX,
          top: magY,
          width: MAGNIFIER_DIAMETER,
          height: MAGNIFIER_DIAMETER,
          borderRadius: '50%',
          border: '2px solid rgba(255,255,255,0.7)',
          boxShadow: '0 2px 12px rgba(0,0,0,0.4)',
          overflow: 'hidden',
          pointerEvents: 'none',
          zIndex: 100,
          background: '#000',
          visibility: cursor && !captureResult ? 'visible' : 'hidden',
        }}
      >
        <canvas
          ref={canvasRef}
          width={MAGNIFIER_DIAMETER}
          height={MAGNIFIER_DIAMETER}
          style={{ width: MAGNIFIER_DIAMETER, height: MAGNIFIER_DIAMETER, display: 'block' }}
        />
      </div>

      {/* ── 预览缩略图（玻璃拟态确认条） ── */}
      {captureResult && SHOW_PREVIEW && (
        <div className="capture-confirm">
          <img src={captureResult.base64} alt="截图预览" className="capture-confirm-preview" />
          <div className="capture-confirm-path">{captureResult.path}</div>
          <div className="capture-confirm-meta">
            选区 ({captureResult.x}, {captureResult.y}) · {captureResult.width} ×{' '}
            {captureResult.height}
          </div>
          <div className="capture-confirm-actions">
            <Button variant="default" onClick={handleReselect}>
              重新选择
            </Button>
            <Button variant="primary" onClick={handleDone}>
              ✓ 确认
            </Button>
          </div>
          <div className="capture-confirm-hint">
            <kbd>Enter</kbd> 确认 · <kbd>Esc</kbd> 取消
          </div>
        </div>
      )}

      {/* ── 初始提示 ── */}
      {!hasSelection && !isDragging && !captureResult && (
        <div
          style={{
            position: 'fixed',
            top: 24,
            left: '50%',
            transform: 'translateX(-50%)',
            background: 'color-mix(in srgb, var(--surface-2) 82%, transparent)',
            color: 'var(--fg-3)',
            border: '1px solid var(--line-2)',
            padding: '8px 20px',
            borderRadius: 'var(--radius-2)',
            fontSize: 13,
            backdropFilter: 'blur(10px)',
            WebkitBackdropFilter: 'blur(10px)',
            pointerEvents: 'none',
            zIndex: 50,
          }}
        >
          拖拽选择截图区域 · 放大镜精确定位 · Esc / 右键取消
        </div>
      )}
    </div>
  )
}