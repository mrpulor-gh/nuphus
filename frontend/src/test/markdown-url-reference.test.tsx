import { describe, expect, it, vi } from 'vitest'
import { renderToString } from 'react-dom/server'
import MarkdownContent, { extractUrls } from '../main-window/chat/MarkdownContent'

describe('MarkdownContent 裸 URL 引用', () => {
  const onUrlClick = vi.fn()

  it('裸 http(s) URL 渲染为可点击链接块（chip 显示域名）', () => {
    const html = renderToString(
      <MarkdownContent content="参考 https://example.com/docs/guide 开始" onUrlClick={onUrlClick} />,
    )
    expect(html).toContain('class="markdown-url-path"')
    expect(html).toContain('data-url-href="https://example.com/docs/guide"')
    expect(html).toContain('>example.com<')
    // 芯片只显示域名，但完整 URL 必须保留在 title 里供用户核对
    expect(html).toContain('title="https://example.com/docs/guide"')
  })

  it('http 与 https 都识别', () => {
    const html = renderToString(
      <MarkdownContent content={'http://a.test/x 与 https://b.test/y'} onUrlClick={onUrlClick} />,
    )
    expect(html).toContain('data-url-href="http://a.test/x"')
    expect(html).toContain('data-url-href="https://b.test/y"')
  })

  it('[text](url) 语法仍是 <a> 标签，不被重复消费成 chip', () => {
    const html = renderToString(
      <MarkdownContent content="见 [文档](https://example.com/a)" onUrlClick={onUrlClick} />,
    )
    // 语法链接保持原行为：<a href target=_blank>，不产生 chip
    expect(html).toContain('<a href="https://example.com/a"')
    expect(html).toContain('target="_blank"')
    expect(html).not.toContain('markdown-url-path')
  })

  it('同一段里 [text](url) 与裸 URL 各走各的链路', () => {
    const html = renderToString(
      <MarkdownContent content={'[文字](https://a.test) 和 https://b.test 结束'} onUrlClick={onUrlClick} />,
    )
    expect(html).toContain('<a href="https://a.test"')
    expect(html).toContain('data-url-href="https://b.test"')
    // 裸 URL 不应只渲染出一个裸文本节点
    expect(html).not.toContain('>https://b.test<')
  })

  it('非 http/https 协议一律不识别（含 file: / javascript: / data:）', () => {
    const html = renderToString(
      <MarkdownContent
        content={'file:///C:/x javascript:alert(1) data:text/html,<b>x</b> ftp://h.test/f'}
        onUrlClick={onUrlClick}
      />,
    )
    expect(html).not.toContain('markdown-url-path')
    expect(extractUrls('file:///C:/x')).toEqual([])
    expect(extractUrls('javascript:alert(1)')).toEqual([])
    expect(extractUrls('ftp://h.test/f')).toEqual([])
  })

  it('行内代码与代码块里的 URL 不识别', () => {
    const inline = renderToString(
      <MarkdownContent content={'运行 `curl https://example.com/x` 即可'} onUrlClick={onUrlClick} />,
    )
    expect(inline).not.toContain('markdown-url-path')

    const block = renderToString(
      <MarkdownContent content={'```sh\ncurl https://example.com/x\n```'} onUrlClick={onUrlClick} />,
    )
    expect(block).not.toContain('markdown-url-path')
  })

  it('未传 onUrlClick 时零回归（裸 URL 保持纯文本）', () => {
    const html = renderToString(<MarkdownContent content="见 https://example.com/x 结束" />)
    expect(html).not.toContain('markdown-url-path')
    expect(html).not.toContain('data-url-href')
    expect(html).toContain('https://example.com/x')
  })

  it('句尾标点不被吞进链接', () => {
    const text = '看 https://example.com/a。'
    expect(extractUrls(text).map(r => text.slice(r.start, r.end))).toEqual([
      'https://example.com/a',
    ])
    const paren = '参考 (https://example.com/a) 即可'
    expect(extractUrls(paren).map(r => paren.slice(r.start, r.end))).toEqual([
      'https://example.com/a',
    ])
    // URL 内部成对括号属于 URL 的一部分，不能剥
    const inner = 'https://example.com/a_(b)'
    expect(extractUrls(inner).map(r => inner.slice(r.start, r.end))).toEqual([inner])
  })

  it('URL 与文件路径同段共存时互不串扰', () => {
    const mixed = String.raw`参考 https://example.com/a 以及 C:\out\c.rs`
    const ranges = extractUrls(mixed).map(r => mixed.slice(r.start, r.end))
    expect(ranges).toEqual(['https://example.com/a'])

    const html = renderToString(
      <MarkdownContent content={mixed} onUrlClick={onUrlClick} onFileClick={vi.fn()} />,
    )
    expect(html).toContain('data-url-href="https://example.com/a"')
    expect(html).toContain(String.raw`data-file-path="C:\out\c.rs"`)
  })

  it('紧邻前缀的片段不当成 URL 起头', () => {
    const text = 'xhttps://a.test/b'
    expect(extractUrls(text)).toEqual([])
  })

  it('列表 / 引用 / 表格等块级语法里的裸 URL 同样成链', () => {
    const html = renderToString(
      <MarkdownContent
        content={['- 列表 https://a.test/1', '', '> 引用 https://b.test/2', '', '| 链接 |', '| --- |', '| https://c.test/3 |'].join('\n')}
        onUrlClick={onUrlClick}
      />,
    )
    expect(html).toContain('data-url-href="https://a.test/1"')
    expect(html).toContain('data-url-href="https://b.test/2"')
    expect(html).toContain('data-url-href="https://c.test/3"')
  })
})