/** Coordinate point */
export interface Point {
  x: number
  y: number
}

/** Movement direction */
export type Direction = 'UP' | 'DOWN' | 'LEFT' | 'RIGHT'

/** Game status */
export type GameStatus = 'IDLE' | 'RUNNING' | 'PAUSED' | 'GAMEOVER' | 'WIN'

/** Grid configuration */
export interface GridConfig {
  cols: number
  rows: number
  cellSize: number
}

/** Full game state */
export interface GameState {
  snake: Point[]
  food: Point | null // null = grid full (win)
  direction: Direction
  nextDirection: Direction
  status: GameStatus
  score: number
  level: number
  grid: GridConfig
}
