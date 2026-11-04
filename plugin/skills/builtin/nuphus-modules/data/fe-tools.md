# 前端 · 工具面板（main-window/tools/）

> 定位：主窗口内的「桌面自动化工具前台」——桌面悬浮工具条（截图/框选/取色/坐标/字典 OCR）、主窗内选区截图交互、Promise 化捕获 hook，以及独立整页的内置文件工具页（图片/视频/音频/文档/PDF 处理）。
> 规模：8 文件。

## 结构

| 文件 | 规模 | 职责 |
|---|---|---|
| `DesktopToolbar.tsx` | 924 | 桌面悬浮工具条 v6：dock 显隐、5 工具按钮、取色 9 槽面板、OCR 字典面板挂载、Ctrl+1/2/3 热键执行、结果轮询分发（renewHold） |
| `OcrDictionary.tsx` | 2016 | 「字库文字识别」全功能面板：自动识别（比对全部字库）、添加字库（截图/字体渲染 → 提取点阵 → 命名保存）、字库查看/删字符/删字库、屏幕吸色 |
| `ToolsPage.tsx` | 1999 | 内置工具页：分类选择器 + 能力卡片网格（Level 1）→ 详情页（文件区+设置区，Level 2），驱动 23 种文件处理能力 |
| `tools.css` | 938 | ToolsPage 专属样式：两级导航、split/stacked 两种详情布局、质量/模式预设卡、压缩结果可视化 |
| `RegionPicker.tsx` | 408 | 主窗内全屏选区组件：截图背景 + canvas 拖拽框选 + 尺寸/坐标标签 + 确认/重选 |
| `useToolCapture.ts` | 205 | 捕获 hook：`start_overlay_mask` → 轮询 `take_capture_result`，Promise 化，带取消/超时/并发保护 |
| `ScreenCaptureTool.tsx` | 67 | 截图模式薄壳：RegionPicker(mode="capture") + 确认后 execute_tool('desktop_screenshot') 落盘 |
| `DesktopToolbar.test.tsx` | 478 | 唯一测试文件：结构契约（废弃项钉死）、显隐时序（假计时器）、热键契约 |

## 功能作用

- **随时呼出的桌面捕获工具**：DesktopToolbar 在聊天区右侧渲染常驻浮动按钮列（dock），提供截图、框选、取色、鼠标坐标、字典 OCR 五个工具 + 「登记应用」与「内置工具页」两个入口。**所有工具最终都通过 `start_overlay_mask` 唤起原生遮罩、再轮询 `take_capture_result` 取结果**。
- **主窗内区域选择交互**：「先全屏截图 → canvas 拖拽框选 → 确认保存」，供 App 级 regionPickerMode 状态挂载。
- **工作流/Agent 请求用户输入时的捕获**：useToolCapture 把「起遮罩 + 轮询」协议封装成 Promise，供 UserInputPrompt（request_user_input 的 screenshot/region/mouse_pos/color 输入类型）复用。
- **内置文件工具页**：两级导航把 23 种文件处理能力（图片/视频/音频/文档/PDF）以卡片形式暴露。

## 规则与坑

- 工具页定位（ToolsPage.tsx:1-8 文件头注释）：**「工具是内部机制（invoke 命令），不是 agent 工具调用项」**——命令实现位于 `src-tauri/src/commands/tools/*`，LLM 不可经 execute_tool 调用。
- 模块之下是 `commands/toolbar.rs`（截图遮罩生命周期）、`commands/desktop.rs`（鼠标坐标/应用登记）、`commands/dict_ocr.rs`（字典 OCR）与 nuphus crate 的桌面工具执行体。
- 挂载点：App.tsx:689（DesktopToolbar）、:879-895（ScreenCaptureTool/RegionPicker）、:1146-1149（懒加载 ToolsPage）。
