import { render, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { GithubPage } from './GithubPage'
import {
  ALL_CONTRIBUTORS,
  CONTRIBUTOR_ROUNDS,
  REPO_COMMITTERS,
  REPO_URL,
  pullUrl,
} from './githubContributors'

const contributorCount = CONTRIBUTOR_ROUNDS.reduce((n, r) => n + r.contributors.length, 0)
const contributionCount = CONTRIBUTOR_ROUNDS.reduce(
  (n, r) => n + r.contributors.reduce((m, c) => m + c.contributions.length, 0),
  0,
)

describe('GithubPage 贡献者页', () => {
  it('按发布轮次渲染：轮次标题、状态与 PR 数与数据一致', () => {
    const { container } = render(<GithubPage />)

    const rounds = [...container.querySelectorAll('.github-round')]
    expect(rounds).toHaveLength(CONTRIBUTOR_ROUNDS.length)

    // 轮次顺序与数据一致（最新在前）；未发布轮次显示「开发中」
    const versions = rounds.map(r => r.querySelector('.github-round-version')?.textContent)
    expect(versions).toEqual(CONTRIBUTOR_ROUNDS.map(r => (r.date ? `v${r.version}` : r.version)))
    const unreleasedIdx = CONTRIBUTOR_ROUNDS.findIndex(r => r.date === null)
    if (unreleasedIdx >= 0) expect(rounds[unreleasedIdx].textContent).toContain('开发中')

    // 每个轮次的 PR 计数 = 该轮贡献条目数
    rounds.forEach((node, index) => {
      const expected = CONTRIBUTOR_ROUNDS[index].contributors.reduce(
        (n, c) => n + c.contributions.length,
        0,
      )
      expect(within(node as HTMLElement).getByText(`${expected} 个 PR`)).toBeInTheDocument()
    })
  })

  it('每位贡献者都有主页链接（href 正确、target=_blank、rel=noreferrer）', () => {
    const { container } = render(<GithubPage />)

    // 只统计轮次卡片内的贡献者链接（页头头像墙也是同样的主页链接，另有用例覆盖）
    const rounds = container.querySelector('.github-rounds') as HTMLElement
    const links = within(rounds).getAllByRole('link')
    const profileLinks = links.filter(link => {
      const href = link.getAttribute('href') ?? ''
      return href.startsWith('https://github.com/') && href !== REPO_URL && !href.includes('/pull/')
    })

    // 仓库入口（https://github.com/<org>/<repo>）与 PR 链接不计入「贡献者主页」
    expect(profileLinks).toHaveLength(contributorCount)
    expect(profileLinks.some(l => l.getAttribute('href') === REPO_URL)).toBe(false)

    for (const link of profileLinks) {
      expect(link).toHaveAttribute('target', '_blank')
      expect(link).toHaveAttribute('rel', 'noreferrer')
    }
    // 每个用户名（同一人可能出现在多个轮次）都能点击跳转到自己的主页
    const users = [...new Set(CONTRIBUTOR_ROUNDS.flatMap(r => r.contributors.map(c => c.user)))]
    for (const user of users) {
      const linksForUser = screen.getAllByRole('link', { name: user })
      expect(linksForUser.length).toBeGreaterThan(0)
      for (const link of linksForUser) {
        expect(link).toHaveAttribute('href', `https://github.com/${user}`)
      }
    }
  })

  it('每条贡献都有内容与 PR 链接（指向本仓库的 pull/<n>）', () => {
    render(<GithubPage />)

    const prLinks = screen
      .getAllByRole('link')
      .filter(link => (link.getAttribute('href') ?? '').includes('/pull/'))
    expect(prLinks).toHaveLength(contributionCount)

    for (const round of CONTRIBUTOR_ROUNDS) {
      for (const contributor of round.contributors) {
        for (const item of contributor.contributions) {
          const link = screen.getByRole('link', { name: `#${item.pr}` })
          expect(link).toHaveAttribute('href', pullUrl(item.pr))
          expect(link).toHaveAttribute('target', '_blank')
          expect(link).toHaveAttribute('rel', 'noreferrer')
          expect(screen.getByText(item.summary)).toBeInTheDocument()
        }
      }
    }
  })

  it('保留仓库入口链接，并给出数据来源说明', () => {
    render(<GithubPage />)

    const repoLink = screen.getByRole('link', { name: /打开仓库/ })
    expect(repoLink).toHaveAttribute('href', REPO_URL)
    expect(repoLink).toHaveAttribute('target', '_blank')
    expect(repoLink).toHaveAttribute('rel', 'noreferrer')
    expect(screen.getByText(/数据来自本仓库 CHANGELOG/)).toBeInTheDocument()
  })

  it('标题下说明栏：欢迎提交 issue/PR 的号召文案，且仓库入口位于说明栏内', () => {
    const { container } = render(<GithubPage />)

    const intro = screen.getByText(/我们鼓励由使用者到共同开发者的转变/)
    expect(intro).toHaveClass('github-page-intro')
    // 位置约束：说明文案与仓库入口同属页头说明栏（.github-page-head），不再挂在页脚
    const head = container.querySelector('.github-page-head')
    expect(head).not.toBeNull()
    expect(head?.contains(intro)).toBe(true)
    expect(head?.contains(screen.getByRole('link', { name: /打开仓库/ }))).toBe(true)
    expect(container.querySelector('.github-page-foot .github-repo-entry')).toBeNull()
  })

  it('头像：页头头像墙与轮次卡片统一为 GitHub 圆形头像 + 首字母兜底', () => {
    const { container } = render(<GithubPage />)

    // 页头头像墙：每位历史贡献者一枚，位置紧跟副标题
    const stack = container.querySelector('.github-contributor-stack') as HTMLElement
    expect(stack).not.toBeNull()
    expect(stack.previousElementSibling).toHaveClass('github-page-subtitle')

    const avatars = [...stack.querySelectorAll('.github-avatar')]
    expect(avatars).toHaveLength(ALL_CONTRIBUTORS.length)
    expect([...stack.querySelectorAll('img')].map(i => i.getAttribute('src'))).toEqual(
      ALL_CONTRIBUTORS.map(u => `https://github.com/${u}.png?size=96`),
    )

    // 轮次卡片内的头像同款（原先的首字母色块一并替换）
    const rounds = container.querySelector('.github-rounds') as HTMLElement
    const roundAvatars = [...rounds.querySelectorAll('.github-contributor-avatar')]
    expect(roundAvatars).toHaveLength(contributorCount)
    const roundImgs = [...rounds.querySelectorAll('img')]
    expect(roundImgs).toHaveLength(contributorCount)
    for (const img of roundImgs) {
      expect(img.getAttribute('src')).toMatch(/^https:\/\/github\.com\/[\w-]+\.png\?size=96$/)
    }

    // 每枚（含两处）都有同尺寸首字母兜底：离线 / 被 CSP 拦 / 404 时不出现破图或空洞
    for (const avatar of [...avatars, ...roundAvatars]) {
      const fallback = avatar.querySelector('.github-avatar-fallback')
      expect(fallback?.textContent).toMatch(/^[A-Z0-9]$/)
      expect(fallback).toHaveAttribute('aria-hidden', 'true')
    }
  })

  it('叠压纪律：相邻头像按尺寸的 30% 重叠（幅度经 --gh-overlap 下发）', () => {
    const { container } = render(<GithubPage />)
    const stack = container.querySelector('.github-contributor-stack') as HTMLElement
    // jsdom 不算布局，故钉住「下发给样式表的叠压变量」本身（30% → -0.3）
    expect(stack.style.getPropertyValue('--gh-overlap')).toBe('-0.3')
  })

  it('头像列并入 GitHub `/contributors` 名单：与轮次表取并集、去重、API 序在前', () => {
    // 提交数降序的 API 名单排在前，轮次表里 API 未覆盖的人补在后
    expect(ALL_CONTRIBUTORS.slice(0, REPO_COMMITTERS.length)).toEqual(REPO_COMMITTERS)

    // 并集 = API 名单 ∪ 轮次表用户；且无重复
    const roundUsers = [
      ...new Set(CONTRIBUTOR_ROUNDS.flatMap(r => r.contributors.map(c => c.user))),
    ]
    expect(new Set(ALL_CONTRIBUTORS)).toEqual(new Set([...REPO_COMMITTERS, ...roundUsers]))
    expect(ALL_CONTRIBUTORS).toHaveLength(new Set(ALL_CONTRIBUTORS).size)

    // fouyzjl：轮次表当初按「缺证不写」略去，本次由 API 确认（12 次提交）后并入
    expect(ALL_CONTRIBUTORS).toContain('fouyzjl')
    // zhoupeiyu515-ui：API 未把其 commit 关联到账号 → 只能由轮次表带来
    expect(ALL_CONTRIBUTORS).toContain('zhoupeiyu515-ui')
  })
})
