# 前端 · 设置页与业务页面（main-window/pages/ components/ hooks/）

> 定位：设置中心（SettingsCenter）以「左导航 + 右内容」居中弹窗组织全部 19 个设置/业务子页；**ModelsPage（4365 行）是全模块最重的单文件**，承担多 Provider 管理、自定义中转站实例（OAuth/自定义标头）、能力模型绑定与协议覆盖；更新机制由 useAppUpdate 统一持有 tauri-plugin-updater 状态，被更新提示条与更新页共用。
> 规模：53 文件（pages 41 + components 9 + hooks 1 + memories 1 + knowledge 1）。

## 结构

| 分组 | 文件 | 职责 |
|---|---|---|
| 设置中心外壳 | `SettingsCenter.tsx`（464） | 19 分区五组导航（按「用户目的」）、宿主分流、焦点陷阱 |
| 模型配置族 | `ModelsPage.tsx`（4365）/ `ModelCard.tsx`（226）/ `ModelProtocolSelect.tsx`（31）/ `ChatGPTSettings.tsx`（437）/ `RowCtxEditor.tsx`（101） | 模型配置整页（Provider 管理 + 自定义实例 + 能力绑定 + 协议覆盖）；模型卡片；协议覆盖下拉（chat-completions/responses/anthropic）；ChatGPT 套餐账号 OAuth；上下文窗口 badge↔行内编辑 |
| 系统设置族 | `GeneralPage`（268）/ `SecurityPage`（187，权限矩阵 + macOS 权限报告）/ `UpdatePage`（190）/ `hooks/useAppUpdate.ts`（239）/ `components/UpdateNotice.tsx`（61）/ `DataDirsPage`（120）/ `SessionGroupsPage`（106）/ `ShortcutsDialog`（84）+ `shortcuts.ts`（36，唯一事实源） | 语言/字号/关闭行为/HUD；工具三开关；版本与更新；数据目录只读展示；分组折叠上限；快捷键清单 |
| 资产与知识族 | `SkillsPage`（228）/ `McpPage`（157）/ `memories/MemoriesPage.tsx`（921）/ `knowledge/KnowledgePage.tsx`（231）/ `SoulPage`（206）/ `CustomAgentsPage`（474） | 技能包管理（目录/Git 安装、卸载）；MCP server 只读列表；记忆管理（概览/会话/经验三 tab）；知识库搜索/标签筛选；灵魂（头像 + 身份行）；自定义 Agent（l2Prompt/工具白名单） |
| 网络与外部族 | `MobilePage`（656，网络控制中心：密码/直连/P2P/中继/VPS）/ `BrowserPage`（354，managed/external CDP）/ `ExternalAgentsPage`（624，投递序列/窗口捕获）/ `components/AgentIconAuto.tsx`（190）/ `components/ApprovalModal.tsx`（202） | 移动端与中继配置；浏览器执行环境；外部 Agent 配置；图标单一实现；危险动作审批弹窗 |
| 插件与彩蛋 | `AppShellPage`（437，iframe 沙箱 + Bridge 桥接）/ `PluginAppsPage`（661）/ `PluginDevPage`（468）/ `components/PluginRestoreFab.tsx`（79）/ `GithubPage`（170）+ `githubContributors.ts`（1046）/ `SnakeGame/*`（466）/ `HelpPage`（161） | 应用插件全屏宿主；插件市场面板；开发者中心五步向导；贡献者页；贪吃蛇彩蛋（纯函数 game.ts + canvas 渲染） |
| 增强判断族 | `JevSettings.tsx`（566）/ `LayaBackendPanel.tsx`（478） | Jev 云端 / Laya 自托管双 tab 的增强判断模型设置 |

## 功能作用

- **设置入口统一组织**：历史上每个页面各自挂在 App.tsx 的 CompactModal 里，SettingsCenter 把它们收进一个居中大弹窗按五组导航，右侧内容区复用同一批页面组件（lazy 说明符与 App.tsx 各入口完全一致 → 命中同一 chunk，不重复打包）。
- **模型配置单一真相界面**：管「用哪个模型、用哪家服务商、密钥/地址/协议/能力」，落盘一律走后端命令，**磁盘是唯一真值，前端不做乐观增删**（ModelsPage.tsx:2026-2028）。
- **应用自更新共享状态**：启动延迟自动检查 + 用户点击才下载安装，顶部提示条与更新页共用同一份状态。

## 规则与坑

- 页面只经 `main-window/lib/api.ts` 的 invoke 包装与后端交互，不含业务逻辑。
- 各子页同时保留 App.tsx CompactModal 旧链路，两条入口共用同一组件同一 chunk（如 SkillsPage / SecurityPage）。
- 模型分区从设置中心分流到 `.models-page-host` 全屏宿主，画布分区分流到 `.canvas-workbench-host`。
