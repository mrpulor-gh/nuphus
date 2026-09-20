/**
 * GithubPage.tsx — 社区贡献者页（设置中心 · 管理组 + Ctrl+K「GitHub」）
 *
 * 定位：替代原「付费插件市场筹备页」。按**发布轮次**展示本仓库已合并的社区 PR：
 * 贡献者 → 贡献内容 → GitHub 主页（可点击）；说明栏内保留仓库入口。
 *
 * 数据：全部来自仓库真实记录，集中于 `./githubContributors`（溯源注释见该文件）；
 * 本文件不内联数据、不联网、不引用外链图片——头像用用户名首字母色块，离线可用。
 * 所有外链一律 `target="_blank"` + `rel="noreferrer"`。
 */
import { useLanguage } from '../../locales'
import { IconExternalLink } from '../../ui/Icons'
import { CONTRIBUTOR_ROUNDS, REPO_URL, profileUrl, pullUrl } from './githubContributors'
import '../../styles/github-contributors.css'

export function GithubPage() {
  const { t } = useLanguage()

  return (
    <div className="github-page">
      <div className="github-page-head">
        <div className="github-page-title">{t('github.title')}</div>
        <p className="github-page-subtitle">{t('github.subtitle')}</p>
        <p className="github-page-intro">{t('github.intro')}</p>
        <a className="github-repo-entry" href={REPO_URL} target="_blank" rel="noreferrer">
          <IconExternalLink size={13} />
          <span>{t('github.repoEntry')}</span>
        </a>
      </div>

      <div className="github-rounds">
        {CONTRIBUTOR_ROUNDS.map(round => {
          const prCount = round.contributors.reduce((n, c) => n + c.contributions.length, 0)
          return (
            <section className="github-round" key={round.version}>
              <div className="github-round-head">
                {/* 未发布轮次没有日期 → 版本号用 CHANGELOG 原词，状态位显示「开发中」 */}
                <span className="github-round-version">
                  {round.date ? `v${round.version}` : round.version}
                </span>
                <span className="github-round-meta">
                  {round.date ?? t('github.roundInProgress')}
                </span>
                <span className="github-round-meta">{t('github.prCount', String(prCount))}</span>
              </div>

              <div className="github-contributors">
                {round.contributors.map(contributor => (
                  <article className="github-contributor" key={contributor.user}>
                    <a
                      className="github-contributor-head"
                      href={profileUrl(contributor.user)}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {/* 首字母色块替代头像：不引用外链图片，离线与断网时同样可读 */}
                      <span className="github-contributor-avatar" aria-hidden="true">
                        {contributor.user.slice(0, 1).toUpperCase()}
                      </span>
                      <span className="github-contributor-name">{contributor.user}</span>
                      <IconExternalLink size={12} />
                    </a>

                    <ul className="github-contributor-list">
                      {contributor.contributions.map(item => (
                        <li className="github-contributor-item" key={item.pr}>
                          <a
                            className="github-pr-link"
                            href={pullUrl(item.pr)}
                            target="_blank"
                            rel="noreferrer"
                          >
                            #{item.pr}
                          </a>
                          <span className="github-contributor-summary">{item.summary}</span>
                        </li>
                      ))}
                    </ul>
                  </article>
                ))}
              </div>
            </section>
          )
        })}
      </div>

      <div className="github-page-foot">
        <p className="github-data-note">{t('github.dataNote')}</p>
      </div>
    </div>
  )
}
