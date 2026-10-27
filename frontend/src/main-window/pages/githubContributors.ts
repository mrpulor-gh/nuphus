/**
 * githubContributors.ts — GitHub 贡献者页数据（设置中心 · 管理组 / Ctrl+K「GitHub」）
 *
 * 数据来源（全部可在本仓库复核）：
 *   ① `CHANGELOG.md` —— 各版本段落里的 PR 号与改动描述；
 *   ② `README.md`「致谢」段 —— 贡献者 GitHub 用户名与主页链接；
 *   ③ `git log` —— `Merge PR #NN` 合并提交及其分支内提交作者（把 PR 归属到人）。
 *
 * 用户名可信度（均经 GitHub API `/users/<login>` 200 校验）：
 *   - yuansui486：README 致谢 #23 / #26 / #28 的链接用户名，且这些合并提交的作者名相同；
 *   - zhoupeiyu515-ui：README 致谢 #31 的链接用户名；#32 分支提交作者邮箱与 #31 完全一致（同一人）。
 *   - Steooenwolf-666：2026-09-28 经 `/contributors` API 确认该账号真实存在且已有 21 次提交，
 *     本轮 PR #67 / #70~#75 的作者均为该账号，头像 u/156271754——新增提交者，按发版铁律
 *     补入头像墙与轮次表。
 *   未收录 PR 归属：fouyzjl（#13~#19 / #20 轮次）——当时提交作者名无法确认为 GitHub 用户名，
 *   按「缺证不写」略去，宁缺勿造。2026-09-22 经 `/contributors` API 确认该账号真实存在
 *   且有 12 次提交，已并入头像墙（见 REPO_COMMITTERS）；轮次表内的贡献记录仍待
 *   CHANGELOG / README 出处再补，不凭印象回填。#79（0.2.23）已按同一出处补入。
 *   （2026-09-29 重跑该 API：fouyzjl 已增至 13 次提交。）
 *
 * `Unreleased` 轮次（#87 / #88 / #84）取证：2026-09-29 经 `/compare/v0.2.23...main` API
 * 确认该区间 ahead=27 / behind=0，并用 `/pulls?state=closed` 核对三个 PR 的 merged_at
 * 与作者（#87 01:49 与 #88 10:48 均 yuansui486，#84 10:53 为 zhoupeiyu515-ui），
 * 三者均能在 git log 找到 `Merge PR #NN` 合并提交。两位作者均已在既有轮次出现，
 * 非新提交者，故头像墙无需新增。该组随 v0.2.24 发行（tag 日期 2026-09-30，
 * 与 CHANGELOG `## [0.2.24] - 2026-09-30` 一致）；#91 同轮追加：2026-09-29 16:09
 * 经 `/issues/91/events` 确认 merged，作者 Steooenwolf-666 已在 0.2.23 轮次出现
 * （非新提交者），出处为 `Merge PR #91`（fb52f668）。
 *
 * `0.2.25` 轮次（零 PR）取证：2026-10-02 经 `/pulls?state=closed` API 核对，最后合并的 PR
 * 仍是 #91（2026-09-29，已收录于 0.2.24 轮次，非新提交者）；`git log v0.2.24..HEAD`
 * 区间 24 个提交全部为仓库所有者直推、零合并 PR。经大王指示，本轮的作者重要功能以
 * **commit 出处**补入（`pr: null` + `commit: 短 sha`，`/commit/<sha>` 可公开核对），
 * 不写 PR 号——无出处不写，宁缺勿造。
 *
 * `0.2.26` 轮次（零 PR）取证：2026-10-05 重跑 `/pulls?state=closed` API，最后合并的 PR 仍是
 * #91（2026-09-29，已收录于 0.2.24 轮次，非新提交者）；`/contributors` API 六个账号与提交数
 * 与 REPO_COMMITTERS 完全一致（头像墙无新增）。`git log v0.2.25..HEAD` 区间同样零合并 PR，
 * 全为仓库所有者直推。经大王指示（本轮无 PR），作者自己的重要功能继续以 **commit 出处**
 * 补入（`pr: null` + `commit: 短 sha`，`/commit/<sha>` 可公开核对），措辞对齐提交标题；
 * 纯格式 / lint 提交不收录。
 *
 * `0.2.27` 轮次（零 PR）取证：2026-10-10 经 `/pulls?state=closed` API 核对，v0.2.26（2026-10-05）
 * 之后无任何合并 PR（最后合并仍是 #91，2026-09-29，已收录，非新提交者）；`/contributors`
 * API 六账号与 REPO_COMMITTERS 一致，头像墙无新增。`git log v0.2.26..HEAD` 区间 38 个提交
 * 全为仓库所有者直推，按既定口径以 **commit 出处**收录重要功能（`pr: null` + `commit: 短 sha`）；
 * 纯格式 / lint / 发版杂务提交不收录。
 *
 * `0.2.28` 轮次取证：2026-10-12 经 `/pulls?state=closed` API 核对，v0.2.27（2026-10-10）之后
 * 唯一合并 PR 为 #104（zhoupeiyu515-ui，热门模型快捷 chip + 筛选框多关键词，2026-10-11 合并，
 * merge commit 3dc1dd4b）。`git log v0.2.27..HEAD` 区间
 * 其余提交全为仓库所有者直推，按既定口径以 **commit 出处**收录重要功能；纯格式 / lint /
 * 发版杂务提交不收录。历史轮次中的 #21 条目经所有者复核不属于本仓库贡献记录，已移除，
 * 不得回填。
 *
 * `0.2.29` 轮次取证：2026-10-15 经 `/pulls?state=closed` 与 `/compare/v0.2.28...main` API 核对，
 * v0.2.28（2026-10-12）之后唯一合并 PR 为 #105（zhoupeiyu515-ui，会话行 hover 重叠修复 +
 * ModelsPage 测试基线恢复，2026-10-14 合并，merge commit 4110640b；含 Zpy 署名的两个
 * squash 内 commit，同一账号）。zhoupeiyu515-ui 已在头像墙（v0.2.24 起收录），无新提交者。
 * 区间其余 42 个提交全为仓库所有者直推，按既定口径以 **commit 出处**收录重要功能；
 * 纯格式 / lint / 发版杂务提交不收录。
 *
 * `0.2.30` 轮次（零 PR）取证：2026-10-19 经 `/pulls?state=closed` API 核对，v0.2.29（2026-10-15）
 * 之后至 v0.2.30（2026-10-17）无任何合并 PR（最后合并仍是 #105，2026-10-14，已收录于 0.2.29
 * 轮次，非新提交者）；`git log v0.2.29..v0.2.30` 全为仓库所有者直推，按既定口径以 **commit 出处**
 * 收录重要功能（`pr: null` + `commit: 短 sha`）；纯格式 / lint / 发版杂务提交不收录。
 *
 * `0.2.31` 轮次取证：2026-10-19 经 `/pulls?state=closed` API 核对，v0.2.30 之后合并 #107
 * （ChatGPT 套餐登录与 Responses 推理，2026-10-18 23:19:33）与 #108（IPC 参数修复，同日
 * 23:19:17），作者均为 Steooenwolf-666；该账号已在 0.2.23 轮次收录，非新提交者，头像墙无新增。
 * 区间其余提交全为仓库所有者直推，按既定口径以 **commit 出处**收录重要功能；纯格式 / lint /
 * 发版杂务提交不收录。`/contributors` API 六个账号与既有列表一致。
 *
 * `1.0.0` 轮次（零 PR）取证：2026-10-22 经 `/pulls?state=closed` API 核对，v0.3.1（2026-10-21）
 * 之后无任何合并 PR（最后合并仍是 #108，2026-10-18，已收录于 0.2.31 轮次）；`/contributors`
 * API 六账号与 REPO_COMMITTERS 完全一致（78/24/13/4/1/1），头像墙无新增。区间提交全为仓库
 * 所有者直推，按既定口径以 **commit 出处** 收录重要功能；纯格式 / lint / 发版杂务提交不收录。
 *
 * `1.1.0` 轮次取证：2026-10-24 经 `/search/issues?q=is:pr+is:merged+merged:>=2026-10-22` API
 * 核对，v1.0.0（2026-10-22）之后合并 PR 共两个：#110（Steooenwolf-666，启动更新提醒 +
 * 任务期间延迟安装两个分支提交，2026-10-22 23:18 合并，merge commit 2a9e2f2a）与 #112
 * （zhoupeiyu515-ui，滑块 step=1 + 行内「恢复默认」，2026-10-24 06:04 合并，merge commit
 * 8c615e2e；分支提交 312bd64d 作者署名 Zpy，与 0.2.29 轮次备注的邮箱归属一致）。
 * 两位均非新提交者（前者自 0.2.23 轮次起在头像墙，后者见于 0.2.24/0.2.29 等轮次），
 * REPO_COMMITTERS 六账号无新增、头像墙不变。`git log v1.0.0..HEAD` 区间其余提交全为仓库
 * 所有者直推，按既定口径以 **commit 出处**收录重要功能；3ac9dbb9（oauth_callback 断言
 * 放宽）属测试基建、b951090f 为 1.0.0  latest.json 资产刷新，均不收录。
 *
 * `1.1.2` 轮次取证：2026-10-26，v1.1.1（2026-10-25）之后**零合并社区 PR**（最后合并仍为 #116/#115，
 * 已收录于 1.1.1 轮次，非新提交者）。按既有口径，本轮作者自己的重要功能以 **commit 出处**补入
 * （`pr: null` + 短 sha，`/commit/<sha>` 可公开核对）；纯格式 / lint 提交不收录。
 *
 * `1.1.1` 轮次取证：2026-10-25，v1.1.0（2026-10-24）之后合并 PR 共两个：#115
 * （zhoupeiyu515-ui，ego lite 外部 CDP 浏览器识别，merge a624a617）与 #116
 * （zhoupeiyu515-ui，切换主题后背景图残留，merge 8a13ffcf；合并后维护者补
 * df7dc1d2 释放过期解析的 blob URL）。作者已在 0.2.24 起收录于头像墙，非新
 * 提交者，REPO_COMMITTERS 无新增。维护者直推的核心修复按既定口径以 **commit
 * 出处**收录（d89e1f4 / ce29afb / 68abc7b），纯格式 / 杂务提交不收录。
 *
 * ⛔ 新增记录前必须先在上面的三处找到出处并经所有者确认；不得凭印象补充贡献者或贡献内容。
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

/** 提交详情 URL（作者直推、无 PR 时的出处，`/commit/<sha>` 可公开核对） */
export const commitUrl = (sha: string) => `${REPO_URL}/commit/${sha}`

