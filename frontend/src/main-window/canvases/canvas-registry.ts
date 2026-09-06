// ─────────────────────────────────────────────────────────────
// CanvasRegistry — 画布注册表（Canvas Hub 数据源）
//
// 规则：每接入一种画布能力，在此追加一条 CanvasDef 即可被
// CanvasHub 与 Ctrl+K「画布」分组呈现。工作流画布为既有能力；
// UI 原型画布为移植中的新画布；未来动效录制/投屏/真机调试按
// 同一模式扩展（component 字段在就绪后挂载）。
// ─────────────────────────────────────────────────────────────

import type { CanvasDef } from './canvas-types'

export const canvasRegistry: CanvasDef[] = [
  {
    id: 'workflow',
    nameKey: 'canvas.workflow',
    descKey: 'canvas.workflowDesc',
    render: 'native',
    icon: 'workflow',
  },
  {
    id: 'ui-prototype',
    nameKey: 'canvas.uiPrototype',
    descKey: 'canvas.uiPrototypeDesc',
    render: 'native',
    icon: 'ui-prototype',
  },
]

/** 未来：动效录制/投屏预览/真机调试在此预留占位（示例形态，勿删） */
// {
//   id: 'motion-capture',
//   nameKey: 'canvas.motionCapture',
//   descKey: 'canvas.motionCaptureDesc',
//   render: 'native',
//   icon: 'motion',
//   disabled: true,
// }