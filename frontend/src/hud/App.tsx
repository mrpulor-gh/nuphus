import { useState, useEffect, useRef, useCallback } from 'react'
import { listen, invoke } from '../core/bridge'
import type { NuphusEvent } from '../core/types'
import {
  CircleCheck,
  CircleX,
  TriangleAlert,
  Info,
  Sun,
  Clock,
  MessageSquare,
  CornerUpRight,
  FileCode,
  List,
  RotateCcw,
  Pause,
  Play,
  Square,
  X,
} from 'lucide-react'

// ═══════════════════════════════════════════════════════════════════
//  Types
// ═══════════════════════════════════════════════════════════════════

type HudPhase =
  | 'hidden'
  | 'running'
  | 'done'
  | 'error'
  | 'warning'
  | 'info'
  | 'success'
  | 'workflow'
  | 'workflow_wait'
  | 'security'

interface HudState {
  text: string
  phase: HudPhase
  paused?: boolean
  step_kind?: string | null
}

const LIGHT_COLORS: Record<HudPhase, { accent: string; bg: string; glow: string; text: string }> = {
  hidden: { accent: '#9ca3af', bg: 'rgba(0,0,0,0.02)', glow: 'rgba(0,0,0,0.03)', text: '#888888' },
  running: {
    accent: '#111111',
    bg: 'rgba(17,17,17,0.04)',
    glow: 'rgba(0,0,0,0.04)',
    text: '#1a1a1a',
  },
  done: {
    accent: '#2e7d32',
    bg: 'rgba(46,125,50,0.04)',
    glow: 'rgba(0,0,0,0.04)',
    text: '#1a1a1a',
  },
  error: {
    accent: '#c62828',
    bg: 'rgba(198,40,40,0.04)',
    glow: 'rgba(0,0,0,0.06)',
    text: '#1a1a1a',
  },
  warning: {
    accent: '#d97706',
    bg: 'rgba(217,119,6,0.04)',
    glow: 'rgba(0,0,0,0.04)',
    text: '#1a1a1a',
  },
  info: { accent: '#111111', bg: 'rgba(17,17,17,0.03)', glow: 'rgba(0,0,0,0.03)', text: '#1a1a1a' },
  success: {
    accent: '#2e7d32',
    bg: 'rgba(46,125,50,0.04)',
    glow: 'rgba(0,0,0,0.04)',
    text: '#1a1a1a',
  },
  workflow: {
    accent: '#6d28d9',
    bg: 'rgba(109,40,217,0.04)',
    glow: 'rgba(0,0,0,0.04)',
    text: '#1a1a1a',
  },
  workflow_wait: {
    accent: '#d97706',
    bg: 'rgba(217,119,6,0.04)',
    glow: 'rgba(0,0,0,0.04)',
    text: '#1a1a1a',
  },
  security: {
    accent: '#c62828',
    bg: 'rgba(198,40,40,0.06)',
    glow: 'rgba(0,0,0,0.08)',
    text: '#1a1a1a',
  },
}

function getTheme(): 'dark' | 'light' {
  try {
    const t = localStorage.getItem('nuphus_theme')
    if (t === 'light') return 'light'
  } catch {}
  return 'dark'
}

// ═══════════════════════════════════════════════════════════════════
//  Design tokens (Void & Spark – matching main app)
// ═══════════════════════════════════════════════════════════════════

