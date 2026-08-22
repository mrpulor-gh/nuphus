import React from 'react'
import '../../styles/markdown.css'

interface MarkdownContentProps {
  content: string
  /** 可选：点击裸文件路径（Windows 绝对路径 + 白名单扩展名）时回调 */
  onFileClick?: (path: string) => void
}

/**
 * Complete lightweight Markdown renderer (zero dependencies)
 * Supports: headings / bold / italic / strikethrough / inline code / code blocks / links / lists / tables / blockquotes / horizontal rules / task lists
 *
 * ⚠️ Key design: separate code blocks (```) first, then process inline markup in text.
 *    All characters inside code blocks are output as-is, not misinterpreted as Markdown syntax.
 */
const MarkdownContent = React.memo(function MarkdownContent({
  content,
  onFileClick,
}: MarkdownContentProps) {
  // Normalize line endings: \r\n / \r → \n, prevent Windows line endings from breaking split(/\n\n+/)
  const normalized = content.replace(/\r\n/g, '\n').replace(/\r/g, '\n')

  // ── 1. Separate code blocks ──
  // Highest priority: extract all ```...``` blocks first, ensure * ` _ ~ etc. inside blocks aren't damaged by inline rendering
  const codeBlockRegex = /```(\w*)\n([\s\S]*?)```/g
  const parts: Array<
    { type: 'code'; lang: string; code: string } | { type: 'text'; text: string }
  > = []

  let lastIndex = 0
  let match: RegExpExecArray | null

  while ((match = codeBlockRegex.exec(normalized)) !== null) {
    if (match.index > lastIndex) {
      parts.push({ type: 'text', text: normalized.slice(lastIndex, match.index) })
    }
    const lang = match[1] || ''
    // Remove leading/trailing extra newlines, preserve internal original indentation
    const code = match[2].replace(/^\n+/, '').replace(/\n+$/, '')
    parts.push({ type: 'code', lang, code })
    lastIndex = match.index + match[0].length
  }
  if (lastIndex < normalized.length) {
    parts.push({ type: 'text', text: normalized.slice(lastIndex) })
  }

  if (parts.length === 0) {
    return <>{content}</>
  }

  return (
    <>
      {parts.map((part, i) =>
        part.type === 'code' ? (
          <pre key={i} className="markdown-code-block">
            {part.lang && <div className="code-lang-label">{part.lang}</div>}
            <code className={part.lang ? `lang-${part.lang}` : ''}>
              {highlightCode(part.code, part.lang)}
            </code>
          </pre>
        ) : (
          <MarkdownText key={i} text={part.text} onFileClick={onFileClick} />
        ),
      )}
    </>
  )
})

// ── 2. Scan line by line to split block-level elements ──
// Handle \r\n line endings + single-line block element separation
function MarkdownText({
  text,
  onFileClick,
}: {
  text: string
  onFileClick?: (path: string) => void
}) {
  const lines = text.split('\n')
  const elements: React.ReactNode[] = []
  let current: string[] = []

  function flush() {
    if (current.length > 0) {
      elements.push(
        <BlockRenderer
          key={elements.length}
          block={current.join('\n')}
          onFileClick={onFileClick}
        />,
      )
      current = []
    }
  }

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const nextLine = lines[i + 1]

    // Empty line forces block split
    if (line.trim() === '') {
      flush()
      continue
    }

    // Check if current line starts a new block-level element
    const isBlockStart =
      line.startsWith('#') || // heading
      /^[-*_]{3,}\s*$/.test(line) || // horizontal rule
      line.startsWith('>') || // blockquote
      /^[-*+]\s/.test(line) || // unordered list
      /^\d+\.\s/.test(line) || // ordered list
      /^[-*+]\s\[[ x]\]\s/i.test(line) || // task list
      (line.includes('|') && nextLine && nextLine.includes('|') && /^[\s:|:-]+$/.test(nextLine)) // table

    if (isBlockStart && current.length > 0) {
      flush()
    }

    current.push(line)

    // Table: consume all consecutive lines containing |
    if (line.includes('|') && nextLine && nextLine.includes('|') && /^[\s:|:-]+$/.test(nextLine)) {
      i++ // Skip header row, enter data row consumption
      while (i < lines.length && lines[i].includes('|')) {
        current.push(lines[i])
        i++
      }
      i-- // while went one step too far, step back
      flush()
      continue
    }
  }

  flush()

  return <>{elements}</>
}

