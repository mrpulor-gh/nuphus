---
title: MCP 工具参考
id: mcp-tools
type: skill
tags: [mcp, 工具, cli, 集成, server]
---

# MCP 工具参考

> 通过 `skill_read mcp-tools` 加载本文档。WorkAgent 设计工作流时按需查阅。

## 原理

Nuphus 通过 MCP (Model Context Protocol) 连接外部工具。每个 MCP server 暴露一组 `tools`——像内置工具一样，可以在工作流中用 `kind: "mcp"` 步骤调用：

```json
{ "kind": "mcp", "server": "github", "tool": "search_issues",
  "params": { "query": "repo:nuphus bug", "limit": 5 },
  "capture": { "as": "issues", "as_type": "json" } }
```

`server` 对应 `plugin/mcp/servers.yaml` 中的 key，`tool` 为 MCP 工具名，`capture.as_type: "json"` 自动解析响应。可选 `timeout_secs`、`on_error`（支持 abort/skip/retry/allow_codes）。

## 已配置的 Server

以下列表来自 `plugin/mcp/servers.yaml`。具体工具签名见各 server 小节。

> 暂无已配置的 MCP server。在 `plugin/mcp/servers.yaml` 中配置后需更新本文档「已配置的 Server」章节。

---

## 工具发现

如果 servers.yaml 中配置了新 server 但本文档未列出其工具：

1. 在工作流中调用 `tools/list` 自省：
```json
{ "kind": "mcp", "server": "github", "tool": "tools/list",
  "params": {}, "capture": { "as": "tools", "as_type": "json" } }
```
2. 检查 `{{tools}}` 变量获取完整的工具名和 inputSchema

---

## 常用 MCP Server 速查

### GitHub (`@modelcontextprotocol/server-github`)

环境变量：`GITHUB_PERSONAL_ACCESS_TOKEN`

| 工具 | 说明 |
|------|------|
| `search_repositories` | 搜索仓库 |
| `search_issues` | 搜索 Issues |
| `create_issue` | 创建 Issue |
| `create_pull_request` | 创建 PR |
| `get_file_contents` | 读取文件内容 |
| `create_or_update_file` | 创建或更新文件 |

### Postgres (`@modelcontextprotocol/server-postgres`)

环境变量：`DATABASE_URL`

| 工具 | 说明 |
|------|------|
| `query` | 执行 SQL 查询 |

### Slack (`@modelcontextprotocol/server-slack`)

环境变量：`SLACK_BOT_TOKEN`

| 工具 | 说明 |
|------|------|
| `send_message` | 发送消息 |
| `list_channels` | 列出频道 |
| `get_channel_history` | 获取频道历史 |

### Filesystem (`@modelcontextprotocol/server-filesystem`)

参数：`--directory /path/to/allowed/dir`

| 工具 | 说明 |
|------|------|
| `read_file` | 读取文件 |
| `write_file` | 写入文件 |
| `list_directory` | 列目录 |
| `search_files` | 搜索文件 |

## 设计原则

- **MCP 优先于 GUI**：目标软件有 MCP server 时优先走 `kind: "mcp"`，更快更稳且不依赖布局。
- **MCP 优先于 system_shell**：MCP 返回结构化 JSON，比 CLI 文本解析可靠。
- **降级路径**：MCP server 不可用时降级到 system_shell 调用对应 CLI。
