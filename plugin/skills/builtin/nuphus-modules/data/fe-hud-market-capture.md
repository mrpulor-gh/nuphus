# 前端 · HUD 浮窗/插件市场/截图覆盖层（hud/ market/ capture-overlay/）

> 定位：三个**独立于主窗口**的前端单元——HUD 常驻置顶进度浮窗、可运行时摘除的插件市场模块、全屏截图/取色覆盖层；三者分别由 Tauri 以「配置声明 / 主窗动态加载 / 按需创建」三种不同方式拉起，共同点是**都不承载业务逻辑，只做渲染与 IPC 转发**。
> 规模：10 文件（源 8 + CSS 2）。

## 结构

| 单元 | 文件 | 职责 |
|---|---|---|
| HUD 浮窗 | `hud/main.tsx`（9）/ `hud/App.tsx`（842）/ `hud/hud.css`（25） | 300×58 逻辑像素、无装饰、透明、always_on_top、skip_taskbar、不抢焦点的置顶小窗；相位色板（HudPhase/HudState）、图标解析、计时器（useLiveTimer）、双事件通道、暂停/继续/终止三按钮、相位自动隐藏 |
| 插件市场 | `market/index.ts`（12）/ `market/data.ts`（103）/ `market/MarketSection.tsx`（255）/ `market/market.css`（46） | 可删除目录设计：PluginAppsPage 用 `import.meta.glob('../../market/index.ts')` 运行时加载；市场索引拉取（GitHub Pages）、宽容解析、排序/过滤展示、安装动作转成唯一一条 Rust 命令 `plugin_market_install` |
| 截图覆盖层 | `capture-overlay/main.tsx`（52，含主题引导 IIFE）/ `CaptureOverlay.tsx`（710）/ `styles.css`（108） | 全屏不透明遮罩窗，承接「截图/选区/取色/鼠标坐标/ROI 证据/find_image 模板」六种框选模式；放大镜 RAF 绘制、遮罩、选区、确认条 |

## 功能作用

- **HUD = 执行进度的第二块屏幕**：主窗口被最小化/遮挡时用户仍能看到 Agent 在干什么；也承载工作流的暂停/继续/终止三个控制按钮。窗口由 tauri.conf.json 静态声明（label="hud"，visible=false），前端只负责把 Rust 推来的 {text, phase} 渲染成对应相位（颜色/图标/计时）并按相位自动隐藏。
- **插件市场**：市场索引拉取 + 宽容解析 + 排序/过滤 + 安装转发；**开源基础版删掉整个 src/market/ 后 glob 零匹配 → 市场静默降级，编译/构建不受影响**。
- **截图覆盖层**：一次完整框选交互（模式分发 → 放大镜 → 遮罩 → 选区 → 确认条）。

## 规则与坑

- **前端零业务、零真相源**：HUD 的成败相位由后端 ToolResult 决定，截图的像素由后端 PRE_SCREENSHOT 冻结帧决定，市场的安装由 Rust 安装器决定；前端只做「显示 + 转发 + 容错」。
- **Windows 上截图覆盖层已被原生 Win32 分层窗口链路取代**（src-tauri/src/commands/capture/surface/mod.rs:24-29），WebView 覆盖层退化为「非 Windows 且无原生实现」时的兜底链路，且窗口改为按需创建（main.rs:774-778）。
- 前端有 4 个 Vite 多页入口（index.html / mobile.html / capture_overlay.html / hud.html），本模块覆盖其中 2 个独立窗口 + 1 个按需摘除模块。
- 覆盖层样式必须引入设计系统令牌 + compact 按钮（styles.css）。
