# 浏览器自动化（src/browser/ + crates/nuphus-browser）

> 定位：基于 chromiumoxide 的 Rust 原生 CDP 浏览器自动化核心——负责 Chrome/Edge 定位与启动、附着与自愈重连、页面交互与快照、Cookie 导入、下载管理，并托管 preview:// 标注 overlay 脚本。不依赖任何外部驱动（chromedriver / Playwright）。
> 规模：覆盖路径 10 个（1 shim + 9 crate 内），关联 45 个。

## 结构

| 文件 | 职责 |
|---|---|
| `src/browser/mod.rs`（15） | 主 crate 侧重导出 shim（对外 API 不变），Tauri 壳经它取脚本常量而无需直接依赖 browser crate |
| `crates/nuphus-browser/src/client.rs`（6111） | **核心**：BrowserClient 全量能力 + 启动/附着/自愈 + 单测与集成测试 |
| `crates/nuphus-browser/src/chrome_finder.rs`（207） | 跨平台定位 Chrome/Chromium/Edge，管理 Nuphus 自管 profile 目录 |
| `crates/nuphus-browser/src/shared.rs`（71） | 进程级常驻 tokio runtime + 进程级共享 BrowserClient 单例 + get-or-launch |
| `crates/nuphus-browser/src/cookie_source.rs`（51） | 可插拔 Cookie 数据源（host 注册 loader，模块不关心来源） |
| `crates/nuphus-browser/src/annotation_overlay.rs`（770） | 标注 overlay JS 脚本常量（**生成物**，真源在前端） |
| `crates/nuphus-browser/src/helpers.js`（226） | `window.__nuphus` 批量执行 helper（include_str! 注入页面） |
| `third_party/chromiumoxide/`（43 文件） | chromiumoxide 0.9.1 本地 patch：on-demand Runtime 执行模式 |
| `frontend/tools/overlay-script.src.js` + `sync-overlay-script.mjs` | overlay 脚本真源 + 搬运/防漂移校验脚本 |

架构一句话：**shared.rs 管「进程级资源」，client.rs 管「一个浏览器实例的生命周期与全部 CDP 能力」，chrome_finder/cookie_source 是被注入的外部能力，annotation_overlay 是被引出的脚本资产**。

## 功能作用

- **把 Chrome/Edge 变成可编程的 Agent 之手**：导航、点击、输入、按键、滚动、截图、提取、批量执行，全部走 CDP。
- **把「浏览器连接」当易碎资源治理**：连接会死（Chrome 被用户关掉/崩溃/指纹浏览器窗口重开换端口），内建「证明式判死 + 自愈重连」（ConnectionLiveness / heal_external_endpoint / reconnect）。
- **把实例选择权交给用户**：既支持 Nuphus 自管 Chrome（固定 profile，登录态持久），也支持附着用户配置的指纹浏览器（AdsPower 等，经 --remote-debugging-port 暴露 CDP，窗口重开后按 exe 路径自动重新解析端口）。

## 规则与坑

- 底层依赖经仓库内 `[patch.crates-io]` 指向 `third_party/chromiumoxide`（chromiumoxide 0.9.1 本地修订版），目的是关掉 `Runtime.enable` 的 CDP 可检测性。
- overlay 脚本真源在前端，Rust 常量是生成物——有防漂移校验脚本（sync-overlay-script.mjs 双重比对）。
- 跨 crate 单一真源：原 `src/browser` 内联模块整体抽离为独立 crate，供 nuphus 主 crate 与 nuphus-mcp 共用。
