/**
 * 临时伙伴的「刷新续命」存储。
 *
 * 背景：webview 刷新（F5 / 界面刷新）只重载前端，Rust 进程与 `slots` 表
 * 不重建——后端临时会话与执行体仍然活着。把 key 持久化，刷新后挂回同一个
 * key 即可无缝继续（消息列表随 key 落 localStorage）。
 *
 * 边界（诚实声明）：整机重启（Rust 进程也重建）后后端会话已不在，恢复的
 * 只有 UI 消息；下一轮发送会建新执行体，模型上下文从零开始——这是 Side Chat
 * 「不落库」设计的必然代价，不做会话级持久化去突破它。
 *
 * 所有读写都 try/catch：隐私模式 / 超额等场景下静默降级（刷新续命失效），
 * 不阻断主流程。
 */

const KEY = 'nuphus.sidechat.key'
const MIN = 'nuphus.sidechat.min'

const messagesKey = (key: string) => `nuphus.sidechat.msgs.${key}`

export function loadSideChatKey(): string | null {
  try {
    return localStorage.getItem(KEY)
  } catch {
    return null
  }
}

export function saveSideChatKey(key: string): void {
  try {
    localStorage.setItem(KEY, key)
  } catch {
    /* 刷新续命降级：不影响当轮使用 */
  }
}

export function loadSideChatMinimized(): boolean {
  try {
    return localStorage.getItem(MIN) === '1'
  } catch {
    return false
  }
}

export function saveSideChatMinimized(minimized: boolean): void {
  try {
    localStorage.setItem(MIN, minimized ? '1' : '0')
  } catch {
    /* 同上 */
  }
}

export function loadSideChatMessages<T>(key: string): T[] {
  try {
    const raw = localStorage.getItem(messagesKey(key))
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed) ? (parsed as T[]) : []
  } catch {
    return []
  }
}

export function saveSideChatMessages(key: string, messages: unknown[]): void {
  try {
    localStorage.setItem(messagesKey(key), JSON.stringify(messages))
  } catch {
    /* 超额 / 隐私模式：忽略，刷新后从后端可见历史之外重来 */
  }
}

/** 关闭 / 主会话切换时调用：连消息一起清，避免 localStorage 积陈旧会话。
 *  注意：**不清偏好**（PREFS）——模型 / 权限是用户级选择，关面板、切主会话
 *  都不该丢；要真清得用户手动改回默认。 */
export function clearSideChatStorage(key: string): void {
  try {
    localStorage.removeItem(KEY)
    localStorage.removeItem(MIN)
    localStorage.removeItem(messagesKey(key))
  } catch {
    /* 忽略 */
  }
}

// ── 面板偏好（用户级，跨刷新 / 跨开关保持）──
//
// 与 KEY / MIN / 消息的分界：那三项是**会话级**状态，关闭即清；偏好是**用户级**
// 选择。此前只有前三项落了盘，模型每次刷新都被打回「跟随主会话」，权限开关也被
// 打回全开——用户每次重开面板都要重新选一遍。

export interface SideChatPrefs {
  /** 模型 id；空 = 跟随主会话当前模型 */
  model: string
  /** 模型所属提供商；与 model 成对（同名模型跨提供商时必须靠它消歧） */
  provider: string
  /** 推理强度覆盖；null = 提供商默认 */
  effort: string | null
  fileAccess: boolean
  webSearch: boolean
  /**
   * 面板内选定的项目目录（绝对路径）；空 = 跟随主会话当前项目。
   * 与全局 `preferences.project_dir` 刻意分离：临时伙伴常常用来「顺手看看别的
   * 项目」，改它不该影响主会话的目录注入。选中的路径随每条消息下发（见
   * side_chat_send 的 project_dir），不做进程级 chdir。
   */
  projectPath: string
  /**
   * 面板尺寸（px）。null = 默认几何（CSS 的 width + top/bottom 推导高度）。
   * 用户拉过边缘或一键放大后才落盘——默认值不写进 localStorage，升级 / 清偏好
   * 都能一键回到初始几何。
   */
  size: { w: number; h: number } | null
}

const PREFS = 'nuphus.sidechat.prefs'

const DEFAULT_PREFS: SideChatPrefs = {
  model: '',
  provider: '',
  effort: null,
  fileAccess: true,
  webSearch: true,
  projectPath: '',
  size: null,
}

export function loadSideChatPrefs(): SideChatPrefs {
  try {
    const raw = localStorage.getItem(PREFS)
    if (!raw) return DEFAULT_PREFS
    // 逐字段校验：旧版本 / 手改过的脏数据按缺省值回落，不让整个偏好失效
    const p = JSON.parse(raw) as Partial<SideChatPrefs>
    return {
      model: typeof p.model === 'string' ? p.model : DEFAULT_PREFS.model,
      // 旧版本没这个字段：缺省空串（= 让后端按 id 解析，行为与升级前一致）
      provider: typeof p.provider === 'string' ? p.provider : DEFAULT_PREFS.provider,
      effort: typeof p.effort === 'string' ? p.effort : DEFAULT_PREFS.effort,
      fileAccess: typeof p.fileAccess === 'boolean' ? p.fileAccess : DEFAULT_PREFS.fileAccess,
      webSearch: typeof p.webSearch === 'boolean' ? p.webSearch : DEFAULT_PREFS.webSearch,
      projectPath: typeof p.projectPath === 'string' ? p.projectPath : DEFAULT_PREFS.projectPath,
      // 尺寸：必须是「两个正数」才认，否则回落默认几何。旧版本没这个字段 /
      // 手改坏的脏数据都不能让面板变成 0×0。
      size:
        p.size &&
        typeof p.size === 'object' &&
        typeof (p.size as { w?: unknown }).w === 'number' &&
        typeof (p.size as { h?: unknown }).h === 'number' &&
        (p.size as { w: number }).w > 0 &&
        (p.size as { h: number }).h > 0
          ? { w: (p.size as { w: number }).w, h: (p.size as { h: number }).h }
          : null,
    }
  } catch {
    return DEFAULT_PREFS
  }
}

export function saveSideChatPrefs(prefs: SideChatPrefs): void {
  try {
    localStorage.setItem(PREFS, JSON.stringify(prefs))
  } catch {
    /* 隐私模式 / 超额：忽略，刷新后回默认 */
  }
}
