/* ═══════════════════════════════════════════
   debug.ts — 高频调试日志开关（默认关闭）

   背景：IPC / 事件热路径上的「成功路径」日志（每次 invoke 打结果体、每个
   token_usage 事件打一行）会把 DevTools 控制台打满——既掩盖真正的错误，
   又让每次调用都白做一次结果序列化（开销落在主线程）。实测 dev 环境
   ≈67 条/分钟，DevTools 打开后连元素面板都点不动。

   约定：**默认静音**；排查时在 DevTools 控制台开启后刷新：
     localStorage.setItem('nuphus:debug', '1')   // 开
     localStorage.removeItem('nuphus:debug')     // 关

   边界：只用于「高频 + 成功」路径。失败 / 异常 / 罕见路径仍应直接
   console.warn / console.error —— 那些是必须被看见的信号，不受本开关约束。
   ═══════════════════════════════════════════ */

/** localStorage 键：置为 '1' 即开启高频调试日志 */
export const DEBUG_LOG_KEY = 'nuphus:debug'

/** 模块加载时读一次（刷新后生效，避免热路径上反复读 localStorage） */
const enabled = (() => {
  try {
    return localStorage.getItem(DEBUG_LOG_KEY) === '1'
  } catch {
    // 隐私模式 / 非浏览器环境：按关闭处理
    return false
  }
})()

/**
 * 高频调试日志是否开启。调用点必须用它包住**整段**日志逻辑
 * （含字符串拼接、JSON.stringify），关闭时才能真正零开销：
 *
 *   if (debugEnabled()) console.log(`[Trace] ${expensive()}`, JSON.stringify(payload))
 */
export function debugEnabled(): boolean {
  return enabled
}
