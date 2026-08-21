# Agent 问答台（样板插件）

Nuphus App Plugin（`.nuph`）P1 样板：经 Bridge `agent.chat` 调 Agent 问答的完整链路
（插件 → 桥接器鉴权 → `plugin_agent_chat` command → `submit_user_message` 共享入口，
`source="plugin:{id}"` 可溯）。

## 功能

- 极简聊天窗：消息列表 + 输入框 + 发送按钮（Enter 发送 / Shift+Enter 换行）
- 气泡背景使用语义 token `--msg-user-bg` / `--msg-assistant-bg`，
  随主窗口主题切换与「气泡不透明度」滑块实时变化（tokens.css + theme.css 双引用）
- 等待回复期间按钮显示「思考中…」并禁用输入（v1 同步等待，宿主侧 120s 超时）
- 对话历史经 `kv.*` 持久化：key=`history`，仅保留最近 50 条，
  重新打开宿主后自动恢复

## 权限

`permissions: ["agent.chat", "kv", "theme.get"]`——三者均在 v1 权限枚举内
（`theme.get` 同时为桥接器只读免校验方法，声明只为显式契约）。

## 打包为 .nuph

`.nuph` = ZIP 包，**zip 根目录直接包含 `manifest.json` 与 `index.html`**（不可套子目录）。

在 `examples/agent-chat/` 目录下执行：

```powershell
cd C:\Users\Administrator\Nuphus\examples\agent-chat
Compress-Archive -Path manifest.json, index.html -DestinationPath agent-chat.nuph -Force
```

> 若后端已支持 ZIP 扩展名，`.zip` 亦可直接安装（安装器按 ZIP 解析）。

## 安装与验证

1. 启动 Nuphus（插件伺服复用移动端 server，宿主会自动拉起）。
2. 主窗口按 `Ctrl+K` → 输入「插件」→ 打开插件页。
3. 点击「安装插件」选择 `agent-chat.nuph`，或把文件拖入插件页。
4. 列表出现 **Agent 问答台** → 点击条目打开全屏宿主。

宿主页内验证：

- **agent.chat 贯通**：输入问题发送 → 等待后出现 Agent 最终回复（独立运行时执行，
  与桌面/手机会话完全隔离——不占主会话 busy、事件不进主窗口/手机；上下文由插件自管 history 注入）。
- **等待中 loading**：发送后按钮变「思考中…」且输入禁用，回复到达后恢复。
- **超时**：Agent 执行超过 120s → 气泡显示 `[TIMEOUT] 对话超时（120 秒）`。
- **串行**：上一条未完成时再点发送，按钮已被禁用（桥接器侧 `BUSY` 为纵深防线）。
- **kv 持久化**：关闭宿主重新打开 → 历史（最近 50 条）恢复；数据落盘
  `plugin/apps/com.nuphus.agent-chat/kv.json`。
- **主题跟随**：主窗口切换主题 / 拖动气泡不透明度滑块 → 气泡颜色实时变化。

## manifest 字段说明

`manifest.json` 中 `"sample": true` 标记本插件为官方示例（插件页渲染「示例」徽章，普通用户可安全卸载）；该字段为 Schema v1 可选扩展，缺省 `false`，不影响旧包解析。

## 目录结构

```
examples/agent-chat/
├─ manifest.json   # 契约 §3 Schema v1（id=com.nuphus.agent-chat，声明 icon: icon.svg）
├─ index.html      # 入口（引 /plugins-shared/tokens.css|theme.css|bridge.js）
├─ icon.svg        # 插件图标（manifest 引用；打包命令仅含 manifest+index，icon 由伺服层从插件目录提供）
└─ README.md
```