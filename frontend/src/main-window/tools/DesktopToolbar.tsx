// DesktopToolbar.tsx — Desktop tool floating bar v4
// Ctrl+U to invoke, freely draggable
// Screenshot/region/OCR overlay tools: start_overlay_mask returns immediately, poll take_capture_result for results
// Completely solve Tauri event loss / oneshot blocking null issue

import { useState, useRef, useEffect, useCallback } from 'react'
import { invoke as bridgeInvoke } from '../../core/bridge'

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    return (await invoke(cmd, args)) as T
  } catch (e: any) {
    const parts: string[] = []
    if (e?.message) parts.push(e.message)
    if (typeof e === 'string') parts.push(e)
    try {
      parts.push(JSON.stringify(e))
    } catch {}
    const msg = parts.filter(Boolean).join(' | ') || '未知错误'
    throw new Error(`Tauri invoke ${cmd} failed: ${msg}`)
  }
}

import {
  IconCamera,
  IconCrop,
  IconCrosshair,
  IconType,
  IconGrip,
  IconX,
  IconCopy,
  IconCheck,
  IconPin,
  IconPinOff,
} from '../../ui/Icons'

import { Pipette as IconDropper } from 'lucide-react'

import { OcrDictionary } from './OcrDictionary'
import { Button, IconButton } from '../../ui/Button'

interface DesktopToolbarProps {
  visible: boolean
  onClose: () => void
}

type ToolMode = null | 'screenshot' | 'picker' | 'mouse_pos' | 'ocr' | 'color_picker'

type ResultType = 'text' | 'ocr' | 'info'

interface ToolResult {
  type: ResultType
  content: string
  title: string
}

interface ColorSlot {
  hex: string
  rgb: number[]
  filled: boolean
}

const TOOLTIP: Record<NonNullable<ToolMode>, string> = {
  screenshot: '截图 — 框选区域保存为图片（自动隐藏窗口）',
  picker: '选区 — 返回区域坐标（自动隐藏窗口）',
  mouse_pos: '鼠标位置 — 实时显示光标坐标',
  ocr: '游戏文字识别 — 框选区域提取文字（颜色+字典匹配）',
  color_picker: '取色 — 选取屏幕某点颜色值',
}

interface ToolBtn {
  mode: ToolMode
  icon: React.ElementType
  label: string
  desc: string
}

const TOOLS: ToolBtn[] = [
  { mode: 'screenshot', icon: IconCamera, label: '截图', desc: '截图保存' },
  { mode: 'picker', icon: IconCrop, label: '选区', desc: '获取坐标' },
  { mode: 'mouse_pos', icon: IconCrosshair, label: '鼠标', desc: '实时坐标' },
  { mode: 'color_picker', icon: IconDropper, label: '取色', desc: '屏幕取色' },
  { mode: 'ocr', icon: IconType, label: '游戏字典', desc: '游戏提取' },
]

