# 前端 · i18n 与样式体系（locales/ styles/ 主题链路）

> 定位：Nuphus 前端的「语言层 + 皮肤层」——中英双语词条体系、设计 Token 单一来源、三主题/自定义主题/皮肤背景的完整应用链路。
> 规模：locales 5 文件 + styles 62 个 CSS + 主题链路 4 文件。

## 结构

三条相对独立的链路：

| 链路 | 文件 | 职责 |
|---|---|---|
| i18n | `locales/index.ts`（83）/ `zh.ts`（2463）/ `en.ts` / `workflowEditor.ts`（411）/ `workflowCanvasCopy.ts`（105） | React Context 形式的 `useLanguage()`（t(key, ...args) 插值替换）；中文词条表（扁平 Record<string,string>）+ 英文镜像（**键集合严格相等，测试钉死**）；工作流画布/编辑器专用双语表（[zh, en] 元组）；遗留「中文原文→英文」查找表 |
| 样式 Token | `styles/tokens.css`（547）/ `themes.css` + 约 62 个业务 CSS | tokens.css 是全项目设计变量的**单一来源**，分语义层/基础层/别名兼容层三层；themes.css 承载三主题差异；业务 CSS 按职责分组（聊天域 15 / 工作流域 4 / 设置页域 21 / 桌面工具域 4 / 悬浮件 4 / 移动端 3 / 插件 2 等） |
| 主题与皮肤 | `hooks/useTheme.tsx`（362）/ `hooks/customTheme.ts`（310）/ `ui/skinBg.ts`（119）/ `ui/assetUrl.ts` + `ui/SkinBackdrop.tsx` | ThemeProvider 管理内置主题切换、「我的主题」自定义列表（CSS 变量覆盖 + 皮肤背景快照）、实时预览、插件主题快照同步；customTheme 是纯数据模型与颜色派生（无 React 依赖）；skinBg 是皮肤背景的**唯一写入入口**（解析 → 广播 → 唯一渲染组件消费） |

## 功能作用

- **i18n 链路**：useLanguage 提供 t() 插值；zh/en 双词典；工作流编辑器专用双语表。
- **样式 Token 链路**：所有 UI 代码消费 tokens.css 的语义变量，不出现未经定义的硬编码值。
- **主题与皮肤链路**：三主题（dark 默认 / light / fresh）切换；「我的主题」自定义（CSS 变量覆盖 + 不透明度键集 + JSON 校验）；皮肤背景（本地路径 → blob URL → 唯一渲染组件）。

## 规则与坑

- **顶层设计宪法是 `frontend/docs/DESIGN-RATIONALE.md`**（2026-07-19 前端整体重塑档案）：所有后续 UI 代码必须以其为准，禁止出现未经定义的颜色/字号/间距/圆角/动效数值。
- skinBg 是皮肤背景唯一写入入口——解析、广播、唯一渲染组件消费，三处不得各写一半。
