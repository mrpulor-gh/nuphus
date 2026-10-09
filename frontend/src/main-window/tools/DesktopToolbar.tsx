// DesktopToolbar.tsx — 桌面工具浮窗条 v6
// 右侧固定按钮列（dock 常驻 + hover 从上到下逐次显形），**不可拖拽**、无快捷键、无置顶
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
  IconX,
  IconCopy,
  IconCheck,
  IconAppWindow,
} from '../../ui/Icons'

import { Pipette as IconDropper } from 'lucide-react'

import { OcrDictionary } from './OcrDictionary'
import { Button, IconButton } from '../../ui/Button'

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
  ocr: '字典 — 框选区域提取文字（颜色+字典匹配）',
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
  { mode: 'ocr', icon: IconType, label: '字典', desc: '文字识别' },
]

/**
 * hover 入场错峰步长（ms）：第 n 个按钮延迟 n × 步长，由 CSS 的 --stagger-delay 消费。
 * 取 40ms —— 与 --transition-fast(120ms) 相邻，6 个按钮总时长约 320ms：
 * 看得清「从上到下逐次显示」，又不会拖到像加载动画。
 */
const STAGGER_STEP_MS = 40

/** 把错峰步长交给样式层：只写 CSS 变量，不在 JS 里做定时器（卸载后无残留回调） */
const staggerStyle = (index: number) =>
  ({ '--stagger-delay': `${index * STAGGER_STEP_MS}ms` }) as React.CSSProperties
export function DesktopToolbar() {
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

  return (
    <>
      {/* ── Dock：右侧固定按钮列。常驻占位 + hover 从上到下逐次显形 ──
          列排布直接落在 dock 上（竖向 / gap 4px / 右对齐），同 .chat-header-right：
          按钮等宽 30px，flex-end 与 center 视觉等价，用 flex-end 与 header 一致。
          热区 = 这一列按钮的真实盒子，dock 不加任何 padding/margin/inset 撑大。 */}
      <div className="desktop-toolbar-dock">
        {/* Tool buttons —— 下标即错峰序号：第 i 个延迟 i × STAGGER_STEP_MS 入场 */}
        {TOOLS.map((tool, i) => (
          <IconButton
            key={tool.mode}
            variant={activeTool === tool.mode ? 'desktop-toolbar-active' : 'desktop-toolbar'}
            label={tool.label}
            onClick={() => handleToolClick(tool.mode)}
            title={tool.desc}
            style={staggerStyle(i)}
          >
            <tool.icon size={15} />
            <span className="desktop-toolbar-label">{tool.label}</span>
          </IconButton>
        ))}

        <IconButton
          variant="desktop-toolbar"
          label="登记应用"
          disabled={loading}
          title="通过本地选择器登记未被自动发现的桌面应用；不会立即启动应用"
          style={staggerStyle(TOOLS.length)}
          onClick={async () => {
            setLoading(true)
            try {
              const registered = await tauriInvoke<{ name: string } | null>(
                'desktop_register_application',
              )
              if (registered)
                setResult({
                  type: 'info',
                  title: '应用已登记',
                  content: `${registered.name}\n现在可在工作流中按应用名称选择，不需要提供启动脚本。`,
                })
            } catch (error) {
              setResult({ type: 'info', title: '应用登记失败', content: String(error) })
            } finally {
              setLoading(false)
            }
          }}
        >
          <IconAppWindow size={15} />
          <span className="desktop-toolbar-label">登记应用</span>
        </IconButton>

        {/* ── 鼠标坐标：读数 + 关闭**并排成一组**，且**常驻不受 hover 显隐约束** ──
            为什么必须常驻：整列平时 opacity:0，只在 hover dock 时才显形。若坐标
            读数跟着一起隐藏，用户为了看读数/去别处用坐标，鼠标一离开 dock 就
            什么都看不见了（2026-10-09 大王报障「点击后坐标位置获取不到」），
            关闭钮也一起消失、无从点击。故本组自成一体：只要 activeTool 是
            mouse_pos 就常驻显示，点关闭才收。
            关闭钮与读数同级并排（原实现把它排在列末尾、落在读数「下方」）。 */}
        {activeTool === 'mouse_pos' && (
          <div className="desktop-toolbar-pos-group" role="group" aria-label="鼠标坐标">
            <button
              type="button"
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
            <button
              type="button"
              className="desktop-toolbar-pos-close"
              aria-label="关闭鼠标坐标"
              title="关闭鼠标坐标"
              onClick={() => setActiveTool(null)}
            >
              <IconX size={15} />
            </button>
          </div>
        )}
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
            // 遮罩归位到弹窗族语义键：原写死 rgba(0,0,0,0.5) 三主题同值，
            // 与设置中心等弹窗宿主（--overlay-bg）不统一。
            background: 'var(--overlay-bg)',
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
            // 遮罩归位到弹窗族语义键（同上：原写死 rgba(0,0,0,0.5)）。
            background: 'var(--overlay-bg)',
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
              // 面板底归位弹窗族：原 --glass-bg-soft（α 三主题写死，且带 rgba 硬兜底），
              // 滑块（主题设置 → 界面不透明度 → 控制面板）遍历不到它。与
              // .settings-center-panel / .session-rail-drawer / .wfst-panel / .wcf-content 同一按键。
              background: 'var(--panel-bg)',
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
                <IconX size={14} />
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
