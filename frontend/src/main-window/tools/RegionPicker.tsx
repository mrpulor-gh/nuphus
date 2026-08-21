// RegionPicker.tsx — Region selection interactive component
// User drags to select screen area, returns relative coordinates
// Workflow primitive: basic interaction layer for screenshot, find image, locate

import { useState, useRef, useEffect, useCallback } from 'react'
import { invoke } from '../../core/bridge'
import { IconX, IconCheck } from '../../ui/Icons'
import { Button } from '../../ui/Button'

interface Region {
  x: number
  y: number
  width: number
  height: number
}

interface RegionPickerProps {
  /** Close callback */
  onClose: () => void
  /** Confirm region callback */
  onConfirm: (region: Region) => void
  /** Mode: picker=return coords only / capture=screenshot save */
  mode?: 'picker' | 'capture'
  /** Screenshot save path (for capture mode) */
  capturePath?: string
  /** Preloaded screenshot path — if provided, skip internal desktop_screenshot */
  bgImagePath?: string
}

export function RegionPicker({
  onClose,
  onConfirm,
  mode = 'picker',
  bgImagePath,
}: RegionPickerProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  // Screenshot background
  const [bgImage, setBgImage] = useState<HTMLImageElement | null>(null)
  const [loading, setLoading] = useState(true)
  const [screenW, setScreenW] = useState(1920)
  const [screenH, setScreenH] = useState(1080)

  // Selection state
  const [isDragging, setIsDragging] = useState(false)
  const [region, setRegion] = useState<Region | null>(null)
  const dragStart = useRef<{ x: number; y: number } | null>(null)
  const dragEnd = useRef<{ x: number; y: number } | null>(null)

  // Screenshot loading
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        let imgPath: string
        let screenW = 1920
        let screenH = 1080

        if (bgImagePath) {
          // Preloaded screenshot: skip desktop_screenshot, use externally provided path
          imgPath = bgImagePath
          // Screen size already obtained externally, but query again to be safe
          const size = await invoke<{ width: number; height: number }>('execute_tool', {
            tool_name: 'desktop_screen_size',
            params: {},
          })
          if (cancelled) return
          if (size) {
            screenW = size.width
            screenH = size.height
          }
        } else {
          // Get screen size
          const size = await invoke<{ width: number; height: number }>('execute_tool', {
            tool_name: 'desktop_screen_size',
            params: {},
          })
          if (cancelled || !size) return
          screenW = size.width
          screenH = size.height

          // Capture fullscreen — 不传 path：Rust 端自动使用 captures_dir() 生成正确绝对路径
          const result = await invoke<{ width: number; height: number; path: string }>(
            'execute_tool',
            {
              tool_name: 'desktop_screenshot',
              params: {},
            },
          )
          if (cancelled || !result) return
          imgPath = result.path
        }

        setScreenW(screenW)
        setScreenH(screenH)

        // Load as Image
        const img = new Image()
        const { convertFileSrc } = await import('@tauri-apps/api/core')
        img.src = convertFileSrc(imgPath)
        await new Promise<void>((resolve, reject) => {
          img.onload = () => resolve()
          img.onerror = () => reject(new Error('Failed to load screenshot'))
        })
        if (!cancelled) {
          setBgImage(img)
          setLoading(false)
        }
      } catch (e) {
        console.error('RegionPicker: screenshot failed', e)
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [bgImagePath])

  // Draw Canvas (background + selection + info)
  const draw = useCallback(() => {
    const canvas = canvasRef.current
    const img = bgImage
    if (!canvas || !img) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    // Set Canvas size to match screen
    canvas.width = screenW
    canvas.height = screenH

    // Draw screenshot background
    ctx.drawImage(img, 0, 0, screenW, screenH)

    // Semi-transparent mask
    ctx.fillStyle = 'rgba(0,0,0,0.35)'
    ctx.fillRect(0, 0, screenW, screenH)

    if (dragStart.current && dragEnd.current) {
      // Calculate selection (supports drag in any direction)
      const sx = Math.min(dragStart.current.x, dragEnd.current.x)
      const sy = Math.min(dragStart.current.y, dragEnd.current.y)
      const ex = Math.max(dragStart.current.x, dragEnd.current.x)
      const ey = Math.max(dragStart.current.y, dragEnd.current.y)
      const w = ex - sx
      const h = ey - sy

      // Erase mask over selection area (show original image)
      ctx.save()
      ctx.beginPath()
      ctx.rect(sx, sy, w, h)
      ctx.clip()
      ctx.clearRect(sx, sy, w, h)
      ctx.drawImage(img, sx, sy, w, h, sx, sy, w, h)
      ctx.restore()

      // Selection border — glow effect
      ctx.strokeStyle = 'rgba(59,130,246,0.8)'
      ctx.lineWidth = 2
      ctx.shadowColor = 'rgba(59,130,246,0.5)'
      ctx.shadowBlur = 8
      ctx.strokeRect(sx, sy, w, h)
      ctx.shadowBlur = 0

      // Corner markers (four small squares)
      const cornerSize = 6
      ctx.fillStyle = 'rgba(59,130,246,0.9)'
      ;[
        [sx, sy],
        [ex - cornerSize, sy],
        [sx, ey - cornerSize],
        [ex - cornerSize, ey - cornerSize],
      ].forEach(([cx, cy]) => {
        ctx.fillRect(cx, cy, cornerSize, cornerSize)
      })

      // Coordinate label (floating above selection)
      const label = `${w} × ${h}   (${sx}, ${sy})`
      ctx.font = '13px system-ui, sans-serif'
      ctx.textAlign = 'center'
      const metrics = ctx.measureText(label)
      const lw = metrics.width + 20
      const lh = 30
      const lx = sx + w / 2 - lw / 2
      const ly = sy - lh - 8

      // Label background
      ctx.fillStyle = 'rgba(14,14,20,0.9)'
      ctx.shadowColor = 'rgba(0,0,0,0.4)'
      ctx.shadowBlur = 8
      ctx.beginPath()
      ctx.roundRect(lx, ly, lw, lh, 6)
      ctx.fill()
      ctx.shadowBlur = 0

      // Label border
      ctx.strokeStyle = 'rgba(59,130,246,0.3)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.roundRect(lx, ly, lw, lh, 6)
      ctx.stroke()

      // Label text
      ctx.fillStyle = '#f5f5fa'
      ctx.font = '13px system-ui, sans-serif'
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillText(label, sx + w / 2, ly + lh / 2)
    }
  }, [bgImage, screenW, screenH])

  // Redraw
  useEffect(() => {
    draw()
  }, [draw])

  /** Convert mouse event coordinates (CSS px) to Canvas internal resolution coordinates */
  const canvasToScreen = (clientX: number, clientY: number) => {
    const cvs = canvasRef.current
    if (!cvs) return { x: clientX, y: clientY }
    const rect = cvs.getBoundingClientRect()
    // Canvas internal resolution vs CSS display size → scale factor
    const sx = cvs.width / rect.width
    const sy = cvs.height / rect.height
    return {
      x: Math.round((clientX - rect.left) * sx),
      y: Math.round((clientY - rect.top) * sy),
    }
  }

  // Mouse events
  const handleMouseDown = (e: React.MouseEvent) => {
    const { x, y } = canvasToScreen(e.clientX, e.clientY)
    dragStart.current = { x, y }
    dragEnd.current = { x, y }
    setIsDragging(true)
  }

  const handleMouseMove = (e: React.MouseEvent) => {
    if (!isDragging || !dragStart.current) return
    const { x, y } = canvasToScreen(e.clientX, e.clientY)
    dragEnd.current = { x, y }
    draw()
  }

  const handleMouseUp = () => {
    if (!isDragging || !dragStart.current || !dragEnd.current) {
      setIsDragging(false)
      return
    }
    setIsDragging(false)

    const sx = Math.min(dragStart.current.x, dragEnd.current.x)
    const sy = Math.min(dragStart.current.y, dragEnd.current.y)
    const ex = Math.max(dragStart.current.x, dragEnd.current.x)
    const ey = Math.max(dragStart.current.y, dragEnd.current.y)
    const w = ex - sx
    const h = ey - sy

    if (w < 5 || h < 5) {
      // Too small, treat as accidental touch
      dragStart.current = null
      dragEnd.current = null
      draw()
      return
    }

    setRegion({ x: sx, y: sy, width: w, height: h })
  }

  const handleConfirm = () => {
    if (!region) return
    onConfirm(region)
  }

  const handleRetry = () => {
    setRegion(null)
    dragStart.current = null
    dragEnd.current = null
    draw()
  }

  return (
    <div
      ref={containerRef}
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        zIndex: 200,
        cursor: isDragging ? 'crosshair' : region ? 'default' : 'crosshair',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      {/* Canvas layer */}
      <canvas
        ref={canvasRef}
        style={{
          display: 'block',
          width: screenW + 'px',
          height: screenH + 'px',
          position: 'absolute',
          top: 0,
          left: 0,
        }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
      />

      {/* Bottom toolbar */}
      <div
        style={{
          position: 'fixed',
          bottom: 32,
          left: '50%',
          transform: 'translateX(-50%)',
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          background: 'rgba(14,14,20,0.92)',
          backdropFilter: 'blur(16px)',
          border: '1px solid rgba(255,255,255,0.08)',
          borderRadius: 14,
          padding: '10px 20px',
          boxShadow: '0 4px 24px rgba(0,0,0,0.5)',
          zIndex: 201,
        }}
      >
        {/* Mode label */}
        <span
          style={{
            fontSize: 12,
            color: 'var(--spark-tertiary)',
            fontFamily: 'var(--font-mono)',
            marginRight: 4,
          }}
        >
          {mode === 'capture' ? '截图' : '框选'}
        </span>

        {/* Separator */}
        <div style={{ width: 1, height: 20, background: 'var(--glass-2)' }} />

        {/* Coordinate info */}
        <span
          style={{
            fontSize: 12,
            color: 'var(--spark-secondary)',
            fontFamily: 'var(--font-mono)',
            minWidth: 100,
          }}
        >
          {region
            ? `${region.width}×${region.height}  at (${region.x}, ${region.y})`
            : loading
              ? '加载中...'
              : '拖拽选择区域'}
        </span>

        {/* Separator */}
        <div style={{ width: 1, height: 20, background: 'var(--glass-2)' }} />

        {/* Action buttons */}
        {region ? (
          <>
            <Button variant="default" onClick={handleRetry}>
              重选
            </Button>
            <Button variant="primary" onClick={handleConfirm}>
              <IconCheck size={13} />
              确认{mode === 'capture' ? '截图' : '选区'}
            </Button>
          </>
        ) : (
          <Button variant="default" onClick={onClose}>
            <IconX size={13} />
            取消
          </Button>
        )}
      </div>
    </div>
  )
}
