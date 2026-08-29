import React, { useState } from 'react'

interface MarkdownContentProps {
  content: string
}

/**
 * Complete lightweight Markdown renderer (zero dependencies)
 * Supports: headings / bold / italic / strikethrough / inline code / code blocks / links / lists / tables / blockquotes / horizontal rules / task lists
 *
 * ⚠️ Key design: separate code blocks (```) first, then process inline markup in text.
 *    All characters inside code blocks are output as-is, not misinterpreted as Markdown syntax.
 */
const MarkdownContent = React.memo(function MarkdownContent({ content }: MarkdownContentProps) {
  // 代码块复制反馈（记录已复制的代码块索引，短暂显示「已复制」）
  const [copiedIdx, setCopiedIdx] = useState<number | null>(null)

  // 跨环境复制：优先 Clipboard API（secure context），HTTP 环境 fallback textarea
  const copyCode = async (code: string, idx: number) => {
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(code)
      } else {
        const ta = document.createElement('textarea')
        ta.value = code
        ta.style.position = 'fixed'
        ta.style.opacity = '0'
        document.body.appendChild(ta)
        ta.select()
        document.execCommand('copy')
        document.body.removeChild(ta)
      }
      setCopiedIdx(idx)
      setTimeout(() => setCopiedIdx(prev => (prev === idx ? null : prev)), 1500)
    } catch {
      /* 复制失败静默：按钮保持可点，不阻塞阅读 */
    }
  }

  // Normalize line endings: \r\n / \r → \n, prevent Windows line endings from breaking split(/\n\n+/)
  const normalized = content.replace(/\r\n/g, '\n').replace(/\r/g, '\n')

  // ── 1. Separate code blocks ──
  // Highest priority: extract all ```...``` blocks first, ensure * ` _ ~ etc. inside blocks aren't damaged by inline rendering
  // 闭合可选：用户缺闭合 ``` 时（手动输入/粘贴截断），从 ```lang\n 到文本结尾仍作为代码块提取，
  // 防止内部 **bold** / # heading / | table | 被当普通 markdown 误解析破坏代码展示。
  const codeBlockRegex = /```(\w*)\n([\s\S]*?)(```|$)/g
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
          <pre key={i} className="m-md-code-block">
            <div className="m-md-code-head">
              {part.lang && <span className="code-lang-label">{part.lang}</span>}
              <button
                type="button"
                className="m-md-code-copy"
                onClick={() => void copyCode(part.code, i)}
                aria-label="复制代码"
              >
                {copiedIdx === i ? '已复制' : '复制'}
              </button>
            </div>
            <code className={part.lang ? `lang-${part.lang}` : ''}>
              {highlightCode(part.code, part.lang)}
            </code>
          </pre>
        ) : (
          <MarkdownText key={i} text={part.text} />
        ),
      )}
    </>
  )
})

