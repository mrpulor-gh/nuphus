# 桌面控制与自动化（src/desktop/ src/desktop_automation/）

> 定位：Nuphus 的「眼睛与手」——Rust 原生桌面感知/操作层。向上经两条互补通道供给 Agent 工具层：`DesktopClient`（坐标级输入 + 截图 + OCR/YOLO 感知 + 字库找字）与 `desktop_automation`（UIA/AX 语义自动化 + 风险门 + 决策提供者）；向下把 Win32 / CoreGraphics / X11 / Apple Accessibility 全部封装在平台适配器内。
> 规模：29 文件约 18000 行。

## 结构

| 通道 | 文件 | 职责 |
|---|---|---|
| 坐标通道 | `desktop/client.rs`（1895） | 中枢：screenshot / perceive / find_text / wait_for，返回统一 serde_json::Value，被 tools/desktop_executors.rs 直接 match 分发 |
| 坐标通道 | `capture_context.rs`（356）/ `ui_perception.rs`（412）/ `vision.rs`（571）/ `vision_ocr.rs`（925）/ `paddle_ocr.rs`（720）/ `linux_window.rs` / `macos_window.rs` / `targets.rs`（995） | 截图溯源与坐标换算；OCR+YOLO 合并去重；纯内存像素找色；视觉大模型 OCR；PP-OCRv4 ONNX 本地 OCR；平台窗口管理；桌面目标注册/目录/绑定/保存恢复 |
| 字库管线 | `desktop/dict_ocr/`（6 文件） | 前景背景色分析 → 二值化 → 位图模板匹配全屏滑窗找字 → 投影法字符切分 → .dict 字库（bincode）存取 |
| 语义通道 | `desktop_automation/{types 566, runner 713, risk 138, outcome 102}` | 平台无关内核：UiNode/Observation/ActionCandidate/RiskClass/ExecutionGrant/LocalPolicy/RunLimits；有界自动化主循环 + 四大 trait（ComputerObserver/CandidateBuilder/ComputerExecutor/Verifier） |
| 重适配器 | `windows_uia.rs`（3128）/ `macos_accessibility.rs`（1662）+ `native.rs`（1380） | 两个平台适配器（占本模块 52% 代码量）：观察/候选/执行/回放 |
| 决策后端 | `jev.rs`（759）/ `laya.rs`（697） | Jev（System One）与 Laya 自托管决策提供者（同协议不同线型） |

## 功能作用

- **截图链路**（xcap/Win32）、**输入链路与前台保证**、**OCR 三层**（本地 PaddleOCR / 视觉大模型 / 字库找字）、**坐标换算**（screen_point：图像坐标→屏幕坐标）、**颜色查找**（9 档扫描方向）。
- **UIA/AX 语义自动化**：平台无关的「观察 → 候选 → 策略 → 执行 → 验证」有界闭环，被 `tools/semantic_desktop.rs` 包装成 `desktop_semantic_*` / `desktop_agent_step` 工具。

## 规则与坑

- **隐私铁律**：原生句柄（HWND / AX 指针 / COM 元素 / PID / 几何）**永不跨出适配器**；模型只能看到脱敏 Observation 与 ActionCandidate，只能回一个 candidate_id。
- **「投递成功 ≠ 业务目标达成」**：`outcome.rs` 把「发没发出去」（DispatchState）与「做完了吗」（Verification）分成两个正交概念；`Verification::Unexpected/Unknown` 时**不重放非幂等动作**（`runner.rs:399-403`）。
- **资源互斥归属**：`DesktopTargetService::owner()` 取 `automation_gate::current_execution_owner()`，token 不可跨执行体使用（`targets.rs:97-100`）。
