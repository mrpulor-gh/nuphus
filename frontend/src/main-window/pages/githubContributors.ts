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
 *   未收录：zjl（#13~#19 / #20 轮次）——提交作者名无法确认为 GitHub 用户名，
 *   按「缺证不写」略去，宁缺勿造。
 *
 * ⛔ 新增记录前必须先在上面的三处找到出处；不得凭印象补充贡献者或贡献内容。
 */

/** 本仓库主页（与 Ctrl+K 命令、旧筹备页使用同一地址） */
export const REPO_URL = 'https://github.com/mrpulor-gh/nuphus'

/** 贡献者主页 URL */
export const profileUrl = (user: string) => `https://github.com/${user}`

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
    version: 'Unreleased',
    date: null,
    contributors: [
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 32,
            summary: '设置中心（左导航 + 右内容）：居中弹窗、宿主分流、焦点陷阱与快捷键守卫',
          },
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
