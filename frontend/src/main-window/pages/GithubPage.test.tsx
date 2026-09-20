import { render, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { GithubPage } from './GithubPage'
import { CONTRIBUTOR_ROUNDS, REPO_URL, pullUrl } from './githubContributors'

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
    render(<GithubPage />)

    const links = screen.getAllByRole('link')
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

  it('头像为首字母色块：不引用外链图片，首字母大写，且不污染可访问名', () => {
    const { container } = render(<GithubPage />)

    expect(container.querySelector('img')).toBeNull()
    const avatars = [...container.querySelectorAll('.github-contributor-avatar')]
    expect(avatars).toHaveLength(contributorCount)
    for (const avatar of avatars) {
      expect(avatar.textContent).toMatch(/^[A-Z0-9]$/)
      expect(avatar).toHaveAttribute('aria-hidden', 'true')
    }
  })
})
