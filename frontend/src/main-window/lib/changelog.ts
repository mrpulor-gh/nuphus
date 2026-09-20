/**
 * changelog.ts — CHANGELOG 段落解析（纯函数，供「版本与更新」展示本版改动）
 *
 * 输入是后端 `get_changelog` 返回的 Markdown 全文（编译期嵌入，离线可读）。
 * 只做「切出目标版本段落 → 拆小节 → 拆条目」这一件事：
 *   - 不引 Markdown 渲染依赖（更新页只需要层级与条目，不需要排版能力）；
 *   - 不改写文案语义，仅去掉 `**` 粗体标记（纯文本展示时它们只是噪声）。
 *
 * 段落边界：`## [版本]` 起，到下一个 `## [` 行为止（与 Keep a Changelog 的层级一致）。
 */

export interface ChangelogGroup {
  /** 小节标题（`### Added` / `### Fixed` …） */
  title: string
  items: string[]
}

export interface ChangelogSection {
  /** 段落标题里的版本号（`0.2.16`、`Unreleased` …） */
  version: string
  /** 段落标题里的日期（`2026-09-19`）；`Unreleased` 段落没有 */
  date: string | null
  groups: ChangelogGroup[]
}

/** `## [0.2.16] - 2026-09-19` / `## [Unreleased]` */
const SECTION_RE = /^##\s+\[([^\]]+)\](?:\s*-\s*(\S+))?/
const GROUP_RE = /^\s*#{3,}\s+(.+?)\s*$/
const ITEM_RE = /^\s*[-*]\s+(.+?)\s*$/
/** 分隔线（`---`）之类的装饰行：不并入条目 */
const RULE_RE = /^\s*([-*_]\s*){3,}$/

/** 纯文本化：去掉 Markdown 粗体标记（`**`），其余原样保留 */
const plainText = (text: string) => text.replace(/\*\*/g, '').trim()

/**
 * 从 CHANGELOG 全文切出 `version` 对应段落。
 * 段落不存在（版本未记录）时返回 null —— 由调用方展示空态，而不是抛错。
 */
export function parseChangelogSection(text: string, version: string): ChangelogSection | null {
  // 容错：调用方可能带来 `v` 前缀（本仓库 getVersion 不带前缀）
  const target = version.trim().replace(/^v/i, '')
  if (!target) return null

  const lines = text.split(/\r?\n/)
  let start = -1
  let end = lines.length
  let date: string | null = null

  for (let i = 0; i < lines.length; i++) {
    const matched = SECTION_RE.exec(lines[i])
    if (!matched) continue
    if (start === -1) {
      if (matched[1].trim() !== target) continue
      start = i
      date = matched[2] ?? null
      continue
    }
    end = i // 下一个版本段落 → 当前段落结束
    break
  }
  if (start === -1) return null

  const groups: ChangelogGroup[] = []
  for (let i = start + 1; i < end; i++) {
    const line = lines[i]
    if (RULE_RE.test(line)) continue

    const group = GROUP_RE.exec(line)
    if (group) {
      groups.push({ title: plainText(group[1]), items: [] })
      continue
    }

    const item = ITEM_RE.exec(line)
    if (item) {
      if (groups.length === 0) groups.push({ title: '', items: [] })
      groups[groups.length - 1].items.push(plainText(item[1]))
      continue
    }

    // 条目续行（当前格式为单行条目，此处兜住未来的折行以免静默丢文本）
    if (line.trim() && groups.length > 0) {
      const last = groups[groups.length - 1]
      const lastIndex = last.items.length - 1
      if (lastIndex >= 0) last.items[lastIndex] = `${last.items[lastIndex]} ${plainText(line)}`
    }
  }

  return { version: target, date, groups: groups.filter(g => g.items.length > 0) }
}

/** 段落条目总数（用途：段落存在但没有任何条目时，界面按空态处理） */
export function countSectionItems(section: ChangelogSection): number {
  return section.groups.reduce((total, group) => total + group.items.length, 0)
}
