/**
 * githubContributors.ts — GitHub 贡献者页数据（设置中心 · 管理组 / Ctrl+K「GitHub」）
 *
 * 数据来源（全部可在本仓库复核）：
 *   ① `CHANGELOG.md` —— 各版本段落里的 PR 号与改动描述；
 *   ② `README.md`「致谢」段 —— 贡献者 GitHub 用户名与主页链接；
 *   ③ `git log` —— `Merge PR #NN` 合并提交及其分支内提交作者（把 PR 归属到人）。
 *
 * 用户名可信度（三者均经 GitHub API `/users/<login>` 200 校验，2026-09-20）：
 *   - yuansui486：README 致谢 #23 / #26 / #28 的链接用户名，且这些合并提交的作者名相同；
 *   - zhoupeiyu515-ui：README 致谢 #31 的链接用户名；#32 分支提交作者邮箱与 #31 完全一致（同一人）；
 *   - jiangdingwei123-afk：#21 分支提交作者名即 GitHub 用户名（README 未收录该轮次）。
 *   未收录 PR 归属：fouyzjl（#13~#19 / #20 轮次）——当时提交作者名无法确认为 GitHub 用户名，
 *   按「缺证不写」略去，宁缺勿造。2026-09-22 经 `/contributors` API 确认该账号真实存在
 *   且有 12 次提交，已并入头像墙（见 REPO_COMMITTERS）；轮次表内的贡献记录仍待
 *   CHANGELOG / README 出处再补，不凭印象回填。
 *
 * ⛔ 新增记录前必须先在上面的三处找到出处；不得凭印象补充贡献者或贡献内容。
 */

/** 本仓库主页（与 Ctrl+K 命令、旧筹备页使用同一地址） */
export const REPO_URL = 'https://github.com/mrpulor-gh/nuphus'

/** 贡献者主页 URL */
export const profileUrl = (user: string) => `https://github.com/${user}`

/**
 * GitHub 头像地址（圆形头像墙用）。
 *
 * ⚠️ 依赖两处放行，缺一即被拦成破图：
 *   1. CSP `img-src` 必须同时含 `https://github.com`（发起域）与
 *      `https://avatars.githubusercontent.com`（302 后的真实域）——见 src-tauri/tauri.conf.json；
 *   2. 网络可达。离线 / 被拦 / 404 时由同尺寸首字母色块兜底（见 GithubPage 头像墙），
 *      页面不会出现破图或空洞。
 */
export const avatarUrl = (user: string, size = 96) => `https://github.com/${user}.png?size=${size}`

/** PR 详情 URL */
export const pullUrl = (pr: number) => `${REPO_URL}/pull/${pr}`

export interface GithubContribution {
  /** Pull Request 号（来自 `Merge PR #NN` 合并记录） */
  pr: number
  /** 贡献内容（措辞对齐 CHANGELOG 段落 / README 致谢） */
  summary: string
}

export interface GithubContributor {
  /** GitHub 用户名（主页 = https://github.com/<user>） */
  user: string
  contributions: GithubContribution[]
}

export interface GithubRound {
  /** 发布轮次：CHANGELOG 段落标题里的版本号；未发布轮次为 `Unreleased` */
  version: string
  /** 已发布日期（CHANGELOG 段落标题里的日期）；未发布轮次为 null */
  date: string | null
  contributors: GithubContributor[]
}

