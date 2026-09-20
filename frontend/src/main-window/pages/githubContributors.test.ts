import { describe, expect, it } from 'vitest'
import { CONTRIBUTOR_ROUNDS, REPO_URL, profileUrl, pullUrl } from './githubContributors'

/**
 * 数据契约测试：贡献者名单是「可公开核对的事实」的落点，任何新增都必须自证来源
 * （见 githubContributors.ts 文件头的溯源说明）。这里锁住结构与形状，防止：
 *   ① 空数据/占位数据混入；② 主页链接被写成非 GitHub 地址；③ 轮次顺序被打乱。
 */
describe('githubContributors 数据契约', () => {
  const rounds = CONTRIBUTOR_ROUNDS

  it('轮次非空，版本号唯一且非空', () => {
    expect(rounds.length).toBeGreaterThan(0)
    const versions = rounds.map(r => r.version)
    expect(versions.every(v => v.trim().length > 0)).toBe(true)
    expect(new Set(versions).size).toBe(versions.length)
  })

  it('轮次倒序：未发布轮次（若有）在最前，已发布轮次按日期倒序', () => {
    // 未发布轮次最多一个，且只能出现在最前
    expect(rounds.filter(r => r.date === null).length).toBeLessThanOrEqual(1)
    const unreleasedAt = rounds.findIndex(r => r.date === null)
    if (unreleasedAt >= 0) expect(unreleasedAt).toBe(0)

    const dates = rounds.filter(r => r.date !== null).map(r => r.date as string)
    expect(dates.every(d => d.length > 0)).toBe(true)
    const sorted = [...dates].sort((a, b) => b.localeCompare(a))
    expect(dates).toEqual(sorted)
  })

  it('每个贡献者都有用户名与至少一条贡献，贡献描述与 PR 号非空', () => {
    for (const round of rounds) {
      expect(round.contributors.length).toBeGreaterThan(0)
      for (const contributor of round.contributors) {
        expect(contributor.user.trim().length).toBeGreaterThan(0)
        expect(contributor.user).not.toMatch(/\s/)
        expect(contributor.contributions.length).toBeGreaterThan(0)
        for (const item of contributor.contributions) {
          expect(Number.isInteger(item.pr) && item.pr > 0).toBe(true)
          expect(item.summary.trim().length).toBeGreaterThan(0)
        }
      }
    }
  })

  it('所有链接都是 GitHub 地址（主页一律 https://github.com/<user>）', () => {
    expect(REPO_URL.startsWith('https://github.com/')).toBe(true)
    for (const round of rounds) {
      for (const contributor of round.contributors) {
        expect(profileUrl(contributor.user)).toBe(`https://github.com/${contributor.user}`)
        expect(profileUrl(contributor.user).startsWith('https://github.com/')).toBe(true)
        for (const item of contributor.contributions) {
          expect(pullUrl(item.pr)).toBe(`${REPO_URL}/pull/${item.pr}`)
        }
      }
    }
  })

  it('同一 PR 号不重复出现在多个贡献者名下', () => {
    const prs = rounds.flatMap(r => r.contributors.flatMap(c => c.contributions.map(i => i.pr)))
    expect(new Set(prs).size).toBe(prs.length)
  })

  it('用户名集合与已核对的仓库记录一致（新增须同步更新溯源注释）', () => {
    const users = [...new Set(rounds.flatMap(r => r.contributors.map(c => c.user)))].sort()
    expect(users).toEqual(['jiangdingwei123-afk', 'yuansui486', 'zhoupeiyu515-ui'])
  })
})
