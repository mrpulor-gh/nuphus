# Nuphus Shell Hooks

> Hermes 风格生命周期钩子，允许外部脚本在 Agent 关键节点自动调用。无需编写 Rust 插件。

## 钩子类型（4 种）

| 钩子 | 触发时机 | 模式 | 功能 |
|------|---------|------|------|
| `pre_tool_call` | 工具执行前 | **同步**（可 Veto） | 危险命令过滤 + 安全检查。返回 0 = 允许，非 0 = 阻止执行 |
| `post_tool_call` | 工具执行后 | 异步 | 执行结果日志 + 失败通知 |
| `on_session_start` | 会话开始 | 异步 | 创建会话日志 + 更新活跃索引 |
| `on_session_end` | 会话结束 | 异步 | 写入会话日志 + 标记结束 |

## 脚本参数

| 钩子 | 脚本接收参数 |
|------|-------------|
| `pre_tool_call` | `tool_name` `json_params` |
| `post_tool_call` | `tool_name` `json_params` `json_result` |
| `on_session_start` | `session_id` `user_input` |
| `on_session_end` | `session_id` `success` `output` |

## 配置（hooks/hooks.yaml）

```yaml
hooks:
  pre_tool_call: hooks/pre_tool_call.sh
  post_tool_call: hooks/post_tool_call.sh
  on_session_start: hooks/session_start.sh
  on_session_end: hooks/session_end.sh
```

所有路径相对于 Nuphus 项目根目录。每个钩子提供一个 `.ps1`（Windows）和一个 `.sh`（Unix）版本。

## 文件清单

| 文件 | 说明 |
|------|------|
| `hooks.yaml` | 钩子配置文件 |
| `pre_tool_call.ps1` / `.sh` | 工具执行前钩子（危险命令黑名单过滤） |
| `post_tool_call.ps1` / `.sh` | 工具执行后钩子（成功/失败日志 + 失败通知生成） |
| `session_start.ps1` / `.sh` | 会话开始钩子（会话日志初始化） |
| `session_end.ps1` / `.sh` | 会话结束钩子（会话日志归档） |

## pre_tool_call 安全检查

`pre_tool_call.ps1` 会 Veto 以下危险操作：

- 黑名单工具：`delete_file`、`rm`（直接阻止）
- 危险命令模式：`Format-Table`、`Format-List`、`ConvertTo-Json`、`rm -rf`、`$env:`、`Set-ExecutionPolicy`、`Stop-Computer` 等

## 代码集成（src/hooks/mod.rs）

```rust
use nuphus::hooks::{HookConfig, HookRunner};

// 构造配置（路径相对于项目根；每个钩子提供 .ps1 / .sh 双版本，按平台选择）
let config = HookConfig {
    pre_tool_call: Some("hooks/pre_tool_call.ps1".into()),
    post_tool_call: Some("hooks/post_tool_call.ps1".into()),
    on_session_start: Some("hooks/session_start.ps1".into()),
    on_session_end: Some("hooks/session_end.ps1".into()),
};
let runner = HookRunner::new(config);

// pre_tool_call — 返回 bool（true = 允许执行）
let allowed = runner.run_pre_tool_call("system_shell", &params);

// post_tool_call — 异步日志
runner.run_post_tool_call("system_shell", &params, &result);

// 会话事件
runner.run_session_start(&session_id, &input);
runner.run_session_end(&session_id, true, &output);
```

`HookRunner` 通过 `std::process::Command` 启动外部脚本，传递 JSON 序列化参数。每个钩子有独立超时（默认 30s）。

## 日志输出

日志位于 `logs/` 目录：

```
logs/
  pre_tool_call.log      # 每次工具调用的 pre 记录
  post_tool_call.log     # 每次工具调用的 post 记录
  sessions/              # 会话日志
    <session_id>.log     # 单个会话的完整生命周期
    active_sessions.txt  # 活跃会话索引
  notifications/         # 失败通知（自动生成）
```

## 自定义扩展

每个脚本接收固定参数，可根据 `ToolName` / `SessionId` 等添加自定义逻辑。新增钩子只需：
1. 编写 `.ps1` / `.sh` 脚本
2. 在 `hooks.yaml` 注册路径
3. 重启 Nuphus 生效