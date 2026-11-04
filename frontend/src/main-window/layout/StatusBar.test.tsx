import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { LangProvider } from '../../locales'
import { StatusBar } from './StatusBar'

/** 后端 main 发射点 / 历史恢复回填共用的「无读数」哨兵（useEvents.ts 同款判据） */
const SENTINEL = 0xffffffff

function renderBar(mainTokenUsage: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null) {
  return render(
    <LangProvider>
      <StatusBar mainTokenUsage={mainTokenUsage} execTokenUsage={null} contextLimit={1_000_000} />
    </LangProvider>,
  )
}

describe('StatusBar 缓存命中读数', () => {
  it('真实读数 → 渲染百分比（51200 / 130801 ≈ 39%）', () => {
    renderBar({ inputTokens: 130801, outputTokens: 61, cacheHitTokens: 51200 })
    expect(screen.getByText('39%')).toBeTruthy()
  })

  it('哨兵 0xffffffff（无读数）→ 整项不渲染，不得冒充几千%', () => {
    renderBar({ inputTokens: 116936, outputTokens: 0, cacheHitTokens: SENTINEL })
    expect(screen.queryByText(/%/)).toBeNull()
  })

  it('cacheHitTokens = 0 → 不渲染（真实未命中与无读数同样不显示）', () => {
    renderBar({ inputTokens: 130801, outputTokens: 61, cacheHitTokens: 0 })
    expect(screen.queryByText(/%/)).toBeNull()
  })

  it('mainTokenUsage = null（新会话未知态）→ 不渲染', () => {
    renderBar(null)
    expect(screen.queryByText(/%/)).toBeNull()
  })
})