const COLORS: Record<HudPhase, { accent: string; bg: string; glow: string; text: string }> = {
  hidden: {
    accent: '#505060',
    bg: 'rgba(255,255,255,0.04)',
    glow: 'rgba(80,80,96,0.2)',
    text: '#707080',
  },
  running: {
    accent: '#60a5fa',
    bg: 'rgba(96,165,250,0.08)',
    glow: 'rgba(96,165,250,0.25)',
    text: '#f5f5fa',
  },
  done: {
    accent: '#34d399',
    bg: 'rgba(52,211,153,0.08)',
    glow: 'rgba(52,211,153,0.25)',
    text: '#f5f5fa',
  },
  error: {
    accent: '#f87171',
    bg: 'rgba(248,113,113,0.08)',
    glow: 'rgba(248,113,113,0.3)',
    text: '#f5f5fa',
  },
  warning: {
    accent: '#fbbf24',
    bg: 'rgba(251,191,36,0.08)',
    glow: 'rgba(251,191,36,0.25)',
    text: '#f5f5fa',
  },
  info: {
    accent: '#60a5fa',
    bg: 'rgba(96,165,250,0.08)',
    glow: 'rgba(96,165,250,0.2)',
    text: '#f5f5fa',
  },
  success: {
    accent: '#34d399',
    bg: 'rgba(52,211,153,0.08)',
    glow: 'rgba(52,211,153,0.25)',
    text: '#f5f5fa',
  },
  workflow: {
    accent: '#a78bfa',
    bg: 'rgba(167,139,250,0.08)',
    glow: 'rgba(167,139,250,0.25)',
    text: '#f5f5fa',
  },
  workflow_wait: {
    accent: '#fbbf24',
    bg: 'rgba(251,191,36,0.08)',
    glow: 'rgba(251,191,36,0.25)',
    text: '#f5f5fa',
  },
  security: {
    accent: '#ef4444',
    bg: 'rgba(239,68,68,0.12)',
    glow: 'rgba(239,68,68,0.35)',
    text: '#f5f5fa',
  },
}

// ═══════════════════════════════════════════════════════════════════
//  SVG Icons
// ═══════════════════════════════════════════════════════════════════

function IconSpinner({ color, trackColor }: { color: string; trackColor: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" className="hud-spinner">
      <circle cx="7" cy="7" r="5.5" stroke={trackColor} strokeWidth="1.5" fill="none" />
      <path
        d="M12.5 7A5.5 5.5 0 0 0 1.5 7"
        stroke={color}
        strokeWidth="1.5"
        strokeLinecap="round"
        fill="none"
      />
    </svg>
  )
}

function IconCheck({ color }: { color: string }) {
  return <CircleCheck size={14} color={color} absoluteStrokeWidth strokeWidth={1.4} />
}

function IconError({ color }: { color: string }) {
  return <CircleX size={14} color={color} absoluteStrokeWidth strokeWidth={1.4} />
}

function IconWarning({ color }: { color: string }) {
  return <TriangleAlert size={14} color={color} absoluteStrokeWidth strokeWidth={1.4} />
}

function IconInfo({ color }: { color: string }) {
  return <Info size={14} color={color} absoluteStrokeWidth strokeWidth={1.4} />
}

/** Nuphus-proprietary workflow glyph (three nodes + connectors) — no lucide equivalent, kept hand-rolled. */
function IconWorkflow({ color }: { color: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
      <circle cx="4" cy="3" r="1.8" stroke={color} strokeWidth="1.1" />
      <circle cx="10" cy="7" r="1.8" stroke={color} strokeWidth="1.1" />
      <circle cx="4" cy="11" r="1.8" stroke={color} strokeWidth="1.1" />
      <path d="M5.5 4l3.5 2M5.5 10l3.5-2" stroke={color} strokeWidth="0.9" opacity={0.35} />
    </svg>
  )
}

function IconPause({ color }: { color: string }) {
  return <Pause size={9} color={color} fill="currentColor" strokeWidth={1.1} />
}

function IconPlay({ color }: { color: string }) {
  return <Play size={9} color={color} fill="currentColor" strokeWidth={1.1} />
}

function IconStop({ color }: { color: string }) {
  return <Square size={9} color={color} absoluteStrokeWidth strokeWidth={1.1} />
}

function IconClose({ color }: { color: string }) {
  return <X size={9} color={color} absoluteStrokeWidth strokeWidth={1.1} />
}

