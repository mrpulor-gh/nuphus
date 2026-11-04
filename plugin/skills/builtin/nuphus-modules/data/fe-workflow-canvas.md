# 前端 · 工作流画布（main-window/workflow-canvas/）

> 定位：**工作流 IR 的可视化编辑器**——把后端 `wf_list` 原始 IR 经「IR→画布投影 + dagre 分层布局 + xyflow 渲染」变成可下钻、可拖拽、可调试的画布，并用「IrEditOp 编辑管线 + L2/L3 双层校验 + 草稿存储」保证一切修改都能无损写回同一份 IR。
> 规模：97 文件（53 源码 + 10 CSS + 34 测试）≈ 13400 行源码 / 2800 行样式 / 4800 行测试。

## 结构

| 分组 | 文件 | 职责 |
|---|---|---|
| 画布页壳 | `CanvasPage.tsx`（2717） | 加载/保存/运行编排、拖拽、键盘表、弹层总装 |
| 投影 | `projection.ts`（631）/ `types.ts`（193） | IR→CanvasLayer 投影、数据边、合成锚点；画布类型契约（WorkflowIR / CanvasNode / CanvasLayer / IrEditOp / EditCheck） |
| 编辑管线 | `irEdit.ts`（470） | 6 种 IrEditOp 应用器 + 拦截矩阵（checkOp）+ **100 步整树快照撤销栈** + 步骤工厂 |
| 校验 | `validate.ts`（327） | L2 前端镜像校验（V1–V14 + input_reference），与后端 compiler 构成双层 |
| 变量系统 | `variableCatalog.ts`（190）/ `dataEdges.ts`（195）/ `variableRename.ts`（177）/ `fieldReferences.ts` / `runtimeFields.ts` | 控制流感知的变量目录（合流/可能未赋值/循环局部作用域）；`{{var}}` 与 VarRef 扫描；变量重命名（精确 span 替换）；运行期快照→可插入字段树 |
| 检查器 | `Inspector.tsx`（1560）/ `ToolParameterForm.tsx`（535）/ `inspectorDrafts.tsx`（176） | 右侧参数面板（按 kind 生成表单）；JSON Schema 驱动工具参数表单/JSON 双模；画布级草稿存储（跨隐藏面板保留） |
| 执行态与调试 | `runStatus.ts`（329）/ `ExecutionTraceViewer.tsx`（519）/ `WorkflowDebugPanel.tsx`（632）/ `debugSession.ts`（81） | workflow-event → 执行状态快照（rAF 合帧）；运行证据查看器（run/invocation/7 tab）；节点调试弹窗（node/through 两种范围）；调试 IPC 与预检 |
| 周边编辑器 | `WorkflowInputsEditor.tsx`（367）/ `IntentFormPanel.tsx`（355）/ `ScopedEditDialog.tsx`（329）+ `scopedEdit.ts`（272）/ `ToolPalette.tsx`（208）/ `OutlinePanel.tsx`（189）/ `ProblemsPanel.tsx`（368）/ `EnhancedModeToggle.tsx`（235）/ `nodes/StepNode.tsx`（234）/ `edges/` | 外部输入声明；意图表单（把阶段/子步骤文本交给 WorkflowAgent）；AI 局部修改（diff 预览+应用）；左侧工具竖条；结构大纲（替代 MiniMap）；问题面板；增强模式开关；节点卡片；数据边/顺序边 |

## 功能作用

- **IR → 视图投影**：把步骤树（seq/loop/if/wait 容器 + 叶子动作）投影为「分层画布」——root 层横向流（LR）、容器子层树状纵向（TB）、if 双泳道 THEN/ELSE；节点 id 与 Step.id 全局 1:1。
- **变量可见性**：按**控制流合流**（而非 DFS 顺序）计算每一步可见变量、可能未赋值（maybeUnset）、循环局部变量（item/_index）作用域，并在画布上画数据边、悬空锚点。
- **编辑与持久化**：所有编辑表达为 IrEditOp，先过合法性矩阵再 applyOp 生成新树 → 重投影 → `wf_save`。
- **执行态可视化与调试**：订阅 workflow-event 做节点状态叠加；读取 wf_trace_list/read 展示真实运行证据（含执行前后变量快照）；单节点/贯穿调试经 wf_debug_run/control。

## 规则与坑

- **禁止画布维护独立图模型再整体导出**（irEdit.ts:5）——一切修改必须无损写回同一份 IR。
- 上游唯一宿主：`main-window/workflow/CanvasWorkbenchPage.tsx` 以 lazy() 动态导入 CanvasPage、空闲预取 chunk、经 useCanvasLeaveGuard 注册离开确认、并提供 portal 挂载点。
- 测试按「纯函数 → 组件 → 整页编排」三层组织（整页编排 mock 掉 @xyflow/react 与 api）。
