import type { GameState, Direction, Point } from './types'

// ── Configuration ──

const DEFAULT_COLS = 20
const DEFAULT_ROWS = 20
const DEFAULT_CELL = 24
const FOOD_PER_LEVEL = 5
const BASE_INTERVAL_MS = 180
const MIN_INTERVAL_MS = 50
const SPEED_DECREASE = 12

// ── Direction vectors ──

const DIR_VECTORS: Record<Direction, Point> = {
  UP: { x: 0, y: -1 },
  DOWN: { x: 0, y: 1 },
  LEFT: { x: -1, y: 0 },
  RIGHT: { x: 1, y: 0 },
}

const OPPOSITE: Record<Direction, Direction> = {
  UP: 'DOWN',
  DOWN: 'UP',
  LEFT: 'RIGHT',
  RIGHT: 'LEFT',
}

// ── Factory ──

export function createGameState(
  cols = DEFAULT_COLS,
  rows = DEFAULT_ROWS,
  cellSize = DEFAULT_CELL,
): GameState {
  const midX = Math.floor(cols / 2)
  const midY = Math.floor(rows / 2)
  return {
    snake: [
      { x: midX, y: midY },
      { x: midX - 1, y: midY },
      { x: midX - 2, y: midY },
    ],
    food: randomFood([{ x: midX, y: midY }], cols, rows) ?? { x: 0, y: 0 }, // unreachable at init
    direction: 'RIGHT',
    nextDirection: 'RIGHT',
    status: 'IDLE',
    score: 0,
    level: 1,
    grid: { cols, rows, cellSize },
  }
}

// ── Input ──

export function changeDirection(state: GameState, dir: Direction): GameState {
  if (OPPOSITE[dir] === state.direction) return state
  return { ...state, nextDirection: dir }
}

export function startGame(state: GameState, autoStart = false): GameState {
  if (state.status === 'IDLE' || state.status === 'GAMEOVER' || state.status === 'WIN') {
    const next = createGameState(state.grid.cols, state.grid.rows, state.grid.cellSize)
    return autoStart ? { ...next, status: 'RUNNING' } : next
  }
  return { ...state, status: 'RUNNING' }
}

export function togglePause(state: GameState): GameState {
  if (state.status === 'RUNNING') return { ...state, status: 'PAUSED' }
  if (state.status === 'PAUSED') return { ...state, status: 'RUNNING' }
  return state
}

// ── Core Tick ──

export function tick(state: GameState): GameState {
  if (state.status !== 'RUNNING') return state

  let { snake, food, nextDirection, score, level, grid } = state
  const { cols, rows } = grid
  const dir = nextDirection

  const head = snake[0]
  const vector = DIR_VECTORS[dir]
  const newHead: Point = { x: head.x + vector.x, y: head.y + vector.y }

  // Wall collision
  if (newHead.x < 0 || newHead.x >= cols || newHead.y < 0 || newHead.y >= rows) {
    return { ...state, status: 'GAMEOVER' }
  }

  // Food may be null if player already won (shouldn't reach here but guard)
  if (food === null) return { ...state, status: 'WIN' }

  const isEating = newHead.x === food.x && newHead.y === food.y
  const newSnake = [newHead, ...snake]

  if (!isEating) {
    newSnake.pop()
    if (bodyCollision(newSnake)) {
      return { ...state, status: 'GAMEOVER' }
    }
  } else {
    score += level * 10
    level = Math.floor(score / (FOOD_PER_LEVEL * 10)) + 1
    const nextFood = randomFood(snake, cols, rows)
    if (nextFood === null) {
      return { ...state, snake: newSnake, score, level, status: 'WIN' }
    }
    food = nextFood
  }

  return {
    ...state,
    snake: newSnake,
    food,
    direction: dir,
    nextDirection: dir,
    score,
    level,
    status: 'RUNNING',
  }
}

// ── Helpers ──

export function getIntervalMs(level: number): number {
  return Math.max(MIN_INTERVAL_MS, BASE_INTERVAL_MS - (level - 1) * SPEED_DECREASE)
}

function randomFood(snake: Point[], cols: number, rows: number): Point | null {
  const occupied = new Set(snake.map(p => `${p.x},${p.y}`))
  const available: Point[] = []
  for (let x = 0; x < cols; x++) {
    for (let y = 0; y < rows; y++) {
      if (!occupied.has(`${x},${y}`)) available.push({ x, y })
    }
  }
  if (available.length === 0) return null // grid full → win
  return available[Math.floor(Math.random() * available.length)]
}

function bodyCollision(snake: Point[]): boolean {
  const [head, ...body] = snake
  return body.some(p => p.x === head.x && p.y === head.y)
}
