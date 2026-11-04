# 前端 · 工作流面板与运行（main-window/workflow/）

> 定位：**工作流功能的主窗口侧门面**——工作流列表/重命名/删除/新建、运行前外部输入收集与参数方案、定时调度对话框与运行历史回放入口、Chat Agent 配置页，以及「画布工作台」这个懒加载外壳；执行态闸门、输入契约、敏感值边界在此收口。
> 规模：21 文件（9 源码 + 9 测试 + 3 CSS）≈ 3400 行。

## 结构

| 文件 | 规模 | 职责 |
|---|---|---|
| `WorkflowPage.tsx` | 467 | 工作流列表页：搜索/状态徽标/行内重命名/删除二次确认/定时徽标/画布新建/Chat Agent 入口 |
| `WorkflowRunModal.tsx` | 211 | 运行确认弹窗：步骤树预览（seq/loop/if/wait 子步骤展开）+ 外部输入表单 + 续跑提示（可拖拽） |
| `WorkflowInputsForm.tsx` | 396 | 外部输入的**唯一实现**：hook + 表单 + 画布共用的 Dialog |
| `WorkflowInputPresets.tsx` | 196 | 本机参数方案（localStorage 持久化，**敏感字段永不落盘**） |
| `WorkflowScheduleDialog.tsx` | 641 | 定时调度对话框：频率预设 ↔ 五字段 Cron 双向映射、250ms 防抖预览、固定输入值快照、敏感值保留 |
| `ScheduleHistoryPage.tsx` | 318 | 定时运行历史页：筛选/分页/清理/回放入口 |
| `CanvasWorkbenchPage.tsx` + `CanvasWorkbenchLoading.tsx` | 131 + 18 | 画布工作台外壳：目标工作流选择、兜底自选草稿、懒加载 CanvasPage、保留原生拖动区 |
| `ChatAgentConfig.tsx` | 834 | Chat Agent 配置页：模板 CRUD/Active、工作流内联步骤覆盖、默认推理轮数 |

## 功能作用

- **工作流资产管理**：搜索、列表、状态徽标、行内重命名、删除二次确认、「画布新建」（直建空白草稿并跳画布）。
- **运行前输入契约**：把后端 `workflow.inputs[]` 声明渲染成表单，做类型转换/必填校验/敏感值隔离，结果以 payload 形式交给运行入口。
- **运行确认与步骤预览**：步骤树（含子步骤展开）与外部输入表单，给出「上次可续跑」的合并语义提示。
- **定时调度与历史**：ScheduleConfig 编辑（频率预设 ↔ Cron 互转）+ 运行历史筛选/分页/清理/回放。

## 规则与坑

- **输入契约明文**（WorkflowInputsForm.tsx:1-11）：只提交已声明字段、default 预填、required 未填阻断、sensitive 只进内存。
- **暂停/恢复边界必须说清**：本模块**不实现**工作流运行中的暂停/恢复/终止——这三件事在 App.tsx 挂载的 WorkflowTaskPanel 经 useAgentControl.handleWfPause/handleWfResume 调 wf_pause/wf_resume 完成；本模块只在两处与暂停态相关（ScheduleDialog 的 readOnly；RunModal 依据 run_history[0].status 提示「断点续跑时新填输入仅对快照中不存在的变量生效」）。
- 「每月 29/30/31 日」配置重开不被钳成 28 日——有专门回归测试（WorkflowScheduleDialog.persistence.test.tsx）。
