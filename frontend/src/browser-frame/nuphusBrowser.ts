// nuphusBrowser.ts — 壳 ↔ content 通信缝（本期 no-op 占位，下个任务接真桥）。
//
// 背景（docs/browser-shell-arch.md §4.3）：frame（tauri:// 本地页）与 content（远程页）
// 不同源，禁止 postMessage，一切通信走 invoke → Rust → emit。标注 overlay / 录制脚本
// 由 content 侧 initialization_script 注入（下个任务），壳页面这侧只保留一个**调用缝**：
// 组件调 window.__nuphusBrowser.setAnnotate/setRecording，缝的实现本期只有日志。
//
// 为什么现在就挂 window.__nuphusBrowser 而不是等下个任务：缝要先于实现存在，
// 否则下个任务要在「改通信链路」和「改组件」之间二选一；挂在 window 上（而非模块内
// 私有对象）是为了让后续的真桥（content 脚本经 Rust 回灌）可以直接整体替换这两个方法，
// 组件零改动。

/** 壳 ↔ content 通信缝。真桥接入前只有日志，语义与架构文档 §4.3 的 content 侧对象对称 */
export interface NuphusBrowserSeam {
  /** 标注 overlay 开关（content 侧 `__nuphusAnnotator` 的编辑态） */
  setAnnotate(on: boolean): void
  /** 录制开关（content 侧 capture listener 的启停） */
  setRecording(on: boolean): void
}

declare global {
  interface Window {
    /** 壳 ↔ content 通信缝（见本文件头注释）。真桥接入后由后续任务整体替换 */
    __nuphusBrowser?: NuphusBrowserSeam
  }
}

/** 未接入真桥时的占位实现：不静默吞掉——每个开关动作都留一条可追溯日志 */
function createStubSeam(): NuphusBrowserSeam {
  return {
    setAnnotate(on: boolean) {
      console.info(
        `[browser-frame] setAnnotate(${on}) —— 标注 overlay 脚本未接入（下个任务），本期 no-op`,
      )
    },
    setRecording(on: boolean) {
      console.info(`[browser-frame] setRecording(${on}) —— 录制脚本未接入（下个任务），本期 no-op`)
    },
  }
}

/**
 * 取通信缝（不存在则安装占位实现）。
 *
 * 幂等：StrictMode 双挂载 / 多次调用都复用同一个对象，不会把已接入的真桥冲掉。
 */
export function getNuphusBrowser(): NuphusBrowserSeam {
  if (typeof window === 'undefined') return createStubSeam()
  if (!window.__nuphusBrowser) window.__nuphusBrowser = createStubSeam()
  return window.__nuphusBrowser
}
