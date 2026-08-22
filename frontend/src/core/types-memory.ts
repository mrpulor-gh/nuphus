// UserMemory — 前后端对齐的用户记忆管理类型

/// 记忆一维分类（与后端 MemoryKind serde snake_case 对齐）
export type MemoryKind = 'conversation' | 'task_trace' | 'distill' | 'pattern' | 'snapshot'

export interface UserMemory {
  id: string
  session_id: string
  kind: MemoryKind
  agent_type: string
  goal_type: string | null
  title: string
  intent: string
  summary: string
  tags: string[]
  quality_score: number
  user_rating: number | null
  is_marked: boolean
  block_injection: boolean
  pattern: string | null
  created_at: string
  updated_at: string
}

export interface MemoryFilter {
  marked_only?: boolean
  min_quality?: number
  search?: string
  tag?: string
  kind?: MemoryKind
  limit?: number
  offset?: number
}

export interface MemoryUpdates {
  title?: string
  summary?: string
  tags?: string[]
}

export interface MemoryListResult {
  memories: UserMemory[]
  total: number
  offset: number
  limit: number
}

// ── 概览 tab 聚合数据（get_memory_overview）──

export interface MemoryOverview {
  total_entries: number
  success_rate: number
  db_size_bytes: number
  embedded_count: number
  /** 最早记忆条目时间戳（毫秒），0 表示无数据 */
  oldest_ms: number
  /** 最新记忆条目时间戳（毫秒），0 表示无数据 */
  newest_ms: number
  /** 提炼条目数（distill） */
  distill_count: number
  /** 用户标记/模式条目数（pattern） */
  pattern_count: number
}

// 执行步骤（紧凑格式，与后端 PersistedStep 对齐）
export interface ExecutionStep {
  tool: string
  params_summary: string
  result_summary: string
  success: boolean
  duration_ms: number | null
}

// ── 关系标注（Annotation）──

export interface Annotation {
  id: string
  keyword: string
  keywords?: string[]
  description: string
  paths: string[]
  tags: string[]
  group: string
  builtin: boolean
  priority: number
  relations: string[]
  created_at: string
  updated_at: string
}
