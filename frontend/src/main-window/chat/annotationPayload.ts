/**
 * HTML 标注载荷 → 对话引用（发送给丞相）
 *
 * 复用既有 ChatReference 通道（输入区引用 chip + `resolve_references` 前缀注入），
 * 不新增 IPC 协议位：标注载荷序列化为紧凑文本后走 `type: 'quote'` 同一条链——
 * quote 分支的注入语义就是「原文进 Leader 上下文」（见 process.rs 的 quote 分支），
 * 与「让 Leader 读到 selector + comment 完成定向修改」的诉求同构。
 *
 * 为什么不加 `type: 'annotation'` 新协议位：
 *   - Rust 侧 `ChatReference.ref_type` 虽是 String，但 `resolve_references` 的 match
 *     对新类型落 `[Unknown reference type]`——要可用就得改 Rust（本任务 Rust 零改动）；
 *   - 前端 `RefIcon` 是穷尽 switch、`msg-ref-chip--{type}` CSS、mobile_server 双端
 *     同步（messageSelection.ts 的注释已警告）都要跟着加位；
 *   - quote chip 在输入栏/消息气泡的既有渲染直接可用，label 的 60 字符摘要 +
 *     title 全文 already 适配多行文本（首行摘要、全文进 title）。
 *
 * `buildAnnotationRef` 的序列化是 **Leader 视角的载荷本体**：file + 逐条
 * css_selector + comment 是源码修改的充分信息，rect/dpr/outer_html_snippet 为辅助，
 * 三者均已截断（snippet ≤300 字符由注入端保证，整体 ≤2000 字符由 truncateQuote 兜底）。
 */

import type { ChatReference } from '../../core/types'
import { buildQuoteRef } from './messageSelection'

/** 单条标注字段（与 preview_protocol.rs 注入脚本的 postMessage 载荷钉死一致，勿单方改名） */
export interface AnnotationItem {
  css_selector: string
  outer_html_snippet: string
  rect: { x: number; y: number; w: number; h: number }
  dpr: number
  comment: string
  /** P3 区域框选附加字段（元素标注无此字段）；视口坐标，仅辅助信息 */
  region?: { x: number; y: number; w: number; h: number }
}

/** overlay postMessage 顶层载荷（P1 协议冻结字段） */
export interface AnnotationMessage {
  type: 'nuphus:annotations'
  file: string
  annotations: AnnotationItem[]
}

/**
 * 序列化为紧凑文本块。**首行即摘要**——消息气泡 chip 显示 label 前 60 字符，
 * 首行控制在 60 字符内可保证 chip 只露摘要；全文（含每条详情）经 title/注入可见。
 */
export function serializeAnnotations(msg: AnnotationMessage): string {
  const n = msg.annotations.length
  const dpr = msg.annotations[0]?.dpr ?? 1
  const lines: string[] = [`【界面标注】${msg.file} · ${n} 条 · 视口dpr ${dpr}`]
  msg.annotations.forEach((a, i) => {
    lines.push(`${i + 1}. ${a.css_selector}`)
    lines.push(`   comment: ${a.comment.trim() || '（无）'}`)
    lines.push(`   rect: ${a.rect.x},${a.rect.y} ${a.rect.w}x${a.rect.h}`)
    // P3 区域框选条附加 region 辅助行（元素标注无此行；不进 Leader 的必要信息集）
    if (a.region) {
      lines.push(`   region: ${a.region.x},${a.region.y} ${a.region.w}x${a.region.h}`)
    }
    lines.push(`   html: ${a.outer_html_snippet}`)
  })
  return lines.join('\n')
}

/**
 * 标注载荷 → 引用 chip 数据（复用 quote 通道）。
 * truncateQuote 兜底 ≤2000 字符：极端多条标注时不爆上下文，截断处带标记。
 */
export function buildAnnotationRef(msg: AnnotationMessage): ChatReference | null {
  if (msg.annotations.length === 0) return null
  return buildQuoteRef(serializeAnnotations(msg))
}