// ── Recursive nested list rendering ──
function NestedList({
  lines,
  ordered,
  onFileClick,
}: {
  lines: string[]
  ordered: boolean
  onFileClick?: (path: string) => void
}) {
  const Tag = ordered ? 'ol' : 'ul'
  const cls = ordered ? 'markdown-ol' : 'markdown-ul'

  // Calculate base indent
  const nonEmpty = lines.filter(l => l.trim().length > 0)
  if (nonEmpty.length === 0) return null
  const baseIndent = Math.min(...nonEmpty.map(l => l.search(/\S/)))

  // Group by sibling items, collect children
  const items: { line: string; children: string[] }[] = []
  let i = 0
  while (i < lines.length) {
    const line = lines[i]
    if (line.trim().length === 0) {
      i++
      continue
    }
    const indent = line.search(/\S/)
    if (indent < baseIndent) {
      i++
      continue
    }
    if (indent > baseIndent) {
      i++
      continue
    }

    const childLines: string[] = []
    i++
    while (i < lines.length) {
      if (lines[i].trim().length === 0) {
        i++
        continue
      }
      const childIndent = lines[i].search(/\S/)
      if (childIndent <= baseIndent) break
      childLines.push(lines[i])
      i++
    }
    items.push({ line: line.trim(), children: childLines })
  }

  return (
    <Tag className={cls}>
      {items.map((item, idx) => {
        const line = item.line
        // Task list detection
        if (/^[-*+]\s\[[ x]\]\s/i.test(line)) {
          const checked = /^-\s\[x\]/i.test(line)
          const text = line.replace(/^[-*+]\s\[[ x]\]\s+/i, '')
          let childList: React.ReactNode = null
          if (item.children.length > 0) {
            const firstChild = item.children.find(l => l.trim().length > 0)?.trim() || ''
            childList = (
              <NestedList
                lines={item.children}
                ordered={/^\d+\.\s/.test(firstChild)}
                onFileClick={onFileClick}
              />
            )
          }
          return (
            <li key={idx} className="markdown-li markdown-task-item">
              <span className={`markdown-task-checkbox ${checked ? 'checked' : ''}`}>
                {checked ? '✓' : '○'}
              </span>
              <span className="markdown-body">
                <span className={checked ? 'markdown-task-done' : ''}>
                  <MarkdownInline text={text} onFileClick={onFileClick} />
                </span>
                {childList}
              </span>
            </li>
          )
        }

        const content = ordered ? line.replace(/^\d+\.\s+/, '') : line.replace(/^[-*+]\s+/, '')
        const marker = ordered ? (line.match(/^\d+/)?.[0] || '') + '.' : '•'
        const markerClass = ordered ? 'markdown-marker markdown-marker-ol' : 'markdown-marker'

        let childList: React.ReactNode = null
        if (item.children.length > 0) {
          const firstChild = item.children.find(l => l.trim().length > 0)?.trim() || ''
          const childOrdered = /^\d+\.\s/.test(firstChild)
          childList = (
            <NestedList lines={item.children} ordered={childOrdered} onFileClick={onFileClick} />
          )
        }

        return (
          <li key={idx} className="markdown-li">
            <span className={markerClass}>{marker}</span>
            <span className="markdown-body">
              <MarkdownInline text={content} onFileClick={onFileClick} />
              {childList}
            </span>
          </li>
        )
      })}
    </Tag>
  )
}

