// ─────────────────────────────────────────────────────────────
// Canvas 平台基础类型与消息协议
//
// 愿景：Nuphus 的「画布能力平台」承载多种画布工具（工作流编排、
// UI 原型设计、未来的动效录制/投屏预览/真机调试等）。每种画布经
// CanvasRegistry 注册挂载，宿主与画布之间通过标准消息协议通信，
// 新画布只需实现同一协议即可接入。
// ─────────────────────────────────────────────────────────────

/** 画布类型：注册表主键。新增画布在此扩展。 */
export type CanvasKind = 'workflow' | 'ui-prototype'

/** 画布注册项 */
export interface CanvasDef {
  id: CanvasKind
  /** 名称/描述 i18n key（locales canvas.*） */
  nameKey: string
  descKey: string
  /** 渲染方式：native=Nuphus 原生组件；iframe=隔离宿主（未来独立构建画布用） */
  render: 'native' | 'iframe'
  /** 封面图标语义 */
  icon: 'workflow' | 'ui-prototype'
  /** 尚未就绪时置灰（如移植中的画布） */
  disabled?: boolean
  /** 就绪提示角标（如「移植中」） */
  badge?: string
}

/** 画布导出载荷（画布 → 宿主）：prompt 落对话/图片落库/未来 video 落库 */
export interface CanvasExportPayload {
  kind: 'prompt' | 'image' | 'video' | 'code' | 'text'
  text?: string
  dataUrl?: string
  filename?: string
}

/** 宿主 → 画布：主题换肤（为嵌入/预览态画布提供） */
export interface CanvasThemePayload {
  theme: 'dark' | 'light'
}

/** 宿主 → 画布：语言同步（画布产物语言对齐用户偏好） */
export interface CanvasLocalePayload {
  lang: string
}

/**
 * 画布与宿主的统一消息协议（v1 声明，为投屏预览/真机调试等未来
 * 能力预留扩展位——新能力只增消息类型，不改宿主骨架）。
 */
export type CanvasToHostMessage =
  | { type: 'export'; payload: CanvasExportPayload }
  | { type: 'ready' }
  | { type: 'open'; payload: { dataUrl?: string; path?: string } }

export type HostToCanvasMessage =
  | { type: 'theme'; payload: CanvasThemePayload }
  | { type: 'locale'; payload: CanvasLocalePayload }
  | { type: 'ping' }
