import { describe, expect, it } from 'vitest'
import { renderToString } from 'react-dom/server'
import MarkdownContent from '../main-window/chat/MarkdownContent'

/**
 * Issue #66 验证钉：过程性 text 的段落边界。
 * 核心断言：块内单 `\n` 必须产出 md-line 行元素（获得行级断点），
 * 而 `\n\n` 仍产出多个 <p>（段落语义不破）。
 */
describe('Issue #66 段落/行边界', () => {
  it('A. 双换行分段 → 多个 <p>（交付内容语义不破）', () => {
    const content = ['第一段。', '第二段。', '第三段。'].join('\n\n')
    const html = renderToString(<MarkdownContent content={content} />)
    expect((html.match(/markdown-paragraph/g) || []).length).toBe(3)
  })

  it('B. 单换行 → 同一 <p> 内多个 .md-line（issue 主诉）', () => {
    const content = [
      '实际扫一遍，不猜。',
      '处理这个目录：先看内容与权限，再删。',
      '删除失败，exit=1，文件仍在。',
    ].join('\n')
    const html = renderToString(<MarkdownContent content={content} />)
    expect((html.match(/markdown-paragraph/g) || []).length).toBe(1)
    expect((html.match(/md-line/g) || []).length).toBe(3)
  })

  it('C. 混合：过程(单换行) + 交付(双换行) 共存', () => {
    const content = '过程一\n过程二\n\n正式交付段落。'
    const html = renderToString(<MarkdownContent content={content} />)
    expect((html.match(/markdown-paragraph/g) || []).length).toBe(2)
    expect((html.match(/md-line/g) || []).length).toBe(2)
  })

  it('D. 单行零回归：不产生 md-line 包裹', () => {
    const html = renderToString(<MarkdownContent content={'就一行普通文字。'} />)
    expect(html).toContain('就一行普通文字。')
    expect(html).not.toContain('md-line')
  })

  it('E. 行内标记（加粗/代码）在拆行后仍正常', () => {
    const html = renderToString(<MarkdownContent content={'**粗** 与 `code`\n第二行'} />)
    expect(html).toContain('<strong')
    expect(html).not.toContain('**粗**')
    expect((html.match(/md-line/g) || []).length).toBe(2)
  })
})
