import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { IntentFormPanel } from './IntentFormPanel'
import { LangProvider } from '../../locales'

vi.mock('../lib/api', () => ({
  listDataDirs: async () => [{ key: 'plugin', path: 'test-workspace' }],
  getLanguage: async () => '',
}))
beforeEach(() => localStorage.clear())
describe('intent form lifecycle', () => {
  it('restores on reopening, keeps rejected submissions, and clears only explicitly', async () => {
    const submit = vi.fn().mockResolvedValue(false)
    const props = { initialName: 'Workflow', workflowId: 'wf', onClose: vi.fn(), onSubmit: submit }
    const first = render(<IntentFormPanel {...props} />)
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).not.toBeDisabled())
    fireEvent.change(screen.getByPlaceholderText(/阶段名称/), { target: { value: 'Stage draft' } })
    fireEvent.change(screen.getByPlaceholderText(/子步骤 1.1/), { target: { value: 'Step draft' } })
    first.unmount()
    render(<IntentFormPanel {...props} />)
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('Stage draft'))
    expect(screen.getByPlaceholderText(/子步骤 1.1/)).toHaveValue('Step draft')
    fireEvent.click(screen.getByRole('button', { name: '发送给 WorkflowAgent 生成工作流' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('提交失败')
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('Stage draft')
    fireEvent.click(screen.getByRole('button', { name: '清空草稿' }))
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('Stage draft')
    fireEvent.click(screen.getByRole('button', { name: '确认清空' }))
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('')
  })
  it('renders the complete intent form in English', async () => {
    localStorage.setItem('nuphus_language', 'en')
    render(
      <LangProvider>
        <IntentFormPanel
          initialName="My workflow"
          workflowId="en"
          onClose={() => {}}
          onSubmit={() => false}
        />
      </LangProvider>,
    )
    await waitFor(() => expect(screen.getByPlaceholderText(/Stage name/)).not.toBeDisabled())
    expect(screen.getByRole('dialog')).not.toHaveTextContent(/[\u4e00-\u9fff]/)
  })
})
