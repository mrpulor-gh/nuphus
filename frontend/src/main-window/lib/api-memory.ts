// Memory Management API — 用户记忆面板专用 Tauri 命令绑定

import { invoke } from '../../core/bridge'
import type {
  Annotation,
  UserMemory,
  MemoryFilter,
  MemoryUpdates,
  MemoryListResult,
} from '../../core/types-memory'

export function listMemories(filter: MemoryFilter) {
  return invoke<MemoryListResult>('list_memories', { filter })
}

export function updateMemory(id: string, updates: MemoryUpdates) {
  return invoke<UserMemory>('update_memory', { id, updates })
}

export function deleteMemory(id: string) {
  return invoke<void>('delete_memory', { id })
}

export function toggleMark(id: string) {
  return invoke<UserMemory>('toggle_mark_memory', { id })
}

// ── Annotation API ──

export function getAnnotations() {
  return invoke<Annotation[]>('get_annotations')
}

export function addAnnotation(
  keyword: string,
  description: string,
  paths?: string[],
  tags?: string[],
  group?: string,
  priority?: number,
  keywords?: string[],
) {
  return invoke<Annotation>('add_annotation', {
    keyword,
    description,
    keywords: keywords ?? null,
    paths: paths ?? null,
    tags: tags ?? null,
    group: group ?? null,
    priority: priority ?? null,
  })
}

export function updateAnnotation(
  keyword: string,
  description?: string,
  paths?: string[],
  tags?: string[],
  group?: string,
  priority?: number,
) {
  return invoke<Annotation>('update_annotation', {
    keyword,
    description: description ?? null,
    paths: paths ?? null,
    tags: tags ?? null,
    group: group ?? null,
    priority: priority ?? null,
  })
}

export function removeAnnotation(keyword: string) {
  return invoke<void>('remove_annotation', { keyword })
}
