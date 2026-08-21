# Skills 技能包

> 检索式知识包，每个 Skill 是一个结构化的领域知识单元（`skill.json` + `SKILL.md` + `data/`）。
> 通过 `skill_query`/`skill_read` 工具按需检索，不注入 LLM prompt 以节省 token。属于 plugin/ 插件扩展体系的一部分。

## 目录结构

```
plugin/skills/
├── <skill-name>/
│   ├── skill.json       ← 元信息清单（名称/版本/关键词/触发条件/数据源/依赖工具）
│   ├── SKILL.md         ← 核心知识文档（YAML frontmatter + Markdown body）
│   └── data/            ← 附加数据文件（可选）
│
├── agent-orchestration/  ← Agent 平台编排
├── ui-design/            ← UI 设计系统
└── workflow-design/      ← 工作流设计规范
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

## 已注册技能

| 技能 ID | 名称 | 说明 | 路径 |
|---------|------|------|------|
| `agent-orchestration` | Agent 平台编排 | 外部 Agent 平台的登记（team.toml）、启动、交互与并行调度策略 | `agent-orchestration/` |
| `ui-design` | UI 设计系统 | Nuphus UI 设计规范与组件模式 | `ui-design/` |
| `workflow-design` | 工作流设计规范 | 工作流设计模式与步骤编排指南 | `workflow-design/` |