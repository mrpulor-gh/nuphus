import { useEffect, useRef, useCallback, useState } from 'react'
import type { Direction, GameState, Point } from './types'
import {
  createGameState,
  changeDirection,
  startGame,
  togglePause,
  tick,
  getIntervalMs,
} from './game'

// ── Colors ──
const COLORS = {
  bg: '#0d0d0d',
  gridLine: '#1a1a1a',
  snakeHead: '#e5a040',
  snakeBody: '#7a5c2e',
  snakeTail: '#4a3a20',
  food: '#ef5350',
  foodGlow: 'rgba(239, 83, 80, 0.25)',
  text: '#e8e8e8',
  muted: '#666666',
  overlay: 'rgba(13, 13, 13, 0.85)',
}

// ── Key mapping ──
const KEY_DIR: Record<string, Direction> = {
  ArrowUp: 'UP',
  ArrowDown: 'DOWN',
  ArrowLeft: 'LEFT',
  ArrowRight: 'RIGHT',
  w: 'UP',
  W: 'UP',
  s: 'DOWN',
  S: 'DOWN',
  a: 'LEFT',
  A: 'LEFT',
  d: 'RIGHT',
  D: 'RIGHT',
}

export default function SnakeGame() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const stateRef = useRef<GameState>(createGameState())
  const rafRef = useRef<number>(0)
  const lastTick = useRef<number>(0)
  const [display, setDisplay] = useState(stateRef.current)

  // ── Game loop ──
  const loop = useCallback((now: number) => {
    const s = stateRef.current
    const interval = getIntervalMs(s.level)
    if (now - lastTick.current >= interval) {
      lastTick.current = now
      const next = tick(s)
      stateRef.current = next
      setDisplay(next)
    }
    rafRef.current = requestAnimationFrame(loop)
  }, [])

  // ── Draw ──
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const { snake, food, grid, status, score, level } = display
    const { cols, rows, cellSize } = grid
    const w = cols * cellSize
    const h = rows * cellSize

    // Background + grid
    ctx.fillStyle = COLORS.bg
    ctx.fillRect(0, 0, w, h)
    ctx.strokeStyle = COLORS.gridLine
    ctx.lineWidth = 0.5
    for (let x = 0; x <= cols; x++) {
      ctx.beginPath()
      ctx.moveTo(x * cellSize, 0)
      ctx.lineTo(x * cellSize, h)
      ctx.stroke()
    }
    for (let y = 0; y <= rows; y++) {
      ctx.beginPath()
      ctx.moveTo(0, y * cellSize)
      ctx.lineTo(w, y * cellSize)
      ctx.stroke()
    }

    // Food glow + food (skip if won)
    if (food) {
      const fx = food.x * cellSize + cellSize / 2
      const fy = food.y * cellSize + cellSize / 2
      const glow = ctx.createRadialGradient(fx, fy, 2, fx, fy, cellSize)
      glow.addColorStop(0, COLORS.foodGlow)
      glow.addColorStop(1, 'transparent')
      ctx.fillStyle = glow
      ctx.fillRect(
        food.x * cellSize - cellSize / 2,
        food.y * cellSize - cellSize / 2,
        cellSize * 2,
        cellSize * 2,
      )

      ctx.fillStyle = COLORS.food
      ctx.beginPath()
      ctx.arc(fx, fy, cellSize / 2 - 2, 0, Math.PI * 2)
      ctx.fill()
    }

    // Snake
    const bodyCount = snake.length
    snake.forEach((p: Point, i: number) => {
      const t = bodyCount > 1 ? i / (bodyCount - 1) : 0
      if (i === 0) {
        ctx.fillStyle = COLORS.snakeHead
      } else {
        // Gradient from body to tail
        const r = Math.round(122 + t * (74 - 122)) // 122→74
        const g = Math.round(92 + t * (58 - 92)) // 92→58
        const b = Math.round(46 + t * (32 - 46)) // 46→32
        ctx.fillStyle = `rgb(${r},${g},${b})`
      }
      const pad = i === 0 ? 2 : 3
      ctx.fillRect(
        p.x * cellSize + pad,
        p.y * cellSize + pad,
        cellSize - pad * 2,
        cellSize - pad * 2,
      )
    })

    // Overlay for non-running states
    if (status !== 'RUNNING') {
      ctx.fillStyle = COLORS.overlay
      ctx.fillRect(0, 0, w, h)

      ctx.fillStyle = COLORS.text
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'

      if (status === 'IDLE') {
        ctx.font = 'bold 20px system-ui, sans-serif'
        ctx.fillText('贪吃蛇', w / 2, h / 2 - 24)
        ctx.font = '13px system-ui, sans-serif'
        ctx.fillStyle = COLORS.muted
        ctx.fillText('方向键 / WASD 控制', w / 2, h / 2 + 8)
        ctx.fillText('空格键暂停 · 按 Enter 开始', w / 2, h / 2 + 28)
      } else if (status === 'PAUSED') {
        ctx.font = 'bold 20px system-ui, sans-serif'
        ctx.fillText('暂停', w / 2, h / 2)
        ctx.font = '13px system-ui, sans-serif'
        ctx.fillStyle = COLORS.muted
        ctx.fillText('按空格继续', w / 2, h / 2 + 28)
      } else if (status === 'GAMEOVER') {
        ctx.font = 'bold 20px system-ui, sans-serif'
        ctx.fillStyle = COLORS.food
        ctx.fillText('游戏结束', w / 2, h / 2 - 24)
        ctx.font = '14px system-ui, sans-serif'
        ctx.fillStyle = COLORS.text
        ctx.fillText(`得分: ${score}`, w / 2, h / 2 + 8)
        ctx.font = '13px system-ui, sans-serif'
        ctx.fillStyle = COLORS.muted
        ctx.fillText('按 Enter 重新开始', w / 2, h / 2 + 32)
      } else if (status === 'WIN') {
        ctx.font = 'bold 20px system-ui, sans-serif'
        ctx.fillStyle = COLORS.food
        ctx.fillText('你赢了！', w / 2, h / 2 - 24)
        ctx.font = '14px system-ui, sans-serif'
        ctx.fillStyle = COLORS.text
        ctx.fillText(`得分: ${score}`, w / 2, h / 2 + 8)
        ctx.font = '13px system-ui, sans-serif'
        ctx.fillStyle = COLORS.muted
        ctx.fillText('按 Enter 重新开始', w / 2, h / 2 + 32)
      }
    }

    // HUD (always visible, top-right)
    ctx.fillStyle = COLORS.muted
    ctx.font = '12px JetBrains Mono, monospace'
    ctx.textAlign = 'right'
    ctx.textBaseline = 'top'
    ctx.fillText(`分数 ${score}`, w - 10, 8)
    ctx.fillText(`等级 ${level}`, w - 10, 24)
  }, [display])

  // ── Keyboard ──
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const s = stateRef.current

      // Direction keys
      const dir = KEY_DIR[e.key]
      if (dir) {
        e.preventDefault()
        stateRef.current = changeDirection(s, dir)
        // Auto-start on first direction input
        if (stateRef.current.status === 'IDLE') {
          stateRef.current = { ...stateRef.current, status: 'RUNNING' }
          lastTick.current = performance.now()
          rafRef.current = requestAnimationFrame(loop)
        }
        return
      }

      // Space: pause/resume
      if (e.key === ' ') {
        e.preventDefault()
        if (s.status === 'RUNNING' || s.status === 'PAUSED') {
          const next = togglePause(s)
          stateRef.current = next
          if (next.status === 'RUNNING') {
            lastTick.current = performance.now()
          }
        }
        return
      }

      // Enter: start / restart
      if (e.key === 'Enter') {
        e.preventDefault()
        if (
          s.status === 'IDLE' ||
          s.status === 'GAMEOVER' ||
          s.status === 'WIN' ||
          s.status === 'PAUSED'
        ) {
          const next = startGame(s, true)
          stateRef.current = next
          lastTick.current = performance.now()
          if (!rafRef.current) {
            rafRef.current = requestAnimationFrame(loop)
          }
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  // ── Cleanup on unmount ──
  useEffect(() => {
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current)
    }
  }, [])

  const { grid } = display
  const canvasW = grid.cols * grid.cellSize
  const canvasH = grid.rows * grid.cellSize

  return (
    <div className="flex flex-col items-center gap-3 py-2">
      <canvas
        ref={canvasRef}
        width={canvasW}
        height={canvasH}
        className="rounded-md"
        style={{ imageRendering: 'pixelated' }}
        tabIndex={-1}
      />
      <div className="flex gap-4 text-xs text-muted-foreground">
        <span>
          <kbd className="px-1.5 py-0.5 rounded bg-bg-hover border border-border text-xxs font-mono">
            ↑↓←→
          </kbd>{' '}
          <kbd className="px-1.5 py-0.5 rounded bg-bg-hover border border-border text-xxs font-mono">
            WASD
          </kbd>{' '}
          移动
        </span>
        <span>
          <kbd className="px-1.5 py-0.5 rounded bg-bg-hover border border-border text-xxs font-mono">
            Space
          </kbd>{' '}
          暂停
        </span>
        <span>
          <kbd className="px-1.5 py-0.5 rounded bg-bg-hover border border-border text-xxs font-mono">
            Enter
          </kbd>{' '}
          开始/重来
        </span>
      </div>
    </div>
  )
}
