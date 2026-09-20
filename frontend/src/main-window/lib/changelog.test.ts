import { describe, expect, it } from 'vitest'
import { parseChangelogSection, countSectionItems } from './changelog'

// 固定夹具：结构与真实 CHANGELOG.md 一致（`## [版本] - 日期` / `### 小节` / 单行条目）
const CHANGELOG = `# Changelog

所有值得注意的变更记录在此文件。

## [Unreleased]

### Added
- **开发中的改动**：尚未发布。

## [0.2.16] - 2026-09-19

### Added
- **模型刷新与官方清单同步**：点「刷新」后对齐官方 /v1/models。

### Fixed
- **DeepSeek 内置模型清单对齐**（#31）：内置清单同步更新。
- 会话交互与文件路径识别（#28）。

## [0.2.15] - 2026-09-16

### Fixed
- 上一轮的旧改动。
`

describe('parseChangelogSection 版本段落切分', () => {
  it('切出目标版本段落：含小节标题与条目，且不含相邻版本内容', () => {
    const section = parseChangelogSection(CHANGELOG, '0.2.16')

    expect(section).not.toBeNull()
    expect(section?.version).toBe('0.2.16')
    expect(section?.date).toBe('2026-09-19')
    expect(section?.groups.map(g => g.title)).toEqual(['Added', 'Fixed'])
    expect(section?.groups[1].items).toHaveLength(2)

    const flat = section?.groups.flatMap(g => g.items).join('\n') ?? ''
    expect(flat).toContain('模型刷新与官方清单同步')
    // 段落边界：不吞相邻版本（Unreleased / 0.2.15）的内容
    expect(flat).not.toContain('开发中的改动')
    expect(flat).not.toContain('上一轮的旧改动')
  })

  it('去掉 Markdown 粗体标记（纯文本展示不留 ** 噪声）', () => {
    const section = parseChangelogSection(CHANGELOG, '0.2.16')
    const flat = section?.groups.flatMap(g => g.items).join('\n') ?? ''

    expect(flat).not.toContain('**')
    expect(flat).toContain('模型刷新与官方清单同步：点「刷新」后对齐官方 /v1/models。')
  })

  it('版本不存在 → null（调用方据此展示空态，而不是抛错）', () => {
    expect(parseChangelogSection(CHANGELOG, '9.9.9')).toBeNull()
    expect(parseChangelogSection(CHANGELOG, '')).toBeNull()
  })

  it('容错 v 前缀（getVersion 不带前缀，但调用方可能传入）', () => {
    expect(parseChangelogSection(CHANGELOG, 'v0.2.16')?.version).toBe('0.2.16')
  })

  it('未发布轮次：版本号为 Unreleased 且无日期', () => {
    const section = parseChangelogSection(CHANGELOG, 'Unreleased')

    expect(section?.version).toBe('Unreleased')
    expect(section?.date).toBeNull()
    expect(countSectionItems(section!)).toBe(1)
  })

  it('末段（文件最后一个版本段落）也能切到结尾', () => {
    const section = parseChangelogSection(CHANGELOG, '0.2.15')

    expect(section?.date).toBe('2026-09-16')
    expect(countSectionItems(section!)).toBe(1)
  })

  it('段落存在但没有任何条目 → 条目数为 0（界面按空态处理）', () => {
    const section = parseChangelogSection('## [1.0.0] - 2026-01-01\n\n### Added\n\n', '1.0.0')

    expect(section).not.toBeNull()
    expect(countSectionItems(section!)).toBe(0)
    expect(section?.groups).toEqual([])
  })
})
