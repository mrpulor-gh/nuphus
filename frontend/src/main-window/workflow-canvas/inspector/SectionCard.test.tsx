import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { LangProvider } from '../../../locales'
import { SectionCard } from './SectionCard'

function IconStub() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 7V5h16v2M9 20h6M12 5v15" />
    </svg>
  )
}

describe('SectionCard', () => {
  it('标题行渲染标题 / 线性图标 / flag 微标，body 承载字段组', () => {
    render(
      <LangProvider>
        <SectionCard title="画面规格" icon={<IconStub />} flag="/do/model">
          <div data-testid="sec-body-child">字段</div>
        </SectionCard>
      </LangProvider>,
    )
    expect(screen.getByText('画面规格')).toHaveClass('wfp-sec__title')
    expect(screen.getByText('/do/model')).toHaveClass('wfp-sec__flag')
    // 图标包在 14px 线性图标容器里（inspector-panel.css .wfp-sec__icon）
    const head = screen.getByText('画面规格').closest('.wfp-sec__head')
    expect(head?.querySelector('.wfp-sec__icon svg')).not.toBeNull()
    expect(screen.getByTestId('sec-body-child')).toBeInTheDocument()
  })

  it('icon / flag 缺省时不渲染占位', () => {
    const { container } = render(
      <LangProvider>
        <SectionCard title="基本信息">
          <span>字段</span>
        </SectionCard>
      </LangProvider>,
    )
    expect(container.querySelector('.wfp-sec__icon')).toBeNull()
    expect(container.querySelector('.wfp-sec__flag')).toBeNull()
    expect(screen.getByText('基本信息')).toBeInTheDocument()
  })
})
