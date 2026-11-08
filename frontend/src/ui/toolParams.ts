/**
 * 工具调用参数的一行摘要（桌面端临时伙伴 / 移动端执行过程弹窗共用）。
 *
 * 只取「定位信息」类字段：命令、工作目录、路径、URL、查询词。多个命中字段用
 * 两个空格拼成一行，前端不做换行以外的截断——截断会埋掉路径 / 命令的关键段。
 * 没有常用字段时退回精简 JSON，避免整段参数撑爆执行过程区。
 */

const PICK_KEYS = ['command', 'cwd', 'path', 'file', 'file_path', 'url', 'query', 'repo']

export function formatToolParams(params: unknown): string | null {
  if (params === undefined || params === null) return null
  if (typeof params === 'string') return params
  if (typeof params !== 'object') return String(params)
  const p = params as Record<string, unknown>

  const lines: string[] = []
  for (const key of PICK_KEYS) {
    const v = p[key]
    if (v === undefined || v === null) continue
    if (typeof v === 'string' && v.length > 0) {
      lines.push(`${key}: ${v}`)
    } else if (typeof v === 'number') {
      lines.push(`${key}: ${v}`)
    }
  }
  if (lines.length > 0) return lines.join('  ')
  try {
    const s = JSON.stringify(p)
    return s.length > 160 ? s.slice(0, 157) + '…' : s
  } catch {
    return null
  }
}
