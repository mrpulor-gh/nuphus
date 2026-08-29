---
title: MCP 工具参考
id: mcp-tools
type: skill
tags: [mcp, 工具, 集成, server, 工作流]
---

# MCP 工具参考

> 通过 `skill_read mcp-tools` 加载本文档。WorkAgent 设计工作流时按需查阅。

## 原理

Nuphus 通过 MCP (Model Context Protocol) 连接外部工具。每个 MCP server 暴露一组 `tools`，可在工作流中用 `do.mcp` 步骤调用（V2 格式，与内置工具同级别）：

```json
{ "id": "search", "name": "搜问题",
  "do": { "mcp": { "server": "github", "tool": "search_issues",
    "with": { "query": "repo:nuphus bug", "limit": 5 } } },
  "capture": "issues" }
```

- `server`：`plugin/mcp/servers.yaml` 中的 key
- `tool`：MCP 工具名
- `with`：工具参数（支持 `{{var}}` 模板）
- `capture`：**字符串**，输出存入变量（V2 规范，无 `as_type` 对象格式）

## 工具发现

servers.yaml 配置了新 server 但不确定其工具签名时，运行时自省：

```json
{ "id": "list", "name": "列工具",
  "do": { "mcp": { "server": "github", "tool": "tools/list", "with": {} } },
  "capture": "tools" }
```

`{{tools}}` 含完整工具名与 inputSchema。

## 已配置的 Server

以下列表来自 `plugin/mcp/servers.yaml`。配置变更后需同步更新本文档。

> 暂无已配置的 MCP server。在 `plugin/mcp/servers.yaml` 配置后更新本段。

## 常用 Server 速查

| Server | 环境变量 | 常用工具 |
|--------|---------|---------|
| GitHub `@modelcontextprotocol/server-github` | `GITHUB_PERSONAL_ACCESS_TOKEN` | `search_repositories` / `search_issues` / `create_issue` / `get_file_contents` |
| Postgres `@modelcontextprotocol/server-postgres` | `DATABASE_URL` | `query`（执行 SQL） |
| Slack `@modelcontextprotocol/server-slack` | `SLACK_BOT_TOKEN` | `send_message` / `list_channels` / `get_channel_history` |
| Filesystem `@modelcontextprotocol/server-filesystem` | 参数 `--directory <允许目录>` | `read_file` / `write_file` / `list_directory` / `search_files` |

## 设计原则

- **MCP 优先于 GUI**：目标软件有 MCP server 时优先走 `do.mcp`，更快更稳且不依赖布局解析
- **MCP 优先于 system_shell**：MCP 返回结构化 JSON，比 CLI 文本解析可靠
- **降级路径**：MCP server 不可用时降级到 system_shell 调用对应 CLI
- **MCP 调用超时/失败**：用 `on_error`（retry / allow_codes）控制，勿用死循环重试