export function DesktopToolbar({ visible, onClose }: DesktopToolbarProps) {
  const [pos, setPos] = useState(() => {
    const saved = localStorage.getItem('desktop_toolbar_pos')
    return saved ? JSON.parse(saved) : { x: 669, y: 60 }
  })

  // ── Pin (always on top) state ──
  const [pinned, setPinned] = useState(() => {
    return localStorage.getItem('desktop_toolbar_pinned') === 'true'
  })

  const togglePin = useCallback(async () => {
    const newState = !pinned
    try {
      const result = await bridgeInvoke<boolean>('toggle_main_window_topmost')
      setPinned(result ?? newState)
      localStorage.setItem('desktop_toolbar_pinned', String(result ?? newState))
    } catch {
      setPinned(newState)
      localStorage.setItem('desktop_toolbar_pinned', String(newState))
    }
  }, [pinned])
  const dragging = useRef(false)
  const dragOffset = useRef({ x: 0, y: 0 })
  const barRef = useRef<HTMLDivElement>(null)

  // Sub-panel state
  const [activeTool, setActiveTool] = useState<ToolMode>(null)
  const [result, setResult] = useState<ToolResult | null>(null)
  const [loading, setLoading] = useState(false)
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // Mouse position live listener
  const [cursorPos, setCursorPos] = useState({ x: 0, y: 0 })
  const cursorInterval = useRef<ReturnType<typeof setInterval> | null>(null)

  const [copied, setCopied] = useState(false)

  // ── Color picker 9-slot state ──
  const initSlots = () =>
    Array.from({ length: 9 }, (): ColorSlot => ({ hex: '#000000', rgb: [0, 0, 0], filled: false }))
  const [colorSlots, setColorSlots] = useState<ColorSlot[]>(initSlots)
  const [pickingSlotIndex, setPickingSlotIndex] = useState<number | null>(null)
  const pickingSlotIndexRef = useRef<number | null>(null)
  const [colorCopiedIndex, setColorCopiedIndex] = useState<number | null>(null)

  // Dictionary OCR settings panel
  const [showOcrDict, setShowOcrDict] = useState(false)

  // ── Mouse position polling ──
  useEffect(() => {
    if (activeTool === 'mouse_pos') {
      const poll = async () => {
        try {
          const r = await bridgeInvoke<{ x: number; y: number }>('desktop_mouse_position')
          if (r) setCursorPos(r)
        } catch {
          /* ignore */
        }
      }
      poll()
      cursorInterval.current = setInterval(poll, 60)
    } else {
      if (cursorInterval.current) clearInterval(cursorInterval.current)
      cursorInterval.current = null
    }
    return () => {
      if (cursorInterval.current) clearInterval(cursorInterval.current)
    }
  }, [activeTool])

  // ── Drag logic ──
  const savePos = useCallback((x: number, y: number) => {
    localStorage.setItem('desktop_toolbar_pos', JSON.stringify({ x, y }))
  }, [])

  const handleMouseDown = (e: React.MouseEvent) => {
    if (!barRef.current) return
    const rect = barRef.current.getBoundingClientRect()
    dragging.current = true
    dragOffset.current = { x: e.clientX - rect.left, y: e.clientY - rect.top }
  }

  useEffect(() => {
    if (!visible) return
    const handleMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return
      const w = barRef.current?.offsetWidth || 200
      const h = barRef.current?.offsetHeight || 40
      const maxX = Math.max(0, window.innerWidth - w)
      const maxY = Math.max(0, window.innerHeight - h)
      const newX = Math.max(0, Math.min(maxX, e.clientX - dragOffset.current.x))
      const newY = Math.max(0, Math.min(maxY, e.clientY - dragOffset.current.y))
      setPos({ x: newX, y: newY })
    }
    const handleMouseUp = () => {
      if (dragging.current) {
        dragging.current = false
        savePos(pos.x, pos.y)
      }
    }
    window.addEventListener('mousemove', handleMouseMove)
    window.addEventListener('mouseup', handleMouseUp)
    return () => {
      window.removeEventListener('mousemove', handleMouseMove)
      window.removeEventListener('mouseup', handleMouseUp)
    }
  }, [visible, pos, savePos])

  // ── Stop polling ──
  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current)
      pollRef.current = null
    }
  }, [])

  // ── Poll take_capture_result ──
  const startPolling = useCallback(
    (mode: ToolMode) => {
      stopPolling()
      pollRef.current = setInterval(async () => {
        try {
          const raw = await tauriInvoke<any>('take_capture_result')
          if (raw === null || raw === undefined) {
            // No result yet, continue polling
            return
          }
          // Got result! Stop polling
          stopPolling()
          setLoading(false)

          // Process result
          if (raw.cancelled) {
            if (pickingSlotIndexRef.current !== null) {
              // Color picker cancelled: return to panel
              setPickingSlotIndex(null)
              pickingSlotIndexRef.current = null
              setLoading(false)
              return
            }
            setActiveTool(null)
            setResult({ type: 'info', title: '已取消', content: '已取消' })
            return
          }

          const path = raw.path || ''
          const region = raw.region || {}

          switch (mode) {
            case 'color_picker': {
              const slotIdx = pickingSlotIndexRef.current
              if (slotIdx === null) break
              const color = (raw.color_rgb as number[]) || [0, 0, 0]
              const hex = raw.hex || '#000000'
              setColorSlots(prev => {
                const next = [...prev]
                next[slotIdx] = { hex, rgb: color, filled: true }
                return next
              })
              setPickingSlotIndex(null)
              pickingSlotIndexRef.current = null
              setLoading(false)
              return // Don't show result popup, stay in panel
            }
            case 'picker':
              setActiveTool(null)
              setResult({
                type: 'text',
                title: '选区坐标',
                content: `选区: (${region.x}, ${region.y})  ${region.width} × ${region.height}`,
              })
              break
            default:
              // Screenshot: dispatch to ReferenceBar via CustomEvent (no popup, no tool_call wrapping)
              // The user decides whether to OCR, find_image, or just analyze — we only pass the path.
              setActiveTool(null)
              window.dispatchEvent(
                new CustomEvent('nuphus:capture-result', {
                  detail: { path, region, base64: raw.base64 || null },
                }),
              )
              break
          }
        } catch (e: any) {
          // Only stop on take_capture_result command error
          console.error('[DesktopToolbar] poll error:', e)
          stopPolling()
          setLoading(false)
          if (pickingSlotIndexRef.current === null) {
            setActiveTool(null)
            setResult({ type: 'info', title: '轮询错误', content: String(e) })
          } else {
            // Color picker polling error: return to panel
            setPickingSlotIndex(null)
            pickingSlotIndexRef.current = null
          }
        }
      }, 500) // 500ms polling interval
    },
    [stopPolling],
  )

  // ── Cleanup ──
  useEffect(() => {
    return () => {
      stopPolling()
    }
  }, [stopPolling])

  // ── Global Esc exits active tool (mouse pos, input panel, etc.) ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && activeTool !== null) {
        setActiveTool(null)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [activeTool])

  // ── Tool click ──
  const handleToolClick = async (mode: ToolMode) => {
    setResult(null)
    setCopied(false)

    if (mode === 'mouse_pos') {
      setActiveTool(prev => (prev === 'mouse_pos' ? null : 'mouse_pos'))
      return
    }

    if (mode === 'ocr') {
      setShowOcrDict(true)
      return
    }

    if (mode === 'color_picker') {
      if (activeTool === 'color_picker' && pickingSlotIndex === null) {
        setActiveTool(null)
      } else {
        setActiveTool('color_picker')
        setPickingSlotIndex(null)
        pickingSlotIndexRef.current = null
      }
      return
    }

    setActiveTool(mode)
    setLoading(true)
    try {
      // start_overlay_mask: pre-capture fullscreen → hide main window → create overlay → return immediately
      // Result obtained via polling take_capture_result
      // Start polling, wait for overlay_capture_done/cancel
      await tauriInvoke<any>('start_overlay_mask', { mode })
      startPolling(mode)
    } catch (e: any) {
      setResult({ type: 'info', title: '操作失败', content: String(e) })
      setLoading(false)
      setActiveTool(null)
    }
  }

  // ── Color slot click ──
  const handleColorSlotPick = async (index: number) => {
    setPickingSlotIndex(index)
    pickingSlotIndexRef.current = index
    setLoading(true)
    try {
      await tauriInvoke<any>('start_overlay_mask', { mode: 'color_picker' })
      startPolling('color_picker')
    } catch (e: any) {
      setPickingSlotIndex(null)
      pickingSlotIndexRef.current = null
      setLoading(false)
    }
  }

  // ── Copy result ──
  const handleCopyResult = async () => {
    if (!result) return
    try {
      await navigator.clipboard.writeText(result.content)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      try {
        await bridgeInvoke('desktop_clipboard_write', {
          text: result.content,
        })
        setCopied(true)
        setTimeout(() => setCopied(false), 2000)
      } catch {
        /* ignore */
      }
    }
  }

  if (!visible) return null

  return (
    <>
      {/* ── Main toolbar ── */}
      <div ref={barRef} className="desktop-toolbar" style={{ left: pos.x, top: pos.y }}>
        {/* Drag handle */}
        <div className="desktop-toolbar-grip" onMouseDown={handleMouseDown} title="拖拽移动">
          <IconGrip size={14} />
        </div>

        {/* Tool buttons */}
        {TOOLS.map(tool => (
          <IconButton
            key={tool.mode}
            variant={activeTool === tool.mode ? 'desktop-toolbar-active' : 'desktop-toolbar'}
            label={tool.label}
            onClick={() => handleToolClick(tool.mode)}
            title={tool.desc}
          >
            <tool.icon size={16} />
            <span className="desktop-toolbar-label">{tool.label}</span>
          </IconButton>
        ))}

        {/* Separator */}
        <div className="desktop-toolbar-divider" />

        {/* Mouse position indicator */}
        {activeTool === 'mouse_pos' && (
          <>
            <button
              className="desktop-toolbar-pos"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(`${cursorPos.x},${cursorPos.y}`)
                } catch {
                  /* ignore */
                }
              }}
              title="点击复制坐标"
            >
              ({cursorPos.x}, {cursorPos.y})
            </button>
            <IconButton
              variant="desktop-toolbar"
              label="停止鼠标跟踪"
              onClick={() => setActiveTool(null)}
              style={{ color: '#f87171' }}
            >
              <IconX size={14} />
            </IconButton>
          </>
        )}

        {/* Pin button */}
        <IconButton
          variant={pinned ? 'desktop-toolbar-active' : 'desktop-toolbar'}
          label={pinned ? '取消置顶' : '固定窗口置顶'}
          onClick={togglePin}
          style={{ color: pinned ? '#3b82f6' : undefined }}
        >
          {pinned ? <IconPin size={15} /> : <IconPinOff size={15} />}
        </IconButton>

        {/* Separator */}
        <div className="desktop-toolbar-divider" />

        {/* Close button */}
        <IconButton variant="ghost" label="关闭" onClick={onClose} title="关闭 (Ctrl+U)">
          <IconX size={14} />
        </IconButton>
      </div>

      {/* ── Dictionary OCR panel ── */}
      {showOcrDict && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 200,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: 'rgba(0,0,0,0.5)',
            backdropFilter: 'blur(4px)',
          }}
          onClick={() => setShowOcrDict(false)}
        >
          <div onClick={e => e.stopPropagation()} style={{ width: '90vw', maxWidth: 780 }}>
            <OcrDictionary onClose={() => setShowOcrDict(false)} />
          </div>
        </div>
      )}

      {/* ── Loading overlay (not shown during color picking, handled by panel itself) ── */}
      {loading && pickingSlotIndex === null && (
        <div className="desktop-toolbar-overlay">
          <div className="desktop-toolbar-loading">
            <span className="desktop-toolbar-spinner" />
            处理中...
          </div>
        </div>
      )}

      {/* ── Color picker panel (9 slots) ── */}
      {activeTool === 'color_picker' && pickingSlotIndex === null && !loading && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 200,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: 'rgba(0,0,0,0.5)',
            backdropFilter: 'blur(4px)',
          }}
          onClick={() => {
            setActiveTool(null)
            setColorCopiedIndex(null)
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              background: 'var(--glass-bg-soft, rgba(10,10,18,0.82))',
              backdropFilter: 'blur(24px)',
              borderRadius: 20,
              padding: 20,
              width: 440,
              maxHeight: '80vh',
              overflow: 'auto',
              border: '1px solid var(--glass-4, rgba(255,255,255,0.08))',
              boxShadow: 'var(--shadow-modal)',
            }}
          >
            {/* Title bar */}
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                marginBottom: 14,
                paddingBottom: 10,
                borderBottom: '1px solid var(--glass-2)',
              }}
            >
              <span
                style={{
                  fontSize: 'var(--fs-h2)',
                  fontWeight: 'var(--fw-semibold)',
                  color: 'var(--spark-primary)',
                }}
              >
                取色器
              </span>
              <button
                onClick={() => {
                  setActiveTool(null)
                  setColorCopiedIndex(null)
                }}
                style={{
                  width: 28,
                  height: 28,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  border: '1px solid var(--glass-2)',
                  background: 'var(--glass-0)',
                  borderRadius: 8,
                  color: 'var(--spark-tertiary)',
                  cursor: 'pointer',
                  fontSize: 16,
                  lineHeight: 1,
                  transition: 'var(--transition-fast)',
                }}
                onMouseEnter={e => {
                  e.currentTarget.style.background = 'var(--glass-3)'
                  e.currentTarget.style.color = 'var(--spark-primary)'
                }}
                onMouseLeave={e => {
                  e.currentTarget.style.background = 'var(--glass-0)'
                  e.currentTarget.style.color = 'var(--spark-tertiary)'
                }}
              >
                ✕
              </button>
            </div>

            {/* 9 slots */}
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {colorSlots.map((slot, i) => (
                <div
                  key={i}
                  onClick={() => handleColorSlotPick(i)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '8px 10px',
                    borderRadius: 10,
                    cursor: 'pointer',
                    background: 'var(--glass-1)',
                    border: '1px solid var(--glass-2)',
                    transition: 'var(--transition-fast)',
                  }}
                  onMouseEnter={e => {
                    e.currentTarget.style.borderColor = 'var(--accent)'
                  }}
                  onMouseLeave={e => {
                    e.currentTarget.style.borderColor = 'var(--glass-2)'
                  }}
                >
                  {/* Color swatch */}
                  <div
                    style={{
                      width: 32,
                      height: 32,
                      borderRadius: 8,
                      flexShrink: 0,
                      background: slot.filled ? slot.hex : 'var(--glass-2)',
                      border: '1px solid var(--glass-3)',
                    }}
                  />

                  {/* Color info */}
                  <div
                    style={{
                      flex: 1,
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 1,
                      minWidth: 0,
                    }}
                  >
                    <span
                      style={{
                        fontFamily: 'var(--font-mono)',
                        fontSize: 13,
                        color: slot.filled ? 'var(--spark-primary)' : 'var(--spark-muted)',
                      }}
                    >
                      {slot.filled ? slot.hex : `槽位 ${i + 1} — 点击取色`}
                    </span>
                    {slot.filled && (
                      <span
                        style={{
                          fontFamily: 'var(--font-mono)',
                          fontSize: 11,
                          color: 'var(--spark-tertiary)',
                        }}
                      >
                        RGB({slot.rgb.join(', ')})
                      </span>
                    )}
                  </div>

                  {/* Action buttons */}
                  {slot.filled ? (
                    <button
                      onClick={e => {
                        e.stopPropagation()
                        const text = `${slot.hex}  RGB: ${slot.rgb.join(', ')}`
                        navigator.clipboard
                          .writeText(text)
                          .then(() => {
                            setColorCopiedIndex(i)
                            setTimeout(() => setColorCopiedIndex(null), 2000)
                          })
                          .catch(() => {})
                      }}
                      style={{
                        background:
                          colorCopiedIndex === i ? 'rgba(76,175,80,0.15)' : 'var(--glass-0)',
                        border: '1px solid var(--glass-2)',
                        borderRadius: 8,
                        padding: '4px 12px',
                        color: colorCopiedIndex === i ? '#4caf50' : 'var(--spark-tertiary)',
                        cursor: 'pointer',
                        fontSize: 11,
                        transition: 'var(--transition-fast)',
                      }}
                      onMouseEnter={e => {
                        if (colorCopiedIndex !== i) {
                          e.currentTarget.style.background = 'var(--glass-2)'
                          e.currentTarget.style.color = 'var(--spark-secondary)'
                        }
                      }}
                      onMouseLeave={e => {
                        if (colorCopiedIndex !== i) {
                          e.currentTarget.style.background = 'var(--glass-0)'
                          e.currentTarget.style.color = 'var(--spark-tertiary)'
                        }
                      }}
                    >
                      {colorCopiedIndex === i ? '✓' : '复制'}
                    </button>
                  ) : (
                    <span style={{ color: 'var(--spark-muted)', fontSize: 11 }}>空</span>
                  )}
                </div>
              ))}
            </div>

            {/* Clear button */}
            <button
              onClick={() => setColorSlots(initSlots())}
              style={{
                marginTop: 12,
                width: '100%',
                padding: '6px 0',
                background: 'var(--glass-0)',
                border: '1px solid var(--glass-2)',
                borderRadius: 8,
                color: 'var(--spark-tertiary)',
                cursor: 'pointer',
                fontSize: 12,
                transition: 'var(--transition-fast)',
              }}
              onMouseEnter={e => {
                e.currentTarget.style.background = 'var(--glass-1)'
                e.currentTarget.style.color = 'var(--spark-secondary)'
              }}
              onMouseLeave={e => {
                e.currentTarget.style.background = 'var(--glass-0)'
                e.currentTarget.style.color = 'var(--spark-tertiary)'
              }}
            >
              清空所有颜色
            </button>
          </div>
        </div>
      )}

      {/* ── Result popup ── */}
      {result && (
        <div
          className="desktop-toolbar-overlay"
          onClick={() => {
            setResult(null)
            setCopied(false)
          }}
        >
          <div className="desktop-toolbar-result" onClick={e => e.stopPropagation()}>
            <div className="desktop-toolbar-result-header">
              <span className="desktop-toolbar-result-title">{result.title}</span>
              <div style={{ display: 'flex', gap: 6 }}>
                <Button variant="default" size="sm" onClick={handleCopyResult}>
                  {copied ? <IconCheck size={13} /> : <IconCopy size={13} />}
                  {copied ? '已复制' : '复制'}
                </Button>
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => {
                    setResult(null)
                    setCopied(false)
                  }}
                >
                  关闭
                </Button>
              </div>
            </div>
            <pre className={`desktop-toolbar-result-content ${result.type}`}>{result.content}</pre>
          </div>
        </div>
      )}
    </>
  )
}