// ── Step kind icons (used when step_kind is present) ──
const STEP_ICONS: Record<string, (color: string) => React.ReactNode> = {
  tool: c => <Sun size={14} color={c} absoluteStrokeWidth strokeWidth={1.1} />,
  wait: c => <Clock size={14} color={c} absoluteStrokeWidth strokeWidth={1.1} />,
  chat_agent: c => <MessageSquare size={14} color={c} absoluteStrokeWidth strokeWidth={1.1} />,
  call: c => <CornerUpRight size={14} color={c} absoluteStrokeWidth strokeWidth={1.1} />,
  script: c => <FileCode size={14} color={c} absoluteStrokeWidth strokeWidth={1.1} />,
  seq: c => <List size={14} color={c} absoluteStrokeWidth strokeWidth={1.1} />,
  loop: c => <RotateCcw size={14} color={c} absoluteStrokeWidth strokeWidth={1.1} />,
}

// ═══════════════════════════════════════════════════════════════════
//  Phase icon resolver
// ═══════════════════════════════════════════════════════════════════

function PhaseIcon({
  phase,
  step_kind,
  color,
  trackColor,
}: {
  phase: HudPhase
  step_kind?: string | null
  color: string
  trackColor: string
}) {
  // When step_kind is provided for workflow phases, render step-specific icon
  if (step_kind && (phase === 'workflow' || phase === 'workflow_wait')) {
    const iconFn = STEP_ICONS[step_kind]
    if (iconFn) return <>{iconFn(color)}</>
  }
  switch (phase) {
    case 'running':
      return <IconSpinner color={color} trackColor={trackColor} />
    case 'workflow':
      return <IconSpinner color={color} trackColor={trackColor} />
    case 'workflow_wait':
      return <IconWorkflow color={color} />
    case 'success':
      return <IconCheck color={color} />
    case 'done':
      return <IconCheck color={color} />
    case 'error':
      return <IconError color={color} />
    case 'warning':
      return <IconWarning color={color} />
    case 'info':
      return <IconInfo color={color} />
    default:
      return null
  }
}

// ═══════════════════════════════════════════════════════════════════
//  Live timer
// ═══════════════════════════════════════════════════════════════════

function useLiveTimer(running: boolean): number {
  const [elapsed, setElapsed] = useState(0)
  const startRef = useRef<number | null>(null)

  useEffect(() => {
    if (running) {
      if (startRef.current === null) startRef.current = Date.now()
      const id = setInterval(() => {
        setElapsed(Date.now() - startRef.current!)
      }, 200)
      return () => clearInterval(id)
    } else {
      startRef.current = null
      setElapsed(0)
    }
  }, [running])

  return elapsed
}

