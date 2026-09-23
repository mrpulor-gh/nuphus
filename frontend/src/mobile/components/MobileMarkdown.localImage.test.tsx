import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import MobileMarkdown from './MobileMarkdown'

const WINDOWS_PATH = String.raw`E:\NUS\_settings_popup_wide.png`

/** 渲染为静态 HTML：只断言「哪些文本进了图片通道」，组件内拉取行为见 LocalImage.test.tsx */
function render(content: string): string {
  return renderToString(<MobileMarkdown content={content} />)
}

describe('MobileMarkdown 本机图片路径', () => {
  it('段落里的图片绝对路径渲染为缩略图容器，前后文字保留', () => {
    const html = render(`验证截图：${WINDOWS_PATH}`)
    expect(html).toContain(`data-image-path="${WINDOWS_PATH}"`)
    expect(html).toContain('m-local-image')
    expect(html).toContain('验证截图：')
  })

  it('Unix / UNC 路径同样进入图片通道', () => {
    expect(render('见 /tmp/shots/home.png 完成')).toContain('data-image-path="/tmp/shots/home.png"')
    expect(render(String.raw`共享 \\nas\shots\login.bmp`)).toContain(
      String.raw`data-image-path="\\nas\shots\login.bmp"`,
    )
  })

  it('行内代码整段即图片路径时按图片渲染（Agent 常用反引号包路径）', () => {
    const html = render(`截图：\`${WINDOWS_PATH}\``)
    expect(html).toContain(`data-image-path="${WINDOWS_PATH}"`)
    expect(html).not.toContain('inline-code')
  })

  it('列表项与表格单元格内的路径同样识别', () => {
    expect(render(`- 产物 ${WINDOWS_PATH}`)).toContain(`data-image-path="${WINDOWS_PATH}"`)
    const table = ['| 步骤 | 截图 |', '| --- | --- |', `| 1 | ${WINDOWS_PATH} |`].join('\n')
    expect(render(table)).toContain(`data-image-path="${WINDOWS_PATH}"`)
  })

  it('非图片路径保持纯文本（行为不变），行内代码不受影响', () => {
    const html = render(String.raw`日志 E:\NUS\notes.txt 已保存`)
    expect(html).not.toContain('data-image-path')
    expect(html).toContain(String.raw`E:\NUS\notes.txt`)

    const codeHtml = render('`const a = 1`')
    expect(codeHtml).toContain('inline-code')
    expect(codeHtml).not.toContain('data-image-path')
  })

  it('URL / markdown 链接 / 代码块里的图片不误伤', () => {
    expect(render('https://example.com/a.png')).not.toContain('data-image-path')
    expect(render('[截图](https://example.com/a.png)')).not.toContain('data-image-path')
    expect(render(`\`\`\`\n${WINDOWS_PATH}\n\`\`\``)).not.toContain('data-image-path')
  })
})
