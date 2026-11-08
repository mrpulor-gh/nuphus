# Skills 技能包

> 检索式知识包，每个 Skill 是一个结构化的领域知识单元（`skill.json` + `SKILL.md` + `data/`）。
> 通过 `skill_query`/`skill_read` 工具按需检索，不注入 LLM prompt 以节省 token。属于 plugin/ 插件扩展体系的一部分。

## 目录结构

```
plugin/skills/
├── builtin/              ← 内置：git 跟踪，随版本发布（编译期内嵌 + 首启落盘到用户数据目录）
│   └── <skill-name>/
│       ├── skill.json    ← 元信息清单（名称/版本/关键词/触发条件/数据源/依赖工具）
│       ├── SKILL.md      ← 核心知识文档（YAML frontmatter + Markdown body）
│       └── data/         ← 附加数据文件（可选）
├── community/            ← 第三方/本地安装：gitignore，可增删，不随版本发布
└── README.md
```

## 规范

- 每个技能包一个子目录，包含 `skill.json`（元信息）+ `SKILL.md`（知识文档）+ 可选 `data/` 目录
- `SKILL.md` 必须包含 YAML frontmatter：
  ```yaml
  ---
  title: <技能名称>
  id: <唯一标识>
  type: skill
  tags: [标签1, 标签2]
  ---
  ```
- `skill.json` 声明 SkillManifest：name / version / displayName / description / keywords / triggers / data_sources
- `data/` 目录存放附加数据文件（Markdown / YAML / JSON），在 `skill.json` 的 `data_sources` 中声明
- 技能包通过 `SkillRegistry` 注册，Agent 通过 `skill_query`/`skill_read` 工具按需检索
- 关键词自动匹配：`skill_query` 按 SkillManifest 声明的 `keywords` 和 `context_hints` 自动匹配用户问题
- `builtin/` 与 `community/` 双来源，读取时 builtin 优先；安装落点只有 `community/`

## 已注册技能（builtin/，随版本发布）

| 技能 ID | 名称 | 说明 | 路径 |
|---------|------|------|------|
| `agent-orchestration` | Agent 平台编排 | 外部 Agent 平台（独立终端 TUI / 桌面 App / Web 等）的登记（team.toml）、启动、交互与并行调度策略 | `builtin/agent-orchestration/` |
| `mcp-tools` | MCP 工具参考 | MCP (Model Context Protocol) 集成参考：V2 步骤格式 do.mcp（server/tool/with）、capture 字符串规范、工具发现（tools/list 自省）、已配置 server 列表 | `builtin/mcp-tools/` |
| `nuphus-modules` | Nuphus 项目模块知识 | Nuphus 32 个模块（core 13 / tauri 8 / frontend 11）结构与作用的自包含摘要库，含项目骨架速答、全局坑速查、问题→摘要路由表 | `builtin/nuphus-modules/` |
| `tools-internal` | 内置工具手册 | Nuphus 工具页 23 个内部机制命令参数手册（PDF 7 / 图像 7 / 视频 5 / 音频 3 含语音克隆 / 文档 1） | `builtin/tools-internal/` |
| `ui-design` | 默认设计语言 | Nuphus 带出项目的默认设计语言：单色 chrome、wash 分层、大圆角、一个主行动。含可直接复制的 tokens.css 与基元 CSS、布局节奏、动效四层、文案口径、禁止清单与验收项；用户未指定风格且项目无既有设计系统时默认使用 | `builtin/ui-design/` |
| `workflow-design` | 工作流设计规范 | 工作流编排核心能力：V2 步骤 schema（14 种 Action）、变量模板与条件表达式、params.json 固化规范、外部输入（inputs 声明）、设计模式库、验证闭环方法论、ui-maps 经验闭环与常见陷阱 | `builtin/workflow-design/` |
| `workflow-interaction` | 工作流交互手册 | 探索与操作真实界面的交互细则：目标绑定、语义观察与分页、增强模式（desktop_agent_step）交接、输入与提交纪律、稳定 locator 固化、用户交互粒度、截图卫生 | `builtin/workflow-interaction/` |
| `workflow-recovery` | 工作流故障恢复 | workflow_run 失败后的恢复协议：阻塞类型识别、就地解决优先级（语义候选/增强判断/视觉回退）、同 id 断点续连、设计缺陷与运行时阻塞的判界、重试上限与汇报 | `builtin/workflow-recovery/` |

> `community/` 为本地/第三方技能（gitignore，不随版本发布），不入上表；当前本地装有 `git-pr-protocol`（Git/PR 协作规范）、`release-poster`（发版宣传图制作规范）。
