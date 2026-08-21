# Hello Nuphus 样板插件

Nuphus App Plugin（`.nuph`）最小示例：主题色卡 + KV 计数器 + Toast + `theme.changed` 事件响应。

## 打包为 .nuph

`.nuph` = ZIP 包，**zip 根目录直接包含 `manifest.json` 与 `index.html`**（不可套子目录）。

在 `examples/hello-plugin/` 目录下执行：

```powershell
cd C:\Users\Administrator\Nuphus\examples\hello-plugin
Compress-Archive -Path manifest.json, index.html -DestinationPath hello-plugin.nuph -Force
```

> 若后端已支持 ZIP 扩展名，`.zip` 亦可直接安装（安装器按 ZIP 解析）。

## 安装与验证

1. 启动 Nuphus（应用启动后移动端 server 默认关闭——插件伺服复用移动端 server，宿主会自动尝试拉起）。
2. 主窗口按 `Ctrl+K` → 输入「插件」→ 打开插件页（或标题栏菜单）。
3. 点击「安装插件」选择 `hello-plugin.nuph`，或把文件拖入插件页。
4. 列表出现 **Hello Nuphus** → 点击条目打开全屏宿主。

宿主页内应看到：
- 四块主题色卡（surface-0 / surface-1 / accent / fg-1）随主窗口主题实时变化；
- KV 计数器：点「+1」计数持久化（`plugin/apps/com.nuphus.hello/kv.json`），重新打开宿主后计数保持；
- 「发 Toast」：主窗口 HUD 出现「来自插件：Hello Nuphus！」；
- 「主题: …」指示：在主窗口切换主题时实时更新（`theme.changed` 事件）。

## 权限说明

`manifest.json` 声明 `permissions: ["kv", "notify", "theme.get"]`——三者均在 Rust 安装器 v1 权限枚举 `PERMISSIONS_ALLOWED`（`kv / notify / theme.get / agent.chat / workflow.run`，plugin_apps.rs:62）内，安装即通过校验。

桥接器端 `theme.get` 为只读主题查询，宿主直接放行。

## manifest 字段说明

`manifest.json` 中 `"sample": true` 标记本插件为官方示例（插件页渲染「示例」徽章，普通用户可安全卸载）；该字段为 Schema v1 可选扩展，缺省 `false`，不影响旧包解析。

## 目录结构

```
examples/hello-plugin/
├─ manifest.json   # 契约 §3 Schema v1
├─ index.html      # 入口（引 /plugins-shared/tokens.css|theme.css|bridge.js）
└─ README.md
```