function formatElapsed(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  if (ms < 10000) return `${(ms / 1000).toFixed(1)}s`
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s`
  return `${Math.floor(s / 60)}m ${s % 60}s`
}

// ═══════════════════════════════════════════════════════════════════
//  Component
// ═══════════════════════════════════════════════════════════════════

export function HudOverlay() {
  const [state, setState] = useState<HudState>({ text: '', phase: 'hidden' })
  const [theme, setTheme] = useState<'dark' | 'light'>(getTheme)
  const hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Phase transition animation key — change triggers CSS class swap
  const animRef = useRef(0)

  const clearHideTimer = useCallback(() => {
    if (hideTimer.current !== null) {
      clearTimeout(hideTimer.current)
      hideTimer.current = null
    }
  }, [])

  const scheduleAutoHide = useCallback((phase: HudPhase) => {
    const delays: Partial<Record<HudPhase, number>> = {
      info: 5000,
      success: 5000,
      warning: 8000,
      done: 15000,
      error: 15000,
    }
    const delay = delays[phase]
    if (delay) {
      hideTimer.current = setTimeout(async () => {
        // Try hiding the Tauri window; retry once on failure
        try {
          await invoke('hud_hide')
        } catch (err) {
          console.warn('[HUD] hud_hide failed, retrying:', err)
          try {
            await invoke('hud_hide')
          } catch {}
        }
        setState(s => ({ ...s, phase: 'hidden' }))
      }, delay)
    }
  }, [])

  /// Shared handler for both HUD event channels
  const applyHudUpdate = useCallback(
    (text: string, phase: HudPhase, step_kind?: string | null) => {
      if (hideTimer.current !== null) {
        clearTimeout(hideTimer.current)
        hideTimer.current = null
      }
      // Guard: backend must only send known phases. An unknown one would make
      // colorScheme[phase] undefined and crash the render → blank window.
      const safePhase = COLORS[phase] ? phase : 'info'
      setState({ text, phase: safePhase, paused: false, step_kind })
      animRef.current++
      if (phase === 'hidden') {
        invoke('hud_hide').catch(() => {})
      } else {
        scheduleAutoHide(phase)
      }
    },
    [scheduleAutoHide],
  )

  useEffect(() => {
    // Channel A: global nuphus-event bus (Rust agent emits HudUpdate → emitter → here)
    const unlistenGlobal = listen<{ seq: number; event: NuphusEvent }>(
      'nuphus-event',
      ({ event }) => {
        if (event.type === 'hud_update') {
          applyHudUpdate(event.text, event.phase as HudPhase, event.step_kind)
        }
      },
    )
    // Channel B: window-local hud-update (frontend invoke → show() → here)
    const unlistenLocal = listen<{ text: string; phase: string }>(
      'hud-update',
      ({ text, phase }) => {
        applyHudUpdate(text, phase as HudPhase)
      },
    )
    return () => {
      clearHideTimer()
      unlistenGlobal.then(fn => fn?.())
      unlistenLocal.then(fn => fn?.())
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clearHideTimer, scheduleAutoHide])

  // Theme sync: listen for localStorage changes from main window
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === 'nuphus_theme') {
        setTheme(e.newValue === 'light' ? 'light' : 'dark')
      }
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [])

  // Safety timeout: auto-done + auto-hide if running exceeds 120s
  useEffect(() => {
    const isRunning = state.phase === 'running'
    if (!isRunning) return
    const timer = setTimeout(() => {
      setState(s => {
        if (s.phase !== 'running') return s
        // scheduleAutoHide for the done phase so the HUD doesn't stay forever
        scheduleAutoHide('done')
        return { ...s, phase: 'done' }
      })
    }, 120_000)
    return () => clearTimeout(timer)
  }, [state.phase, scheduleAutoHide])

  // 三个控制命令在后端都可能失败——例如 hud_stop 在取不到活动工作流时返回
  // "No active workflow"。旧实现一律 `catch { /* ignore */ }`，用户点了**毫无反馈**
  // （"点了没反应"的直接来源）。改为把原因显示在 HUD 上（error 相位）。
  const surfaceControlError = useCallback((action: string, e: unknown) => {
    const msg = typeof e === 'string' ? e : ((e as { message?: string })?.message ?? String(e))
    setState(s => ({ ...s, text: `${action}失败：${msg}`, phase: 'error' }))
  }, [])

  const handlePause = useCallback(async () => {
    try {
      await invoke('hud_pause')
      setState(s => ({ ...s, paused: true }))
    } catch (e) {
      surfaceControlError('暂停', e)
    }
  }, [surfaceControlError])

  const handleResume = useCallback(async () => {
    try {
      await invoke('hud_resume')
      setState(s => ({ ...s, paused: false }))
    } catch (e) {
      surfaceControlError('继续', e)
    }
  }, [surfaceControlError])

  // HUD 终止 = 终止 workflow_run：直接终止，无确认（与桌面主执行终止弹窗不同，行为即设计）
  const handleStop = useCallback(async () => {
    try {
      await invoke('hud_stop')
      setState(s => ({ ...s, phase: 'done' }))
    } catch (e) {
      surfaceControlError('终止', e)
    }
  }, [surfaceControlError])

  // ── Derived state (hooks always called at top — no conditional returns before hooks) ──
  const isRunning = state.phase === 'running' || state.phase === 'workflow'
  const isWorkflow = state.phase === 'workflow' || state.phase === 'workflow_wait'
  const elapsed = useLiveTimer(isRunning)

  // hidden: render a zero-size invisible div instead of null.
  // Returning null triggers #root:empty::after which paints a transparent block.
  if (state.phase === 'hidden') {
    return <div style={{ display: 'none' }} />
  }

  const isBusy = isRunning
  const colorScheme = theme === 'light' ? LIGHT_COLORS : COLORS
  const colors = colorScheme[state.phase]
  const trackColor = theme === 'light' ? 'rgba(0,0,0,0.06)' : 'rgba(255,255,255,0.12)'
  const closeColor = theme === 'light' ? 'rgba(0,0,0,0.2)' : 'rgba(255,255,255,0.3)'
  const animKey = animRef.current

  return (
    <div
      className="hud-root"
      /* 整条可拖动：Tauri 的 drag.js 对 deep 会向上遍历 composedPath，
         只有 <button> 这类可点击元素会阻断（正好让暂停/终止/关闭照常点）。
         窗口是 decorations(false)，没有标题栏，不加这个就完全拖不动。
         双击触发 internal_toggle_maximize，但被 is_resizable() 挡住（HUD 是
         resizable(false)），所以双击无副作用。 */
      data-tauri-drag-region="deep"
      style={
        {
          '--hud-accent': colors.accent,
          '--hud-accent-bg': colors.bg,
          '--hud-glow': colors.glow,
          '--hud-bg': theme === 'light' ? '#fbfbfc' : 'rgba(15,15,20,0.85)',
          '--hud-border':
            theme === 'light' ? '1px solid #e5e5e8' : '1px solid rgba(255,255,255,0.06)',
          '--hud-meta-color': theme === 'light' ? '#888888' : 'rgba(255,255,255,0.35)',
          '--hud-shadow':
            theme === 'light'
              ? '0 0 0 1px rgba(0,0,0,0.04), 0 2px 12px rgba(0,0,0,0.06)'
              : '0 0 0 1px var(--hud-glow), 0 2px 16px rgba(0,0,0,0.5), 0 0 24px var(--hud-glow)',
          '--hud-accent-glow': theme === 'light' ? 'none' : '0 0 8px var(--hud-accent)',
          backdropFilter: theme === 'light' ? 'none' : 'blur(12px)',
          WebkitBackdropFilter: theme === 'light' ? 'none' : 'blur(12px)',
        } as React.CSSProperties
      }
    >
      {/* ── Accent bar (left edge) ── */}
      <span className="hud-accent-bar" />

      {/* ── Icon ── */}
      <span className="hud-icon" key={`icon-${animKey}`}>
        <PhaseIcon
          phase={state.phase}
          step_kind={state.step_kind}
          color={colors.accent}
          trackColor={trackColor}
        />
      </span>

      {/* ── Content area ── */}
      <div className="hud-content">
        <span className="hud-text" style={{ color: colors.text }} key={`text-${animKey}`}>
          {state.text}
        </span>

        {/* ── Meta row: elapsed time / workflow paused indicator ── */}
        <span className="hud-meta">
          {isBusy && elapsed > 0 && <span className="hud-elapsed">{formatElapsed(elapsed)}</span>}
          {state.paused && <span className="hud-paused-label">已暂停</span>}
        </span>
      </div>

      {/* ── Workflow controls ── */}
      {isWorkflow && (
        <span className="hud-controls">
          <button
            className="hud-ctrl-btn"
            onClick={state.paused ? handleResume : handlePause}
            title={state.paused ? '继续' : '暂停'}
          >
            {state.paused ? (
              <IconPlay color={colors.accent} />
            ) : (
              <IconPause color={colors.accent} />
            )}
          </button>
          <button className="hud-ctrl-btn hud-ctrl-btn--stop" onClick={handleStop} title="终止">
            <IconStop color="#f87171" />
          </button>
        </span>
      )}

      {/* ── Close ── */}
      <button
        className="hud-close-btn"
        onClick={() => {
          invoke('hud_hide').catch(() => {})
          setState(s => ({ ...s, phase: 'hidden' }))
        }}
        title="隐藏"
      >
        <IconClose color={closeColor} />
      </button>

      {/* ── Progress bar (bottom edge) ── */}
      {isBusy && <span className="hud-progress" />}

      {/* ═══════════════════════════════════════════════════════
         CSS
         ═══════════════════════════════════════════════════════ */}
      <style>{`
.hud-root {
  all: initial;
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 100%;
  padding: 0 10px 0 0;
  box-sizing: border-box;
  position: relative;
  overflow: hidden;
  user-select: none;
   font-family: var(--font-ui);
  background: var(--hud-bg);
  border: var(--hud-border);
   /* Windows 无边框窗口无法做椭圆角（圆角处显示窗口底色/锯齿），移除为直角 */
   box-shadow: var(--hud-shadow);
   animation: hudEnter 0.2s ease-out;
}

/* ── Accent bar (3px left edge, phase-colored) ── */
.hud-accent-bar {
  display: block;
  flex-shrink: 0;
  width: 3px;
  height: 26px;
  border-radius: 0 2px 2px 0;
  background: var(--hud-accent);
  box-shadow: var(--hud-accent-glow);
  transition: background 0.35s ease, box-shadow 0.35s ease;
}

/* ── Icon ── */
.hud-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 20px;
  height: 20px;
  animation: hudIconPop 0.25s ease-out;
}

