import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider, useLanguage } from '../../locales'
import { OutlinePanel } from './OutlinePanel'
import type { WorkflowStep } from '../../core/types'

vi.mock('../lib/api', () => ({ getLanguage: async () => '' }))

beforeEach(() => localStorage.removeItem('nuphus_language'))

describe('OutlinePanel localization', () => {
  it('retains the Chinese guide by default', () => {
    render(<OutlinePanel steps={[]} selectedId={null} statuses={new Map()} onLocate={() => {}} />)
    expect(screen.getByText('画布操作')).toBeInTheDocument()
    expect(screen.getByText('容器进入子层')).toBeInTheDocument()
    expect(screen.getByText('双击')).toBeInTheDocument()
  })

  it('switches the complete empty guide to English without remounting', () => {
    function ChangeLanguage() {
      const { setLang } = useLanguage()
      return <button onClick={() => setLang('en')}>English</button>
    }
    render(
      <LangProvider>
        <ChangeLanguage />
        <OutlinePanel steps={[]} selectedId={null} statuses={new Map()} onLocate={() => {}} />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('button', { name: 'English' }))
    for (const text of [
      'Canvas shortcuts',
      'Double-click',
      'Open a container',
      'Go to parent layer',
      'Add a step',
      'Edit selected node',
      'Delete selection',
      'Save',
      'Undo / redo',
      'Run',
    ]) {
      expect(screen.getByText(text)).toBeInTheDocument()
    }
    expect(screen.queryByText('画布操作')).not.toBeInTheDocument()
  })

  it('localizes counts and statuses while preserving user node names and navigation', () => {
    localStorage.setItem('nuphus_language', 'en')
    const steps: WorkflowStep[] = [
      {
        id: 'group',
        name: '用户容器名称',
        do: { seq: [{ id: 'child', name: 'User wait', do: { sleep: 1 } }] },
      },
    ]
    const onLocate = vi.fn()
    render(
      <LangProvider>
        <OutlinePanel
          steps={steps}
          selectedId="child"
          statuses={new Map([['child', 'success']])}
          onLocate={onLocate}
        />
      </LangProvider>,
    )
    expect(screen.getByText('Outline · 2 steps')).toBeInTheDocument()
    expect(screen.getByText('1 step')).toBeInTheDocument()
    expect(screen.getByText('用户容器名称')).toBeInTheDocument()
    expect(screen.getByTitle('Previous run succeeded')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'User wait' }))
    expect(onLocate).toHaveBeenCalledWith('child')
  })
})