/** 轮次倒序（最新在前），轮内按贡献时间先后 */
export const CONTRIBUTOR_ROUNDS: GithubRound[] = [
  {
    version: '0.2.17',
    date: '2026-09-21',
    contributors: [
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 32,
            summary: '设置中心（左导航 + 右内容）：居中弹窗、宿主分流、焦点陷阱与快捷键守卫',
          },
          { pr: 35, summary: '会话归属项目文件夹（后端）：归属落库、分组建模与偏好下发' },
          { pr: 36, summary: 'main 守卫工作流：push main 时检查 fmt / tsc / prettier' },
          { pr: 37, summary: '外部 Agent 契约补红线：不得擅自改动目标仓库的 git 历史' },
          {
            pr: 38,
            summary: '会话工作台按项目文件夹分组（前端）：分组渲染、整理/排序/恢复菜单、移动端同步',
          },
          { pr: 39, summary: '派发基线审计：记录工作区 HEAD，完工比对未派发提交' },
        ],
      },
      {
        user: 'yuansui486',
        contributions: [
          { pr: 33, summary: '内置浏览器：规避 CDP 运行期特征检测，并改进 Chrome 启动诊断' },
          { pr: 34, summary: '修复 Windows 浅色主题下主窗口黑边' },
        ],
      },
    ],
  },
  {
    version: '0.2.16',
    date: '2026-09-19',
    contributors: [
      {
        user: 'yuansui486',
        contributions: [
          { pr: 23, summary: 'macOS 系统权限检查与引导、Windows 工作流脚本子进程窗口隐藏' },
          { pr: 26, summary: 'macOS 麦克风权限改为被动查询' },
          {
            pr: 28,
            summary:
              '会话交互与文件路径识别：同行混排 Windows 路径丢失、裸域名误报、路径徽标复制语义、滚轮节流',
          },
        ],
      },
      {
        user: 'zhoupeiyu515-ui',
        contributions: [{ pr: 31, summary: 'DeepSeek 内置模型清单对齐官方 API' }],
      },
    ],
  },
  {
    version: '0.2.15',
    date: '2026-09-16',
    contributors: [
      {
        user: 'yuansui486',
        contributions: [
          {
            pr: 20,
            summary: '自定义服务商 API 接入点补齐，修复同名模型路由与视觉模型—Provider 绑定',
          },
        ],
      },
      {
        user: 'jiangdingwei123-afk',
        contributions: [
          { pr: 21, summary: 'npm 安装后 macOS / Linux 无法启动：启动器幂等补齐可执行位' },
        ],
      },
    ],
  },
]

/**
 * 仓库全部贡献者（GitHub `/contributors` API 快照，2026-09-22 拉取，按提交数降序）。
 *
 * 与 CONTRIBUTOR_ROUNDS 互补，二者取并集才是完整的「历史贡献者」：
 *   - 轮次表按 **PR 归属**记录（覆盖 GitHub 未把 commit 关联到账号的人，如 zhoupeiyu515-ui）；
 *   - 本表按 **commit 作者**记录（覆盖没有 PR 记录的提交，如 fouyzjl 的 12 次提交）。
 *
 * ⛔ 更新本表必须重跑（禁止凭印象增删）：
 *   curl -H 'Accept: application/vnd.github+json' \
 *     'https://api.github.com/repos/mrpulor-gh/nuphus/contributors?per_page=100'
 */
export const REPO_COMMITTERS: string[] = [
  'yuansui486', // 32 commits
  'fouyzjl', // 12 commits
  'mrpulor-gh', // 3 commits（仓库所有者）
  'jiangdingwei123-afk', // 1 commit
]

/**
 * 历史贡献者（跨来源去重）：先按提交数降序的 API 名单，再补轮次表里 API 未覆盖的人。
 * 用于页头头像墙：一眼可见「有多少人参与过」，不必逐轮次读下去。
 * 放在 CONTRIBUTOR_ROUNDS / REPO_COMMITTERS 之后：模块级 IIFE 会立刻求值，写在前面会撞上 TDZ。
 */
export const ALL_CONTRIBUTORS: string[] = (() => {
  const seen = new Set<string>()
  const out: string[] = []
  const push = (user: string) => {
    if (seen.has(user)) return
    seen.add(user)
    out.push(user)
  }
  for (const user of REPO_COMMITTERS) push(user)
  for (const round of CONTRIBUTOR_ROUNDS) {
    for (const c of round.contributors) push(c.user)
  }
  return out
})()