/* ── Content ── */
.hud-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 1px;
  overflow: hidden;
}

.hud-text {
  font-size: 12.5px;
  font-weight: 500;
  line-height: 1.3;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  letter-spacing: -0.01em;
  animation: hudTextSlide 0.2s ease-out;
}

.hud-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 10px;
  font-weight: 400;
  color: var(--hud-meta-color);
  font-family: var(--font-mono);
}

.hud-elapsed,
.hud-paused-label {
  display: inline-flex;
  align-items: center;
}

.hud-elapsed::before {
  content: '';
  display: inline-block;
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: var(--hud-accent);
  opacity: 0.5;
  margin-right: 4px;
}

.hud-paused-label {
  color: var(--hud-meta-color);
}

/* ── Workflow controls ── */
.hud-controls {
  display: flex;
  align-items: center;
  gap: 3px;
  flex-shrink: 0;
  margin-right: 2px;
}

.hud-ctrl-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  background: rgba(255, 255, 255, 0.06);
  color: rgba(255, 255, 255, 0.6);
  cursor: pointer;
  border-radius: 5px;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.hud-ctrl-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  color: rgba(255, 255, 255, 0.9);
}

.hud-ctrl-btn--stop:hover {
  background: rgba(248, 113, 113, 0.15);
}

/* ── Close button ── */
.hud-close-btn {
  position: absolute;
  top: 3px;
  right: 3px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  border-radius: 3px;
  transition: all 0.15s ease;
  flex-shrink: 0;
  opacity: 0.4;
}

