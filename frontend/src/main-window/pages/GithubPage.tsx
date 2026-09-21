/**
 * GithubPage.tsx — 社区贡献者页（设置中心 · 管理组 + Ctrl+K「GitHub」）
 *
 * 定位：替代原「付费插件市场筹备页」。按**发布轮次**展示本仓库已合并的社区 PR：
 * 贡献者 → 贡献内容 → GitHub 主页（可点击）；说明栏内保留仓库入口。
 *
 * 数据：全部来自仓库真实记录，集中于 `./githubContributors`（溯源注释见该文件）；
 * 本文件不内联数据。头像统一为 **GitHub 圆形头像 + 首字母兜底**（页头头像墙 / 轮次卡片同一实现）：
 * 外链需 CSP 放行 github.com 与 avatars.githubusercontent.com；离线 / 被拦 / 404 时 img 自隐，
 * 露出同尺寸首字母色块——不出现破图或空洞。
 * 所有外链一律 `target="_blank"` + `rel="noreferrer"`。
 */
import type { CSSProperties } from 'react'
import { useLanguage } from '../../locales'
import { IconExternalLink } from '../../ui/Icons'
import {
  ALL_CONTRIBUTORS,
  CONTRIBUTOR_ROUNDS,
  REPO_URL,
  avatarUrl,
  profileUrl,
  pullUrl,
} from './githubContributors'
import '../../styles/github-contributors.css'

/**
 * 头像叠压幅度：相邻头像重叠自身宽度的 30%。
 * 单一来源在此，样式经 `--gh-overlap` 消费（改这里即改全站叠压，不必改样式表）。
 */
const AVATAR_OVERLAP = 0.3

/**
 * 头像内容：GitHub 圆形头像 + 同尺寸首字母兜底。
 *
 * 外链不可用（离线 / CSP 未放行 / 418/404）时 `onError` 让 img 自隐，露出底下的
 * 同尺寸色块——页面不会出现破图或空洞。页头头像墙与轮次卡片共用这一份实现。
 */
function AvatarMedia({ user }: { user: string }) {
  return (
    <>
      <span className="github-avatar-fallback" aria-hidden="true">
        {user.slice(0, 1).toUpperCase()}
      </span>
      <img
        className="github-avatar-img"
        src={avatarUrl(user)}
        alt={user}
        loading="lazy"
        decoding="async"
        onError={e => {
          e.currentTarget.style.display = 'none'
        }}
      />
    </>
  )
}

export function GithubPage() {
  const { t } = useLanguage()

  return (
    <div className="github-page">
      <div className="github-page-head">
        <div className="github-page-title">{t('github.title')}</div>
        <p className="github-page-subtitle">{t('github.subtitle')}</p>
        {/* 历史贡献者头像墙：圆形头像按 30% 前后叠压（幅度经 --gh-overlap 下发）。
            外链头像不可用时露出同尺寸首字母色块——不出现破图或空洞。 */}
        <div
          className="github-contributor-stack"
          style={{ '--gh-overlap': `-${AVATAR_OVERLAP}` } as CSSProperties}
        >
          {ALL_CONTRIBUTORS.map(user => (
            <a
              key={user}
              className="github-avatar"
              href={profileUrl(user)}
              target="_blank"
              rel="noreferrer"
              title={user}
            >
              <AvatarMedia user={user} />
            </a>
          ))}
        </div>
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
                      {/* 头像同页头：GitHub 圆形头像 + 首字母兜底（离线 / 被拦时露出色块）。
                          aria-hidden：名字就在旁边，头像纯装饰，避免可访问名重复 */}
                      <span className="github-contributor-avatar" aria-hidden="true">
                        <AvatarMedia user={contributor.user} />
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