export interface GithubContribution {
  /** Pull Request 号（来自 `Merge PR #NN` 合并记录；作者直推提交无 PR，此时为 null） */
  pr: number | null
  /** 作者直推提交的短 sha（无 PR 时给出处，GitHub `/commit/<sha>` 可核对；有 PR 时可省略） */
  commit?: string | null
  /** 贡献内容（措辞对齐 CHANGELOG 段落 / README 致谢） */
  summary: string
}

/**
 * 贡献条目的出处徽标：有 PR 号显示 `#N` 指向 pull；作者直推提交显示短 sha 指向 commit。
 * 二者必有且仅有一（由数据契约测试钉住）。
 */
export const contributionRef = (item: GithubContribution): { label: string; url: string } =>
  item.pr !== null
    ? { label: `#${item.pr}`, url: pullUrl(item.pr) }
    : { label: String(item.commit ?? ''), url: commitUrl(item.commit ?? '') }

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
    version: '1.1.2',
    date: '2026-10-26',
    contributors: [
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '9d692ce9',
            summary:
              'Exec 执行指示器：聊天区左上角常驻头像，执行中边缘弧环绕、点击直达执行追踪面板、60 秒无新派发向左退场',
          },
          {
            pr: null,
            commit: 'e74c39c1',
            summary:
              'HUD 叠加窗口真透明（建窗关闭窗口阴影 + WebView2 底色置全透明，圆角与透明首次真正生效）',
          },
          {
            pr: null,
            commit: '7f9e975f',
            summary:
              'HUD 交互与显示修复（告警措辞改安抚、忍耐态保持动作标题可见、仲裁显示不被 shell 标题覆盖）',
          },
          {
            pr: null,
            commit: '325dce84',
            summary:
              '系统预设主题下背景图不透明度默认 0（#117：残留路径不再凭空点亮，选了图仍可见）',
          },
        ],
      },
    ],
  },
  {
    version: '1.1.1',
    date: '2026-10-25',
    contributors: [
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 115,
            summary: 'ego lite 外部 CDP 浏览器识别（ego.exe 精确匹配 + 名称推断 + 双安装路径覆盖）',
          },
          {
            pr: 116,
            summary:
              '切换主题后背景图残留修复（预设卡无条件回退 + 异步解析序号守卫 + HUD 快捷切主题回退）',
          },
        ],
      },
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: 'd89e1f4',
            summary: 'refine 上下文压缩回归修复（accumulate_distill 恢复对话全清）',
          },
          {
            pr: null,
            commit: 'ce29afb',
            summary: '85% 硬止损恢复为最后兜底 + 提炼轮代码级禁工具 + 上下文超限自救',
          },
          {
            pr: null,
            commit: '68abc7b',
            summary: '提炼后重建 L1 提示词缓存，记忆索引与最新摘要同步',
          },
        ],
      },
    ],
  },
  {
    version: '1.1.0',
    date: '2026-10-24',
    contributors: [
      {
        user: 'Steooenwolf-666',
        contributions: [
          {
            pr: 110,
            summary:
              '启动自动检查新版本并提示用户点击更新（顶部提示条 + 手动更新页共用状态，任务执行期间延迟安装）',
          },
        ],
      },
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 112,
            summary:
              '外观滑块步进改 1% + 每个参数行内「恢复默认」（只回退未保存草稿，不碰已保存主题）',
          },
        ],
      },
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '5f5e993b',
            summary: 'HUD 更新按全局 seq 排序，live 计时不设上限',
          },
          {
            pr: null,
            commit: '2f36f3ca',
            summary: '单实例守卫：重复启动唤醒主窗，不再双进程/双托盘/门铃抢占',
          },
          {
            pr: null,
            commit: '198b91b6',
            summary: 'minimax/qwen 默认路由到官方 Anthropic 兼容端点',
          },
          {
            pr: null,
            commit: 'b8e65c86',
            summary:
              'HUD 优先级仲裁、聊天消息底部动作同行布局与代码块复制、执行中动效平滑化、WorkAgent 跨迭代换行',
          },
          {
            pr: null,
            commit: '7e457500',
            summary: '恢复默认后草稿等值基线即清空（消除假未保存态）；外观浮窗右让位 55px',
          },
          {
            pr: null,
            commit: '59014f0a',
            summary: '工具条 dock 移入 chat-area 栈叠上下文，不再压住右锚定浮层',
          },
        ],
      },
    ],
  },
  {
    version: '1.0.0',
    date: '2026-10-22',
    contributors: [
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: 'eaa2c8c8',
            summary:
              '拆除界面判死架构：心跳判死 / 看门狗恢复阶梯 / eval 探针全部移除，保留日志限流与主线程停摆日志',
          },
          {
            pr: null,
            commit: '7af7a6cb',
            summary: 'ReAct 循环轮中增量快照：执行中崩溃最多丢当前迭代增量，不再吞整个对话',
          },
          {
            pr: null,
            commit: 'ab5c73e6',
            summary:
              'system_shell 超时默认保留进程（杀仅限显式 background:false），长命令不再腰斩重跑',
          },
          {
            pr: null,
            commit: '8b100bc2',
            summary:
              '消息元数据条常驻：动作按钮执行完成才显示，耗时 / 步数实时可见 + 执行中旋转指示',
          },
          {
            pr: null,
            commit: '6c042c9e',
            summary: '后台任务入口执行光影与 hover 提示，标题补耗时小字',
          },
          {
            pr: null,
            commit: '03f9209b',
            summary: '执行中耐心提示（验证/构建/测试中）超过 60s 显示、完成即消（渐变流光）',
          },
        ],
      },
    ],
  },
  {
    version: '0.3.1',
    date: '2026-10-21',
    contributors: [
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '9835fe2e',
            summary: '预览 URL 保持目录边界',
          },
          {
            pr: null,
            commit: '39e0e12b',
            summary: '分项目原则：通用原则全局适用，项目原则按当前项目路径过滤，异项目不泄露',
          },
          {
            pr: null,
            commit: 'a7828be3',
            summary:
              '执行中上下文预算防溢出（分档强制线 75%/80%，大窗口默认 50% 可调 50%~80%）+ refine 落地为整体替换（历史全弃、摘要成唯一记忆）',
          },
          {
            pr: null,
            commit: 'bf82e1a8',
            summary: 'Refine 折叠区（小巧占位点击展开）+ turn-rail refine 分界线过滤',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.31',
    date: '2026-10-19',
    contributors: [
      {
        user: 'Steooenwolf-666',
        contributions: [
          {
            pr: 107,
            summary:
              'ChatGPT 套餐登录与 Responses 推理：通用 OAuth、凭据安全存储、逐模型协议路由与请求约束',
          },
          {
            pr: 108,
            summary: '修复执行点评、内联配置与新会话的 IPC 参数（顶层统一 camelCase）',
          },
        ],
      },
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '44bbaf62',
            summary:
              'StepFun 接入：API Key 与 Step Plan 双通道，step-5-preview / 3.7-flash / 3.5-flash',
          },
          {
            pr: null,
            commit: '9306f969',
            summary: '更新检查优先走 dl.nuphus.com，自建线路国内外可达并保留三级兜底',
          },
          {
            pr: null,
            commit: 'b17927b1',
            summary: '更新检查 10s / 下载 600s 兜底超时，连续 10s 无进度给出停滞提示',
          },
          {
            pr: null,
            commit: '3124171e',
            summary: '退出持久化转后台，8s 看门狗兜底；判死前用 eval 探针确认渲染进程存活',
          },
          {
            pr: null,
            commit: '956ab3ae',
            summary: '日志折叠计数按 sink 隔离，文件与 stderr 不再互相抢首条',
          },
          {
            pr: null,
            commit: '28e7af16',
            summary: '图片预览 portal 到 body 层级置顶，浏览器按钮打开官网，走秒保持可见',
          },
          {
            pr: null,
            commit: 'f1cc4760',
            summary: '心跳回归主线程节奏，撤销 Worker 探针（invoke 依赖 window）',
          },
          {
            pr: null,
            commit: '033f5731',
            summary: '视觉模型与 provider 成对解析，修复同名模型跨段盲选导致的 401',
          },
          {
            pr: null,
            commit: '35f7ebea',
            summary: '工具栏取消固定后恢复自动隐藏计时',
          },
          {
            pr: null,
            commit: '76f4ca98',
            summary:
              '图标链补全：1024 PNG / 32px ICO 首层 / 11 块 ICNS，安装后刷新资源管理器图标缓存',
          },
          {
            pr: null,
            commit: '1b7dd373',
            summary: '低内存机器 release 构建兜底：16MiB 栈、opt-level 2 与 codegen-units 32',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.30',
    date: '2026-10-17',
    contributors: [
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '5c545265',
            summary: '网页搜索源可切换官方 Search / Fetch，失败或限流时降级到内置免费源',
          },
          {
            pr: null,
            commit: 'da653a0c',
            summary: '模型设置新增独立「网页搜索源」页，按服务商段绑定',
          },
          {
            pr: null,
            commit: 'ef920435',
            summary: 'system_shell 超过 300s 自动转后台，进程树与输出保留',
          },
          {
            pr: null,
            commit: '1ca4d7ea',
            summary: '框选 / 取色增加放大镜与方向键微调（Shift 十倍速）',
          },
          {
            pr: null,
            commit: '3c1bec6d',
            summary: '取色工具可直接从屏幕取字体颜色',
          },
          {
            pr: null,
            commit: 'bb2b08db',
            summary: '桌面工具条全局 Ctrl+1/2/3 热键',
          },
          {
            pr: null,
            commit: '2fa28b8a',
            summary: 'MCP 读取机器本地 servers.local.yaml 并合并仓库配置',
          },
          {
            pr: null,
            commit: 'fcc11f8b',
            summary: '视觉工具改用内存帧并新增 wait_for',
          },
          {
            pr: null,
            commit: 'f90df0b1',
            summary: '执行追踪展开思考支持视口锚定折叠',
          },
          {
            pr: null,
            commit: '5560ccc2',
            summary: '定时调度放开界面类步骤，改按「是否会停下来等人」判定',
          },
          {
            pr: null,
            commit: 'e57a0216',
            summary: '所有 Tauri 命令移出主线程，数据库连接池不再阻塞',
          },
          {
            pr: null,
            commit: '99480720',
            summary: '主 webview 死亡自愈：自动重载 + 原生恢复对话框',
          },
          {
            pr: null,
            commit: '6508fb39',
            summary: '发布资产下载自动选择最快镜像节点',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.29',
    date: '2026-10-15',
    contributors: [
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 105,
            summary:
              '修复会话行 hover 动画标签重叠、按钮误触，并恢复 ModelsPage 测试基线（补 mock 缺失的 getVisionResolution）',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.28',
    date: '2026-10-12',
    contributors: [
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 104,
            summary: '热门模型快捷 chip + 筛选框多关键词（空格 OR）',
          },
        ],
      },
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: 'e7c14c26',
            summary: '流式错误体截断改字符边界安全，修复中文错误响应打死 IPC 的 panic',
          },
          {
            pr: null,
            commit: '68c7f424',
            summary: 'webview 假死可检测、可降级、可恢复——界面失联不再静默刷屏',
          },
          {
            pr: null,
            commit: 'bc4d7a32',
            summary: '死循环守卫分级：3 次重复预警不拦截，5 次失败终止并说明原因',
          },
          {
            pr: null,
            commit: '2e3bc511',
            summary:
              '后台任务生命周期闭环：超时按 background 显式声明分流，看门狗强杀超期，退出必清算',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.27',
    date: '2026-10-10',
    contributors: [
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '592fdef3',
            summary: '简白主题按浅色手法重构，清除深色语义残留',
          },
          {
            pr: null,
            commit: '93656dbc',
            summary: 'HTML 预览标注闭环：框选批注回流为结构化引用，按引用定向修改页面',
          },
          {
            pr: null,
            commit: '8bf6d758',
            summary: '应用图标反色为白底黑体，修正 ICO 帧行序',
          },
          {
            pr: null,
            commit: '0241e52b',
            summary: '修 CDP 浏览器入口唤不起：runtime 死锁 + 连接死亡后不自愈',
          },
          {
            pr: null,
            commit: '919dc5c1',
            summary: '执行追踪默认只渲染最近 40 条，上百步不再掉帧',
          },
          {
            pr: null,
            commit: 'ea3c23a8',
            summary: 'web_extract 补齐 SSRF 守卫，收紧为公网抓取策略',
          },
          {
            pr: null,
            commit: '2ba36240',
            summary: '后台任务账本与中断汇报闭环：跨调用子进程可查可结束，中断如实结算汇报',
          },
          {
            pr: null,
            commit: '78854529',
            summary: '外链统一进 Agent 浏览器，冷启动预检提速（6s → 0.9s）',
          },
          {
            pr: null,
            commit: '28e7d28d',
            summary: '桌面工具条改右侧常驻按钮列，撤 Ctrl+U 与扳手菜单',
          },
          {
            pr: null,
            commit: '390ae68f',
            summary: '输入法组词态守卫，Enter 选词不再误发',
          },
          {
            pr: null,
            commit: '53ef4098',
            summary: '执行追踪按步折叠与滚动锚点补偿，折叠 / 下拉 / 贴底不再打架',
          },
          {
            pr: null,
            commit: 'cd416661',
            summary: '增强模式改只读状态行，迁入推理强度弹窗',
          },
          {
            pr: null,
            commit: 'e0d3f368',
            summary: '贴底跟随改手势判定，内容增长永不抢控制',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.26',
    date: '2026-10-05',
    contributors: [
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '0675b9e8',
            summary: 'npm 发布与 git hooks 脚本补 UTF-8 BOM：PS 5.1 按 GBK 误读吞字符致解析失败',
          },
          {
            pr: null,
            commit: 'd099408f',
            summary: 'Gitee 同步脚本 native 调用统一降 EAP：git stderr 通知不再炸流水线误判失败',
          },
          {
            pr: null,
            commit: '0b51ce50',
            summary: '新增 fetch-release-asset.ps1：发布资产预下载与 sha256 对账固化',
          },
          {
            pr: null,
            commit: '0b1abc05',
            summary: '截图管线改 PNG 单次写入、OCR 引擎进程级共享、dev profile 调优',
          },
          {
            pr: null,
            commit: 'c7054b53',
            summary:
              '原生遮罩截图链路接入桌面工具栏五模式：冻结帧 + hover 跟窗框选 + 8 节点 + 单点拾取',
          },
          {
            pr: null,
            commit: '20cb116e',
            summary: '用户资源内容寻址去重：同字节文件不再重复落盘',
          },
          {
            pr: null,
            commit: 'a8824397',
            summary: 'OCR 工具改名「字典」，登记应用按钮补图标与文字',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.25',
    date: '2026-10-02',
    contributors: [
      {
        user: 'mrpulor-gh',
        contributions: [
          {
            pr: null,
            commit: '019859cb',
            summary:
              'Gitee 国内下载点落地：更新入口扩为 GitHub / 镜像 / Gitee raw 三路，任一失败自动回落',
          },
          {
            pr: null,
            commit: 'cc545ca1',
            summary: '模型身份二元组化：以「服务商段 + 模型 id」成对解析绑定，同 id 跨段不再串台',
          },
          {
            pr: null,
            commit: '77aad176',
            summary:
              'refine 按上下文窗口分档重构：大窗口强制线 50%–80% 可调（默认 50%），提示线与强制线分离',
          },
          {
            pr: null,
            commit: 'df0937fc',
            summary:
              'refine 全链路审计修复：越线后真实触发提炼、切换模型作废过期提示、失败重试兜底',
          },
          {
            pr: null,
            commit: 'dc94f6b9',
            summary:
              '轮次元数据改后端单一来源：耗时/步数从执行启动起算、刷新不归零，上下文占用取 API 官方口径',
          },
          {
            pr: null,
            commit: '1f5c823d',
            summary: '会话台按项目限容：镜像改为每项目 10 条，不再挤掉其它项目的会话记录',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.24',
    date: '2026-09-30',
    contributors: [
      {
        user: 'yuansui486',
        contributions: [
          {
            pr: 87,
            summary: '修复外部数据点的 Markdown 渲染（MarkdownInline 导出供复用）',
          },
          {
            pr: 88,
            summary: '明确模型连接成功反馈，并把模型能力探测移到后台不再阻塞切换',
          },
        ],
      },
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 84,
            summary: '补 mobile server / relay 威胁模型与已接受限制，写入 SECURITY.md',
          },
        ],
      },
      {
        user: 'Steooenwolf-666',
        contributions: [
          {
            pr: 91,
            summary: '外部 Agent 投递序列正确传递桌面输入与提交快捷键的失败',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.23',
    date: '2026-09-28',
    contributors: [
      {
        user: 'Steooenwolf-666',
        contributions: [
          { pr: 67, summary: '保留重新编辑时每月 29–31 日的定时规则' },
          { pr: 70, summary: '手机端图片解码失败时显示原始路径' },
          { pr: 71, summary: '修正移动画布步骤时的循环变量作用域判断' },
          { pr: 72, summary: '稳定画布草稿与会话快照测试' },
          { pr: 73, summary: '保护外部 Agent 任务状态并支持最近上报报告预览' },
          { pr: 74, summary: '工作流新增非敏感运行参数方案' },
          { pr: 75, summary: '手机端支持失败图片就地重新加载' },
        ],
      },
      {
        user: 'fouyzjl',
        contributions: [
          { pr: 79, summary: 'switch_model 持久化 base_url，修复"改了调用地址又回退"' },
        ],
      },
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 80,
            summary: '会话工作台启动只展开上次对话所在文件夹，并修好子菜单被抽屉裁切',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.22',
    date: '2026-09-26',
    contributors: [
      {
        user: 'yuansui486',
        contributions: [
          {
            pr: 65,
            summary: '完善工作流节点调试、运行证据与画布编辑体验',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.21',
    date: '2026-09-25',
    contributors: [
      {
        user: 'yuansui486',
        contributions: [
          {
            pr: 58,
            summary: '完善跨平台桌面执行可靠性与工作流进度沟通',
          },
          {
            pr: 59,
            summary: '修复画布工作台窗口拖动并增加原型明暗切换',
          },
          {
            pr: 61,
            summary: '优化工作流节点表单、变量选择与画布布局',
          },
        ],
      },
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 63,
            summary: '增强判断模型 API Key 行增加 TypeSafe 控制台外链',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.20',
    date: '2026-09-24',
    contributors: [
      {
        user: 'yuansui486',
        contributions: [
          {
            pr: 56,
            summary: '完善跨平台目标绑定、语义观察与稳定回放',
          },
        ],
      },
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          {
            pr: 57,
            summary: '手机端可查看电脑本地图片：新增 /file 端点与内联渲染',
          },
        ],
      },
    ],
  },
  {
    version: '0.2.19',
    date: '2026-09-23',
    contributors: [
      {
        user: 'yuansui486',
        contributions: [
          {
            pr: 52,
            summary: 'UIA/Accessibility 语义桌面自动化基础层：语义观察、有限候选与执行前后复核',
          },
          { pr: 54, summary: '增强判断模型：独立配置 + 从本地有限候选中做结构化选择' },
        ],
      },
      {
        user: 'zhoupeiyu515-ui',
        contributions: [{ pr: 50, summary: '日志轮转加固：改名失败不再丢历史' }],
      },
    ],
  },
  {
    version: '0.2.18',
    date: '2026-09-22',
    contributors: [
      {
        user: 'zhoupeiyu515-ui',
        contributions: [
          { pr: 43, summary: '会话台「当前」标记归位到会话行，去掉新建对话行加号，弹窗标题可留空' },
          { pr: 44, summary: '创建项目弹窗：在目标文件夹下生成草稿对话，移除项目中心' },
          { pr: 45, summary: '历史会话按项目标签精确匹配，回填归属路径' },
          { pr: 46, summary: 'Ctrl/Cmd+Enter 改为换行，发送只保留 Enter' },
          { pr: 47, summary: '主窗口与启动窗口四角改用系统圆角' },
          { pr: 48, summary: '菜单 / 抽屉「外点关闭」改捕获阶段 pointerdown，白名单收窄' },
        ],
      },
      {
        user: 'yuansui486',
        contributions: [
          { pr: 40, summary: '修复未初始化工作流会话的当前状态显示' },
          { pr: 42, summary: '工作流外部输入、定时任务与运行回放' },
        ],
      },
    ],
  },
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
    ],
  },
]

/**
 * 仓库全部贡献者（GitHub `/contributors` API 快照，2026-10-19 拉取，按提交数降序）。
 *
 * 与 CONTRIBUTOR_ROUNDS 互补，二者取并集才是完整的「历史贡献者」：
 *   - 轮次表按 **PR 归属**记录（覆盖 GitHub 未把 commit 关联到账号的人）；
 *   - 本表按 **commit 作者**记录（覆盖没有 PR 记录的提交，如 fouyzjl 的 13 次提交）。
 *
 * 两表可能重复（同一人既有 PR 也有直推 commit），头像墙去重由 ALL_CONTRIBUTORS 负责。
 *
 * ⛔ 更新本表必须重跑（禁止凭印象增删）：
 *   curl -H 'Accept: application/vnd.github+json' \
 *     'https://api.github.com/repos/mrpulor-gh/nuphus/contributors?per_page=100'
 */
export const REPO_COMMITTERS: string[] = [
  'yuansui486', // 78 commits
  'Steooenwolf-666', // 24 commits（2026-10-19 API）
  'fouyzjl', // 13 commits
  'mrpulor-gh', // 4 commits（2026-10-19 API；仓库所有者）
  'jiangdingwei123-afk', // 1 commit（2026-10-19 API，main 上的 80a2e053）
  'zhoupeiyu515-ui', // 1 commit
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
