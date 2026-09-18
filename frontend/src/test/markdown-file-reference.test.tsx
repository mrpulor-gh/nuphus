import { describe, expect, it, vi } from 'vitest'
import { renderToString } from 'react-dom/server'
import MarkdownContent, {
  extractFilePaths,
  resolveProjectFilePath,
} from '../main-window/chat/MarkdownContent'

describe('MarkdownContent 文件引用', () => {
  const onFileClick = vi.fn()

  it('将 Windows、macOS/Linux 绝对路径渲染为紧凑文件条目', () => {
    const windowsPath = String.raw`C:\work\src\App.tsx`
    const uncPath = String.raw`\\server\share\report.pdf`
    const html = renderToString(
      <MarkdownContent
        content={`输出：${windowsPath}\n共享：${uncPath}\n以及 /opt/nuphus/config/settings.toml`}
        onFileClick={onFileClick}
      />,
    )
    expect(html).toContain(`data-file-path="${windowsPath}"`)
    expect(html).toContain(`data-file-path="${uncPath}"`)
    expect(html).toContain('data-file-path="/opt/nuphus/config/settings.toml"')
    expect(html).toContain('>TSX<')
    expect(html).toContain('>App.tsx<')
    expect(html).not.toContain(`>${windowsPath}<`)
  })

  it('仅在提供项目基准路径时解析相对路径', () => {
    const relative = 'frontend/src/main.tsx'
    const projectBasePath = String.raw`C:\repo`
    const withoutBase = renderToString(
      <MarkdownContent content={relative} onFileClick={onFileClick} />,
    )
    const withBase = renderToString(
      <MarkdownContent
        content={relative}
        onFileClick={onFileClick}
        projectBasePath={projectBasePath}
      />,
    )
    expect(withoutBase).not.toContain('data-file-path')
    expect(withBase).toContain(`data-file-path="${String.raw`C:\repo\frontend\src\main.tsx`}"`)
    expect(resolveProjectFilePath('../README.md', '/Users/me/repo/app')).toBe(
      '/Users/me/repo/README.md',
    )
  })

  it('同行混排不会让相对路径候选吞掉后续绝对路径', () => {
    const mixed = String.raw`改了 docs/a.md 和 C:\repo\c.rs`
    const windowsOnly = extractFilePaths(mixed).map(range => mixed.slice(range.start, range.end))
    const withRelative = extractFilePaths(mixed, true).map(range =>
      mixed.slice(range.start, range.end),
    )
    expect(windowsOnly).toEqual([String.raw`C:\repo\c.rs`])
    expect(withRelative).toEqual(['docs/a.md', String.raw`C:\repo\c.rs`])

    const absoluteMixed = String.raw`/opt/a/b.md + C:\out\c.rs + \\server\share\d.pdf`
    expect(
      extractFilePaths(absoluteMixed).map(range => absoluteMixed.slice(range.start, range.end)),
    ).toEqual(['/opt/a/b.md', String.raw`C:\out\c.rs`, String.raw`\\server\share\d.pdf`])
  })

  it('不识别 URL、代码块、普通句子和未知扩展名', () => {
    const content = [
      'https://example.com/files/report.pdf',
      'github.com/mrpulor-gh/nuphus/blob/main/README.md',
      '这是 release notes.md 的普通句子',
      'local/path.unknown',
      'local/report.json.bak',
      '```text',
      '/tmp/hidden.rs',
      '```',
    ].join('\n')
    const html = renderToString(
      <MarkdownContent content={content} onFileClick={onFileClick} projectBasePath="/repo" />,
    )
    expect(html).not.toContain('data-file-path')
    expect(extractFilePaths('https://example.com/a.pdf', true)).toEqual([])
    expect(extractFilePaths('github.com/org/repo/README.md', true)).toEqual([])
  })
})