// ── 3. Determine block type and render ──
function BlockRenderer({
  block,
  onFileClick,
}: {
  block: string
  onFileClick?: (path: string) => void
}) {
  const lines = block.split('\n')

  // ▸ Horizontal rule (single line, at least 3 - * _)
  if (/^[-*_]{3,}\s*$/.test(lines[0]) && lines.length === 1) {
    return <hr className="markdown-hr" />
  }

  // ▸ Blockquote (starts with >)
  if (lines[0].startsWith('>')) {
    const quoteText = lines.map(l => l.replace(/^>\s?/, '')).join('\n')
    // Empty quote (> / > alone) → render nothing, otherwise a styled box with no content appears
    if (quoteText.trim().length === 0) return null
    return (
      <blockquote className="markdown-blockquote">
        <MarkdownInline text={quoteText} onFileClick={onFileClick} />
      </blockquote>
    )
  }

  // ▸ Unordered list
  if (/^[-*+]\s/.test(lines[0])) {
    return <NestedList lines={lines} ordered={false} onFileClick={onFileClick} />
  }

  // ▸ Ordered list
  if (/^\d+\.\s/.test(lines[0])) {
    return <NestedList lines={lines} ordered={true} onFileClick={onFileClick} />
  }

  // ▸ Table (second line must be separator: |---| format)
  if (lines.length >= 2 && lines[0].includes('|') && /^[\s:|:-]+$/.test(lines[1])) {
    return <TableRenderer lines={lines} onFileClick={onFileClick} />
  }

  // ▸ Headings (# ~ ######)
  const headerMatch = lines[0].match(/^(#{1,6})\s+(.+)$/)
  if (headerMatch) {
    const level = headerMatch[1].length
    const Tag = `h${level}` as keyof JSX.IntrinsicElements
    return (
      <Tag className={`markdown-h markdown-h${level}`}>
        <MarkdownInline text={headerMatch[2]} onFileClick={onFileClick} />
      </Tag>
    )
  }

  // ▸ Default: paragraph
  return (
    <p className="markdown-paragraph">
      <MarkdownInline text={block} onFileClick={onFileClick} />
    </p>
  )
}

// ── 4. Table rendering ──
function TableRenderer({
  lines,
  onFileClick,
}: {
  lines: string[]
  onFileClick?: (path: string) => void
}) {
  const headers = lines[0]
    .split('|')
    .filter(Boolean)
    .map(s => s.trim())
  const alignParts = lines[1].split('|').filter(Boolean)
  const aligns = alignParts.map(s => {
    const t = s.trim()
    if (t.startsWith(':') && t.endsWith(':')) return 'center'
    if (t.endsWith(':')) return 'right'
    return 'left'
  })
  const body = lines.slice(2).filter(l => l.includes('|'))

  return (
    <div className="markdown-table-wrapper">
      <table className="markdown-table">
        <thead>
          <tr>
            {headers.map((h, i) => (
              <th key={i} style={{ textAlign: aligns[i] as any }}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {body.map((row, ri) => (
            <tr key={ri}>
              {row
                .split('|')
                .filter(Boolean)
                .map((cell, ci) => (
                  <td key={ci} style={{ textAlign: aligns[ci] as any }}>
                    <MarkdownInline text={cell.trim()} onFileClick={onFileClick} />
                  </td>
                ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

// ── 5. Inline rendering ──
// Match order (by priority):
//   1. Inline code `code`            — highest priority, prevent * inside `` from being mis-parsed
//   2. Bold **text**
//   3. Italic *text*                — fixed: removed faulty lookbehind that caused match failures
//   4. Strikethrough ~~text~~
//   5. Link [text](url)
//   6. Bare Windows absolute file path (only when onFileClick provided)

/** 白名单扩展名：扩展名后不得紧跟字母/数字/下划线（避免 .md5 之类误判） */
const FILE_EXT_WHITELIST =
  /\.(?:md|html?|rs|tsx?|jsx?|py|json|toml|css|ya?ml|sh|pdf|png|jpe?g|svg)(?![A-Za-z0-9_])/i
/** 盘符开头绝对路径候选：`:` 在排除集中，天然在第二个冒号处截断（分隔相邻路径） */
const PATH_CANDIDATE_RE = /[A-Za-z]:[\\/][^\r\n<>:"|?*]*/g

/** 在文本中提取所有「盘符开头 + 白名单扩展名」的路径区间 */
function extractFilePaths(text: string): Array<{ start: number; end: number }> {
  const out: Array<{ start: number; end: number }> = []
  const re = new RegExp(PATH_CANDIDATE_RE.source, 'g')
  let m: RegExpExecArray | null
  while ((m = re.exec(text)) !== null) {
    // 前导字符紧贴字母/数字/下划线/斜杠时不视为路径开头（防误伤变量名等）
    if (m.index > 0 && /[A-Za-z0-9_\\/]/.test(text[m.index - 1])) continue
    const candidate = m[0]
    const ext = FILE_EXT_WHITELIST.exec(candidate)
    if (ext && ext.index !== undefined) {
      const end = m.index + ext.index + ext[0].length
      out.push({ start: m.index, end })
      // 候选内扩展名后可能还跟着下一个路径（如 "C:\a.md C:\b.md"），从截断处继续扫描
      re.lastIndex = end
    }
  }
  return out
}

/**
 * 对字符串节点做路径识别渲染；onFileClick 缺省时原样返回（零回归）。
 * 只处理字符串节点，已渲染的 <a>/<strong> 等节点跳过。
 */
function applyFilePaths(
  nodes: React.ReactNode[],
  onFileClick: ((path: string) => void) | undefined,
  prefix: string,
): React.ReactNode[] {
  if (!onFileClick) return nodes
  return nodes.flatMap((n, idx) => {
    if (typeof n !== 'string') return [n]
    const ranges = extractFilePaths(n)
    if (ranges.length === 0) return [n]
    const parts: React.ReactNode[] = []
    let cursor = 0
    ranges.forEach((r, ri) => {
      if (r.start > cursor) parts.push(n.slice(cursor, r.start))
      const path = n.slice(r.start, r.end)
      parts.push(
        <a
          key={`p-${prefix}-${idx}-${ri}`}
          href="#"
          className="markdown-file-path"
          title={path}
          onClick={e => {
            e.preventDefault()
            e.stopPropagation()
            onFileClick(path)
          }}
        >
          {path}
        </a>,
      )
      cursor = r.end
    })
    if (cursor < n.length) parts.push(n.slice(cursor))
    return parts
  })
}

function MarkdownInline({
  text,
  onFileClick,
}: {
  text: string
  onFileClick?: (path: string) => void
}) {
  const boldRegex = /\*\*(.+?)\*\*/g
  const italicRegex = /(?<!\w)\*(?!\*)(.+?)\*(?!\*)/g
  const delRegex = /~~(.+?)~~/g
  const linkRegex = /\[([^\]]+)\]\(([^)]+)\)/g

  type Seg = { t: 'code'; v: string } | { t: 'text'; v: string }

  function tokenizeByCode(s: string): Seg[] {
    const result: Seg[] = []
    let last = 0
    let m: RegExpExecArray | null
    const re = /`([^`]+)`/g
    while ((m = re.exec(s)) !== null) {
      if (m.index > last) result.push({ t: 'text', v: s.slice(last, m.index) })
      result.push({ t: 'code', v: m[1] })
      last = m.index + m[0].length
    }
    if (last < s.length) result.push({ t: 'text', v: s.slice(last) })
    return result
  }

  function renderText(s: string): React.ReactNode[] {
    const nodes: React.ReactNode[] = []
    const segments = tokenizeByCode(s)
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i]
      if (seg.t === 'code') {
        nodes.push(
          <code key={`c-${i}-${seg.v}`} className="inline-code">
            {seg.v}
          </code>,
        )
        continue
      }
      let remainder = seg.v

      function applyOne(
        str: string,
        regex: RegExp,
        wrap: (m: RegExpExecArray) => React.ReactNode,
      ): React.ReactNode[] {
        const out: React.ReactNode[] = []
        let idx = 0
        let m: RegExpExecArray | null
        const re = new RegExp(regex.source, regex.flags.includes('u') ? 'gu' : 'g')
        while ((m = re.exec(str)) !== null) {
          if (m.index > idx) out.push(str.slice(idx, m.index))
          out.push(wrap(m))
          idx = m.index + m[0].length
        }
        if (idx < str.length) out.push(str.slice(idx))
        return out
      }

      let layer: React.ReactNode[] = [remainder]
      layer = layer.flatMap(n =>
        typeof n === 'string'
          ? applyOne(n, boldRegex, m => <strong key={`b-${i}-${m.index}`}>{m[1]}</strong>)
          : [n],
      )
      layer = layer.flatMap(n =>
        typeof n === 'string'
          ? applyOne(n, italicRegex, m => <em key={`i-${i}-${m.index}`}>{m[1]}</em>)
          : [n],
      )
      layer = layer.flatMap(n =>
        typeof n === 'string'
          ? applyOne(n, delRegex, m => <del key={`d-${i}-${m.index}`}>{m[1]}</del>)
          : [n],
      )
      layer = layer.flatMap(n =>
        typeof n === 'string'
          ? applyOne(n, linkRegex, m => {
              // 协议白名单（与移动端 MobileMarkdown 语义对齐）：javascript:/data: 等
              // 伪协议一律降级为纯文本——LLM 输出可含恶意链接，点击即在页面上下文
              // 执行任意 JS，可读 localStorage 中的凭证 → agent 控制权失守
              const url = m[2].trim()
              if (!/^(https?:|mailto:)/i.test(url)) {
                return <span key={`t-${i}-${m.index}`}>{m[1]}</span>
              }
              return (
                <a
                  key={`a-${i}-${m.index}`}
                  href={url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="markdown-link"
                >
                  {m[1]}
                </a>
              )
            })
          : [n],
      )

      // 裸文件路径识别（link 之后；onFileClick 缺省时零回归）
      layer = applyFilePaths(layer, onFileClick, `f-${i}`)

      nodes.push(...layer)
    }
    return nodes
  }

  const result = renderText(text)
  if (result.length === 0) return <>{text}</>
  return <>{result}</>
}

// ═══════════════════════════════════════════════════════════════════════════
// Code syntax highlighting — zero dependencies, regex-based lightweight implementation
// ═══════════════════════════════════════════════════════════════════════════

interface TokenRule {
  regex: RegExp
  className: string
}

const TOKEN_RULES: Record<string, TokenRule[]> = {
  rust: [
    { regex: /\/\/.*$/gm, className: 'token-comment' },
    { regex: /\/\*[\s\S]*?\*\//g, className: 'token-comment' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    {
      regex:
        /\b(?:fn|let|mut|const|static|struct|enum|impl|trait|use|mod|pub|crate|self|Self|super|where|for|if|else|match|while|loop|break|continue|return|async|await|unsafe|move|ref|type|dyn|as|in|box|yield|macro)\b/g,
      className: 'token-keyword',
    },
    {
      regex:
        /\b(?:i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|char|str|String|Vec|Option|Result|Box|Rc|Arc|Cell|RefCell|Mutex|HashMap|BTreeMap|HashSet|BTreeSet|VecDeque|LinkedList|BinaryHeap)\b/g,
      className: 'token-type',
    },
    { regex: /\b(?:true|false|None|Some|Ok|Err)\b/g, className: 'token-builtin' },
    { regex: /\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g, className: 'token-number' },
    { regex: /\b[A-Z][a-zA-Z0-9_]*\b/g, className: 'token-class' },
  ],
  typescript: [
    { regex: /\/\/.*$/gm, className: 'token-comment' },
    { regex: /\/\*[\s\S]*?\*\//g, className: 'token-comment' },
    { regex: /`(?:[^`\\]|\\.|\\\$\{[^}]*\})*`/g, className: 'token-string' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    {
      regex:
        /\b(?:const|let|var|function|class|interface|type|enum|namespace|module|import|export|from|default|extends|implements|new|this|super|static|readonly|private|protected|public|abstract|async|await|return|if|else|switch|case|break|continue|for|while|do|try|catch|finally|throw|typeof|instanceof|in|of|void|delete|yield|debugger|with)\b/g,
      className: 'token-keyword',
    },
    {
      regex:
        /\b(?:string|number|boolean|symbol|bigint|undefined|null|any|unknown|never|object|Array|Map|Set|Promise|Date|RegExp|Error|Function|String|Number|Boolean|Object)\b/g,
      className: 'token-type',
    },
    { regex: /\b(?:true|false|null|undefined|NaN|Infinity)\b/g, className: 'token-builtin' },
    { regex: /\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g, className: 'token-number' },
    { regex: /\b[A-Z][a-zA-Z0-9_]*\b/g, className: 'token-class' },
  ],
  javascript: [
    { regex: /\/\/.*$/gm, className: 'token-comment' },
    { regex: /\/\*[\s\S]*?\*\//g, className: 'token-comment' },
    { regex: /`(?:[^`\\]|\\.|\\\$\{[^}]*\})*`/g, className: 'token-string' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    {
      regex:
        /\b(?:const|let|var|function|class|extends|import|export|from|default|new|this|super|static|async|await|return|if|else|switch|case|break|continue|for|while|do|try|catch|finally|throw|typeof|instanceof|in|of|void|delete|yield|debugger|with)\b/g,
      className: 'token-keyword',
    },
    { regex: /\b(?:true|false|null|undefined|NaN|Infinity)\b/g, className: 'token-builtin' },
    { regex: /\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g, className: 'token-number' },
    { regex: /\b[A-Z][a-zA-Z0-9_]*\b/g, className: 'token-class' },
  ],
  python: [
    { regex: /#.*$/gm, className: 'token-comment' },
    { regex: /"""[\s\S]*?"""/g, className: 'token-string' },
    { regex: /'''[\s\S]*?'''/g, className: 'token-string' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    {
      regex:
        /\b(?:def|class|if|elif|else|for|while|break|continue|return|try|except|finally|raise|with|as|import|from|pass|lambda|yield|assert|del|global|nonlocal|async|await|match|case)\b/g,
      className: 'token-keyword',
    },
    { regex: /\b(?:True|False|None|NotImplemented|Ellipsis)\b/g, className: 'token-builtin' },
    {
      regex:
        /\b(?:int|float|str|list|dict|tuple|set|frozenset|bool|bytes|bytearray|memoryview|object|type|len|range|enumerate|zip|map|filter|sum|min|max|sorted|reversed|any|all|abs|round|pow|divmod|chr|ord|hex|oct|bin|isinstance|issubclass|hasattr|getattr|setattr|delattr|callable|iter|next|slice|repr|format|vars|dir|help|open|input|print)\b/g,
      className: 'token-builtin',
    },
    { regex: /\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g, className: 'token-number' },
    { regex: /\b[A-Z][a-zA-Z0-9_]*\b/g, className: 'token-class' },
  ],
  json: [
    { regex: /"(?:[^"\\]|\\.)*"(?=\s*:)/g, className: 'token-property' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /\b(?:true|false|null)\b/g, className: 'token-builtin' },
    { regex: /\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g, className: 'token-number' },
  ],
  bash: [
    { regex: /#.*$/gm, className: 'token-comment' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    {
      regex:
        /\b(?:if|then|else|elif|fi|for|while|do|done|case|esac|in|function|return|break|continue|shift|exit|export|source|alias|unset|readonly|local|declare|typeset|trap|wait|bg|fg|jobs|kill|test|echo|printf|read|cd|pwd|ls|cat|grep|sed|awk|chmod|chown|mkdir|rmdir|rm|cp|mv|ln|find|sort|uniq|wc|head|tail|cut|paste|join|diff|patch|tar|gzip|gunzip|zip|unzip|ssh|scp|curl|wget|git|docker|cargo|npm|node|python|python3|rustc|make|cmake)\b/g,
      className: 'token-keyword',
    },
    { regex: /\b\d+\b/g, className: 'token-number' },
    { regex: /\$\w+|\$\{[^}]*\}/g, className: 'token-variable' },
  ],
  css: [
    { regex: /\/\*[\s\S]*?\*\//g, className: 'token-comment' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    { regex: /@[a-z-]+/g, className: 'token-atrule' },
    {
      regex:
        /\b(?:px|em|rem|vh|vw|vmin|vmax|%|s|ms|deg|rad|turn|Hz|kHz|dpi|dpcm|dppx|fr|ex|ch|cm|mm|in|pt|pc)\b/g,
      className: 'token-unit',
    },
    { regex: /#[a-fA-F0-9]{3,8}\b/g, className: 'token-color' },
    { regex: /\b\d+(?:\.\d+)?\b/g, className: 'token-number' },
    { regex: /\b[a-z-]+(?=\s*:)/g, className: 'token-property' },
  ],
  toml: [
    { regex: /#.*$/gm, className: 'token-comment' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    { regex: /\b(?:true|false)\b/g, className: 'token-builtin' },
    { regex: /\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g, className: 'token-number' },
    { regex: /^\s*\[.+\]\s*$/gm, className: 'token-section' },
  ],
  html: [
    { regex: /<!--[\s\S]*?-->/g, className: 'token-comment' },
    { regex: /"(?:[^"\\]|\\.)*"/g, className: 'token-string' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    { regex: /&[a-zA-Z0-9#]+;/g, className: 'token-builtin' },
    { regex: /<\/?[a-zA-Z][a-zA-Z0-9]*/g, className: 'token-keyword' },
    {
      regex:
        /\b(?:class|id|style|href|src|alt|title|type|name|value|placeholder|disabled|checked|selected|required|readonly|data-[a-zA-Z0-9-]+|aria-[a-zA-Z0-9-]+)\b(?=\s*[= >])/g,
      className: 'token-property',
    },
    { regex: /(?<=\s)[a-zA-Z][a-zA-Z0-9]*(?=\s*=\s*["'])/g, className: 'token-attribute' },
  ],
  sql: [
    { regex: /--.*$/gm, className: 'token-comment' },
    { regex: /\/\*[\s\S]*?\*\//g, className: 'token-comment' },
    { regex: /'(?:[^'\\]|\\.)*'/g, className: 'token-string' },
    {
      regex:
        /\b(?:SELECT|FROM|WHERE|INSERT|INTO|VALUES|UPDATE|SET|DELETE|CREATE|TABLE|ALTER|DROP|INDEX|VIEW|JOIN|LEFT|RIGHT|INNER|OUTER|FULL|CROSS|ON|AND|OR|NOT|IN|LIKE|BETWEEN|IS|NULL|AS|DISTINCT|ORDER|BY|GROUP|HAVING|LIMIT|OFFSET|UNION|ALL|EXISTS|CASE|WHEN|THEN|ELSE|END|ASC|DESC|COUNT|SUM|AVG|MIN|MAX|CAST|COALESCE|NULLIF|WITH|RECURSIVE|PRIMARY|KEY|FOREIGN|REFERENCES|CASCADE|UNIQUE|CHECK|DEFAULT|IF|BEGIN|COMMIT|ROLLBACK|SAVEPOINT|TRIGGER|FUNCTION|PROCEDURE|EXEC|RETURNS|LANGUAGE|IMMUTABLE|STABLE|STRICT|RETURNING|ON|CONFLICT|DO|NOTHING)\b/g,
      className: 'token-keyword',
    },
    { regex: /\b\d+(?:\.\d+)?\b/g, className: 'token-number' },
    { regex: /\b(?:TRUE|FALSE|NULL|UNKNOWN)\b/g, className: 'token-builtin' },
  ],
  diff: [
    { regex: /^\+[^+].*$/gm, className: 'token-diff-insert' },
    { regex: /^\-[^-].*$/gm, className: 'token-diff-delete' },
    { regex: /^@@ .+ @@.*$/gm, className: 'token-diff-header' },
    { regex: /^diff --git.*$/gm, className: 'token-diff-meta' },
    { regex: /^index .+$/gm, className: 'token-diff-meta' },
    { regex: /^--- .+$/gm, className: 'token-diff-file' },
    { regex: /^\+\+\+ .+$/gm, className: 'token-diff-file' },
  ],
}

// Alias mapping
const LANG_ALIASES: Record<string, string> = {
  ts: 'typescript',
  js: 'javascript',
  py: 'python',
  sh: 'bash',
  shell: 'bash',
  zsh: 'bash',
  yml: 'yaml',
  yaml: 'toml',
  rs: 'rust',
  jsx: 'javascript',
  tsx: 'typescript',
  htm: 'html',
  xhtml: 'html',
  vue: 'html',
  svelte: 'html',
}

function highlightCode(code: string, lang: string): React.ReactNode[] {
  const normalizedLang = LANG_ALIASES[lang] || lang
  const rules = TOKEN_RULES[normalizedLang]

  if (!rules || rules.length === 0) {
    return [<span key="plain">{code}</span>]
  }

  // Collect all match positions
  type Match = { start: number; end: number; className: string; text: string }
  const matches: Match[] = []

  for (const rule of rules) {
    const re = new RegExp(
      rule.regex.source,
      rule.regex.flags.includes('g') ? rule.regex.flags : rule.regex.flags + 'g',
    )
    let m: RegExpExecArray | null
    while ((m = re.exec(code)) !== null) {
      matches.push({
        start: m.index,
        end: m.index + m[0].length,
        className: rule.className,
        text: m[0],
      })
    }
  }

  // Sort by position, dedup (later rules override earlier ones)
  matches.sort((a, b) => a.start - b.start || b.end - a.end)

  const result: React.ReactNode[] = []
  let lastEnd = 0
  let i = 0

  while (i < matches.length) {
    const match = matches[i]

    // Skip overlapping matches
    if (match.start < lastEnd) {
      i++
      continue
    }

    // Add unmatched plain text
    if (match.start > lastEnd) {
      result.push(<span key={`t-${lastEnd}`}>{code.slice(lastEnd, match.start)}</span>)
    }

    // Add highlighted token
    result.push(
      <span key={`${match.className}-${match.start}`} className={match.className}>
        {match.text}
      </span>,
    )
    lastEnd = match.end
    i++
  }

  // Add remaining text
  if (lastEnd < code.length) {
    result.push(<span key={`t-end`}>{code.slice(lastEnd)}</span>)
  }

  return result
}

export default MarkdownContent
