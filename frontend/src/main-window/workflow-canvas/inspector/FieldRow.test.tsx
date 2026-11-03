import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { LangProvider } from '../../../locales'
import { FieldRow } from './FieldRow'

function renderRow(props: Partial<Parameters<typeof FieldRow>[0]> = {}) {
  const { container } = render(
    <LangProvider>
      <FieldRow label="生成数量" {...props}>
        <input aria-label="control" />
      </FieldRow>
    </LangProvider>,
  )
  return { container }
}

describe('FieldRow', () => {
  it('label / control 两列形态（容器 ≥520px 时 CSS 生效，类名不带 stack）', () => {
    const { container } = renderRow()
    const row = container.querySelector('.wfp-frow')
    expect(row).not.toBeNull()
    expect(row).not.toHaveClass('wfp-frow--stack')
    expect(screen.getByText('生成数量')).toHaveClass('wfp-frow__text')
    expect(screen.getByLabelText('control')).toBeInTheDocument()
  })

  it('required 渲染 accent 必填星', () => {
    const { container } = renderRow({ required: true })
    const star = container.querySelector('.wfp-frow__req')
    expect(star).not.toBeNull()
    expect(star).toHaveTextContent('*')
  })

  it('help 渲染帮助图标并带 title（hover 说明）', () => {
    const { container } = renderRow({ help: '越低越稳定；留空用模型默认' })
    const help = container.querySelector('.wfp-frow__help')
    expect(help).not.toBeNull()
    expect(help).toHaveAttribute('title', '越低越稳定；留空用模型默认')
    expect(help?.querySelector('svg')).not.toBeNull()
  })

  it('hint 在无 error 时渲染在 control 下方', () => {
    const { container } = renderRow({ hint: '留空用模型默认' })
    const hint = container.querySelector('.wfp-frow__hint')
    expect(hint).toHaveTextContent('留空用模型默认')
    expect(container.querySelector('.wfp-frow__err')).toBeNull()
  })

  it('error 替换 hint 位不双显', () => {
    const { container } = renderRow({ hint: '留空用模型默认', error: '需为整数' })
    expect(container.querySelector('.wfp-frow__hint')).toBeNull()
    const err = container.querySelector('.wfp-frow__err')
    expect(err).toHaveTextContent('需为整数')
    expect(err?.querySelector('svg')).not.toBeNull()
  })

  it('stack prop 强制单列堆叠形态', () => {
    const { container } = renderRow({ stack: true })
    expect(container.querySelector('.wfp-frow')).toHaveClass('wfp-frow--stack')
    expect(screen.getByText('生成数量')).toBeInTheDocument()
  })

  it('fieldPath 渲染 data-field-path 与 data-draft-field（草稿定位钩子）', () => {
    const { container } = renderRow({ fieldPath: '/do/with/count' })
    const row = container.querySelector('.wfp-frow')
    expect(row).toHaveAttribute('data-field-path', '/do/with/count')
    expect(row).toHaveAttribute('data-draft-field', '/do/with/count')
  })

  it('fieldPath 缺省时不渲染两个 data 属性', () => {
    const { container } = renderRow()
    const row = container.querySelector('.wfp-frow')
    expect(row).not.toHaveAttribute('data-field-path')
    expect(row).not.toHaveAttribute('data-draft-field')
  })
})
