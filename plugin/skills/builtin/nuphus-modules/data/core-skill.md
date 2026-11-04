# 检索式技能系统（src/skill/）

> 定位：以「注册表只登记索引、正文按需检索」为设计核心，用文件系统上的 `plugin/skills/{builtin,community}/` 知识包 + 一个无状态 `SkillRegistry`，把可插拔领域知识变成 LLM 可按 `skill_read`/`skill_query` 随时取用、却不常驻 prompt 的外部记忆。
> 规模：3 文件 971 行。

## 结构

| 文件 | 职责 |
|---|---|
| `types.rs`（108） | 全部 serde 数据结构：SkillManifest / SkillTriggers / DataSource / SkillEntry / SkillQueryInput / SkillQueryHit / SkillQueryOutput |
| `registry.rs`（852） | 注册表全部行为 + 11 个内联测试：plugin_skills_dir / list / get / search / query / get_manifest / get_skill_md / install_from_path / install_from_git / remove / synthesize_manifest / validate_name |
| `mod.rs`（11） | 门面：模块声明 + 再导出 |

配套闭环（不属于本模块）：`src/tools/definitions/skill.rs`（skill_query/skill_read 工具面）、`src-tauri/src/commands/skill.rs`（skill_install/remove/list/install_git IPC 桥）、`src/agent/prompt.rs` 的 `skill_registry_section()`（生成注入 L1 的技能索引段）、`plugin/skills/README.md`（规范文档）。

## 功能作用

- **提示词只放索引**：`- name DisplayName — description` 一行；正文（SKILL.md + data/*）在磁盘上按需取回——system prompt 是最贵资源，全部塞入既不现实也稀释注意力。
- **检索式查询 `query()`**：tokenize（经 `segmenter::segment` 中文分词）+ 关键词匹配，扫 SKILL.md 与 data/ 下文件，按 relevance 降序返回命中全文。
- **双来源 + builtin 优先**：所有读取路径 builtin → community 兜底。
- **安装即验收**：安装成功≠交付成功，装完必须用消费端路径读回一次，读不到就回滚；Git 安装的临时目录用 pid + 纳秒 + 原子序号三重唯一防并发碰撞。

## 规则与坑

- **无状态注册表**：unit struct，所有方法每次实时 `read_dir` 扫盘，没有内存索引/缓存/数据库表——代价是每次调用付 IO，收益是「装完即生效、删完即消失」，无需失效逻辑。
- builtin/（git 追踪，不可删）与 community/（gitignore，可增删）双轨。
- 名称校验与清洗防路径穿越（`registry.rs:81-96`）；manifest 读取剥离 UTF-8 BOM。