.hud-close-btn:hover {
  opacity: 1;
  background: rgba(255, 255, 255, 0.08);
}

/* ── Progress bar (animated sweep at bottom) ── */
.hud-progress {
  position: absolute;
  bottom: 0;
  left: 3px;
  right: 0;
  height: 2px;
  background: linear-gradient(
    90deg,
    var(--hud-accent) 0%,
    rgba(255, 255, 255, 0.12) 60%,
    transparent 100%
  );
  background-size: 200% 100%;
  border-radius: 0 0 8px 0;
  animation: hudProgressSweep 2s ease-in-out infinite;
}

/* ════════════════════════════════════════════════
   Animations
   ════════════════════════════════════════════════ */

@keyframes hudEnter {
  0% { opacity: 0; transform: translateY(4px) scale(0.96); }
  100% { opacity: 1; transform: translateY(0) scale(1); }
}

@keyframes hudIconPop {
  0% { opacity: 0; transform: scale(0.6); }
  60% { transform: scale(1.15); }
  100% { opacity: 1; transform: scale(1); }
}

@keyframes hudTextSlide {
  0% { opacity: 0; transform: translateX(-4px); }
  100% { opacity: 1; transform: translateX(0); }
}

@keyframes hudProgressSweep {
  0% { background-position: 200% 0; opacity: 0.5; }
  50% { opacity: 1; }
  100% { background-position: -100% 0; opacity: 0.5; }
}

@keyframes hudSpin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.hud-spinner {
  animation: hudSpin 1s linear infinite;
}

/* ════════════════════════════════════════════════
   Running state accent bar glow pulse
   ════════════════════════════════════════════════ */

.hud-root:has(> .hud-accent-bar) {
  transition: box-shadow 0.3s ease;
}
`}</style>
    </div>
  )
}