// ── 2. Scan line by line to split block-level elements ──
// Handle \r\n line endings + single-line block element separation
function MarkdownText({ text }: { text: string }) {
  const lines = text.split('\n')
  const elements: React.ReactNode[] = []
  let current: string[] = []

  function flush() {
    if (current.length > 0) {
      elements.push(<BlockRenderer key={elements.length} block={current.join('\n')} />)
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

    // List (ordered/unordered/task): merge consecutive list lines (incl. indented
    // children) into ONE <ol>/<ul> block. Without this every item renders as its own
    // <ol> — in WebViews where the list-style:none CSS rule is not honored (WeChat X5
    // old kernel, stale bundle) each <ol> numbers from 1, stacking with the manual
    // marker → "1. 1. / 1. 2. / 1. 3." double-numbering (real user-reported bug).
    if (/^\d+\.\s/.test(line) || /^[-*+]\s/.test(line)) {
      i++
      while (i < lines.length) {
        const nl = lines[i]
        if (nl.trim() === '') break
        // 只吞「明确的列表行」或「缩进的列表子项」（- / * / 1. 前缀），
        // 不吞普通缩进段落/续行——避免把列表后的缩进内容错误并入列表块。
        if (/^\d+\.\s/.test(nl) || /^[-*+]\s/.test(nl) || /^\s+([-*+]|\d+\.)\s/.test(nl)) {
          current.push(nl)
          i++
          continue
        }
        break
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
function NestedList({ lines, ordered }: { lines: string[]; ordered: boolean }) {
  const Tag = ordered ? 'ol' : 'ul'
  const cls = ordered ? 'm-md-ol' : 'm-md-ul'

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
    <Tag className={cls} style={{ listStyle: 'none' }}>
      {items.map((item, idx) => {
        const line = item.line
        // Task list detection
        if (/^[-*+]\s\[[ x]\]\s/i.test(line)) {
          const checked = /^-\s\[x\]/i.test(line)
          const text = line.replace(/^[-*+]\s\[[ x]\]\s+/i, '')
          let childList: React.ReactNode = null
          if (item.children.length > 0) {
            const firstChild = item.children.find(l => l.trim().length > 0)?.trim() || ''
            childList = <NestedList lines={item.children} ordered={/^\d+\.\s/.test(firstChild)} />
          }
          return (
            <li key={idx} className="m-md-li m-md-task-item">
              <span className={`m-md-task-checkbox ${checked ? 'checked' : ''}`}>
                {checked ? '✓' : '○'}
              </span>
              <span className="m-md-body">
                <span className={checked ? 'm-md-task-done' : ''}>
                  <MarkdownInline text={text} />
                </span>
                {childList}
              </span>
            </li>
          )
        }

        const content = ordered ? line.replace(/^\d+\.\s+/, '') : line.replace(/^[-*+]\s+/, '')
        const marker = ordered ? (line.match(/^\d+/)?.[0] || '') + '.' : '•'
        const markerClass = ordered ? 'm-md-marker m-md-marker-ol' : 'm-md-marker'

        let childList: React.ReactNode = null
        if (item.children.length > 0) {
          const firstChild = item.children.find(l => l.trim().length > 0)?.trim() || ''
          const childOrdered = /^\d+\.\s/.test(firstChild)
          childList = <NestedList lines={item.children} ordered={childOrdered} />
        }

        return (
          <li key={idx} className="m-md-li">
            <span className={markerClass}>{marker}</span>
            <span className="m-md-body">
              <MarkdownInline text={content} />
              {childList}
            </span>
          </li>
        )
      })}
    </Tag>
  )
}

// ── 3. Determine block type and render ──
function BlockRenderer({ block }: { block: string }) {
  const lines = block.split('\n')

  // ▸ Horizontal rule (single line, at least 3 - * _)
  if (/^[-*_]{3,}\s*$/.test(lines[0]) && lines.length === 1) {
    return <hr className="m-md-hr" />
  }

  // ▸ Blockquote (starts with >)
  if (lines[0].startsWith('>')) {
    const quoteText = lines.map(l => l.replace(/^>\s?/, '')).join('\n')
    // Empty quote (> / > alone) → render nothing, otherwise a styled box with no content appears
    if (quoteText.trim().length === 0) return null
    return (
      <blockquote className="m-md-blockquote">
        <MarkdownInline text={quoteText} />
      </blockquote>
    )
  }

  // ▸ Unordered list
  if (/^[-*+]\s/.test(lines[0])) {
    return <NestedList lines={lines} ordered={false} />
  }

  // ▸ Ordered list
  if (/^\d+\.\s/.test(lines[0])) {
    return <NestedList lines={lines} ordered={true} />
  }

  // ▸ Table (second line must be separator: |---| format)
  if (lines.length >= 2 && lines[0].includes('|') && /^[\s:|:-]+$/.test(lines[1])) {
    return <TableRenderer lines={lines} />
  }

  // ▸ Headings (# ~ ######)
  const headerMatch = lines[0].match(/^(#{1,6})\s+(.+)$/)
  if (headerMatch) {
    const level = headerMatch[1].length
    const Tag = `h${level}` as keyof JSX.IntrinsicElements
    // 与桌面端 MarkdownContent 同款修复：紧凑 Markdown（标题与正文间无空行）
    // 会把正文并进标题块，只渲染 lines[0] 会静默丢弃正文
    const rest = lines.slice(1)
    const heading = (
      <Tag className={`m-md-h m-md-h${level}`}>
        <MarkdownInline text={headerMatch[2]} />
      </Tag>
    )
    if (rest.length === 0 || rest.every(l => l.trim() === '')) {
      return heading
    }
    return (
      <>
        {heading}
        <MarkdownText text={rest.join('\n')} />
      </>
    )
  }

  // ▸ Default: paragraph
  return (
    <p className="m-md-paragraph">
      <MarkdownInline text={block} />
    </p>
  )
}

// ── 4. Table rendering ──

/** 估算文本渲染宽度（相对单位）：CJK/全角 = 1，ASCII = 0.55（13px 字体近似） */
function estimateTextWidth(s: string): number {
  let w = 0
  for (const ch of s) {
    w += (ch.codePointAt(0) ?? 0) > 0x2e7f ? 1 : 0.55
  }
  return w
}

/** 去掉 markdown 标记符号（** / ` / ~~ / 链接），避免影响宽度估算 */
function stripMdForWidth(s: string): string {
  return s.replace(/\*\*|~~|`/g, '').replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')
}

function TableRenderer({ lines }: { lines: string[] }) {
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

  // 动态列宽：按每列内容实际宽度估算权重，比例分配（带 min/max 保护）。
  // 短列（# 序号）自然窄，长内容列（说明/状态）自然宽，不靠固定百分比硬切。
  const colTexts = headers.map((h, ci) => {
    const texts = [h]
    for (const row of body) {
      const cells = row.split('|').filter(Boolean)
      if (cells[ci] !== undefined) texts.push(cells[ci].trim())
    }
    return texts
  })
  const colWeights = colTexts.map(texts =>
    Math.max(...texts.map(t => estimateTextWidth(stripMdForWidth(t))), 1),
  )
  const weightTotal = colWeights.reduce((a, b) => a + b, 0)
  // 列宽分配：短列（序号/中文标签）保底 MIN，长列（代码/说明）封顶 MAX。
  // ⚠️ 调参依据（375px 屏、13px 字号）：6 个中文字 ≈ 78px + 16px padding ≈ 25% 屏宽，
  // 因此 MIN=0.18 保证「后端消息链路」类中文标签不逐字竖排（实测旧值 0.08 时
  // 归一化后首列仅 ~19% ≈ 51px，只能放 3.9 字 → 6 字标签断成 2-3 行）。
  // MAX=0.45 防止超长内容列垄断 80%+ 宽度（旧值 0.5 归一化后仍达 ~81%，短列被饿死）。
  const MIN_PCT = 0.18 // 序号/短标签列保底
  const MAX_PCT = 0.45 // 单列不垄断（长内容列让出空间给短列）
  const clamped = colWeights.map(w => Math.min(MAX_PCT, Math.max(MIN_PCT, w / weightTotal)))
  const clampedTotal = clamped.reduce((a, b) => a + b, 0)
  const colWidths = clamped.map(p => `${Math.round((p / clampedTotal) * 100)}%`)
  // table-layout:fixed 只参考表头（首行）宽度，th 内联 width 即列宽定义
  const thStyle = (i: number) => ({ textAlign: aligns[i] as any, width: colWidths[i] })
  const tdStyle = (i: number) => ({ textAlign: aligns[i] as any })

  return (
    <div className="m-md-table-wrapper">
      <table className="m-md-table">
        <thead>
          <tr>
            {headers.map((h, i) => (
              <th key={i} style={thStyle(i)}>
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
                  <td key={ci} style={tdStyle(ci)}>
                    <MarkdownInline text={cell.trim()} />
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
function MarkdownInline({ text }: { text: string }) {
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
              // 协议白名单：javascript:/data: 等伪协议一律降级为纯文本——
              // LLM 输出可含恶意链接，点击即在页面上下文执行任意 JS，
              // 可读 localStorage 中的 mobile token → 桌面 agent 控制权失守
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
                  className="m-md-link"
                >
                  {m[1]}
                </a>
              )
            })
          : [n],
      )

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
