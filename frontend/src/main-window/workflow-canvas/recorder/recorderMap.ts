/**
 * recorderMap.ts — 录制捕获 → 画布真实 Action + step_draft 的纯映射
 *
 * 铁律：每个画布节点必须是真实可执行 Action（tool 步骤 or sleep），禁止 custom。
 * 字段名来自 src/tools/desktop_schemas.rs 的 ToolDefinition：
 * - desktop_mouse:    action(click/double_click/hover/scroll/position/move) x y button direction amount
 * - desktop_input:    mode(type/hotkey) hwnd text send keys
 * - desktop_find_image: template_path region threshold
 */

import type { WorkflowStep } from '../../../core/types'
import type { BuiltStep, IntentPayload, RecDraft, RecPending } from './recorderTypes'

export const TOOL_MOUSE = 'desktop_mouse'
export const TOOL_INPUT = 'desktop_input'
export const TOOL_FIND_IMAGE = 'desktop_find_image'
export const TOOL_BROWSER_NAVIGATE = 'browser_navigate'
export const TOOL_BROWSER_CLICK = 'browser_click'
export const TOOL_BROWSER_EXTRACT = 'browser_extract'
/** 元素级提取工具：browser_extract 只有 max_chars（整页文本，无 selector 参数），
 *  元素级提取用 browser_exec 的 h.extract(sel) helper 表达（executor 白名单已含 browser_exec）。 */
export const TOOL_BROWSER_EXEC = 'browser_exec'

/** 主窗口标题含 Nuphus → 判定为误捕获（index.html title: "Nuphus - 智慧协作伙伴"） */
export function isSelfCapture(windowTitle: string | null | undefined): boolean {
  return !!windowTitle && /nuphus/i.test(windowTitle)
}

/**
 * C3：网页点击/内容获取前置检查——录制会话草稿中是否已有「打开网址」(browser_navigate) 步骤。
 * CDP managed 浏览器非原生浏览器，点击/内容操作必须先在目标页内进行；navigate 自身无需前置。
 */
export function hasNavigateDraft(drafts: RecDraft[]): boolean {
  return drafts.some(d => d.action === 'browser_navigate')
}

/** C3 阻断提示文案（必须精确含「请先添加『打开网址』步骤」） */
export const NAVIGATE_PREREQ_MESSAGE =
  '网页录制前置要求：请先添加『打开网址』步骤打开支持的浏览器（CDP 浏览器非原生浏览器，点击/内容操作前需要先打开目标网页）'

/** 元素级提取脚本：CSS selector 经 JSON.stringify 嵌入，避免引号/反斜杠破坏 JS 字面量 */
export function elementExtractScript(selector: string): string {
  return `h.extract(${JSON.stringify(selector)})`
}

/** desktop_mouse.scroll 方向：滚轮上滚 delta>0 → up */
export function wheelDirection(delta: number | null): 'up' | 'down' {
  return (delta ?? 0) >= 0 ? 'up' : 'down'
}

/** desktop_mouse.scroll ticks：1 格 = 120 的滚轮刻度，下限 1 */
export function wheelAmount(delta: number | null): number {
  return Math.max(1, Math.round(Math.abs(delta ?? 0) / 120))
}

function baseDraft(
  action: string,
  payload: IntentPayload,
  extraParams: Record<string, unknown>,
): Omit<RecDraft, 'evidence'> {
  return {
    action,
    intent: payload.intent.trim(),
    params: { ...extraParams, variable: payload.variable },
    exception_note: payload.exceptionNote.trim() || null,
  }
}

function toBuilt(kind: 'tool' | 'sleep', doAction: WorkflowStep['do'], draft: RecDraft): BuiltStep {
  return { kind, name: draft.intent, do: doAction, draft }
}

/** find_image 步骤捕获匹配坐标的变量名前缀（capture 名须满足 ^[a-zA-Z_][a-zA-Z0-9_.]*$） */
const FIND_CAPTURE_PREFIX = 'fi_x'

/** 生成 find_image 捕获变量名：以「该动作预计步骤序号」作底，同一会话内单调递增防重复捕获 */
export function findCaptureVarName(stepNo: number): string {
  return `${FIND_CAPTURE_PREFIX}${stepNo}`
}

/** 意图面板 text 动作选 chatagent 时写入 chat 步骤消息的占位前缀（WorkflowAgent 阅读后会改写） */
export const CHATAGENT_MESSAGE_PREFIX = '此处由 chatagent 处理：'

/** 意图面板 text 动作选 chatagent 时展示/落盘的说明文案 */
export const CHATAGENT_INTENT_PREFIX = '交给 chatagent：'

/** 内部构建：单个 pending 动作 → 至多一个画布可执行体（特化多节点场景由 buildSteps 先行展开） */
function buildOne(p: RecPending, payload: IntentPayload): BuiltStep | null {
  switch (p.action) {
    case 'click': {
      const ev = p.event
      if (!ev) return null
      const button = ev.button ?? 'left'
      const action = ev.kind === 'double_click' ? 'double_click' : 'click'
      const withParams = { action, x: ev.x, y: ev.y, button }
      const draft: RecDraft = {
        ...baseDraft('click', payload, {
          window_title: ev.window_title || null,
          x: ev.x,
          y: ev.y,
          button,
        }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'tool', tool: TOOL_MOUSE, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_MOUSE, with: withParams }, draft)
    }
    case 'scroll': {
      const ev = p.event
      if (!ev) return null
      const direction = wheelDirection(ev.wheel_delta)
      const amount = wheelAmount(ev.wheel_delta)
      const withParams = { action: 'scroll', x: ev.x, y: ev.y, direction, amount }
      const draft: RecDraft = {
        ...baseDraft('scroll', payload, {
          window_title: ev.window_title || null,
          x: ev.x,
          y: ev.y,
          direction,
          amount,
          wheel_delta: ev.wheel_delta ?? null,
        }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'tool', tool: TOOL_MOUSE, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_MOUSE, with: withParams }, draft)
    }
    case 'text': {
      const ev = p.event
      if (!ev) return null
      const text = payload.text?.trim() || '{{input}}'
      const send = payload.send || 'none'
      const withParams = { mode: 'type', hwnd: ev.hwnd ?? 0, text, send }
      const draft: RecDraft = {
        ...baseDraft('text', payload, {
          window_title: ev.window_title || null,
          x: ev.x,
          y: ev.y,
          button: ev.button ?? 'left',
          text,
          send,
        }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'tool', tool: TOOL_INPUT, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_INPUT, with: withParams }, draft)
    }
    case 'hotkey': {
      const ev = p.event
      if (!ev) return null
      const keys = ev.keys ?? []
      const withParams = { mode: 'hotkey', hwnd: ev.hwnd ?? 0, keys }
      const draft: RecDraft = {
        ...baseDraft('hotkey', payload, {
          window_title: ev.window_title || null,
          keys,
        }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'tool', tool: TOOL_INPUT, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_INPUT, with: withParams }, draft)
    }
    case 'sleep': {
      const seconds = Math.max(0.1, payload.seconds ?? 1)
      const draft: RecDraft = {
        ...baseDraft('sleep', payload, { seconds }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'sleep', seconds },
      }
      return toBuilt('sleep', { sleep: seconds }, draft)
    }
    case 'region':
    case 'find_image': {
      const overlay = p.overlay
      if (!overlay) return null
      const withParams: Record<string, unknown> = {
        template_path: overlay.path,
        threshold: 0.9,
        region: overlay.rect,
      }
      const draft: RecDraft = {
        ...baseDraft(p.action, payload, {
          template_path: overlay.path,
          threshold: 0.9,
          rect: overlay.rect,
        }),
        evidence: { screenshot: overlay.path, rect: overlay.rect },
        canvas: { kind: 'tool', tool: TOOL_FIND_IMAGE, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_FIND_IMAGE, with: withParams }, draft)
    }
    // ── browser 动作组：每个动作单节点（tool 步骤，with 字段对齐浏览器工具 schema）──
    case 'browser_navigate': {
      const url = (payload.url ?? '').trim()
      if (!url) return null
      const withParams = { url }
      const draft: RecDraft = {
        ...baseDraft('browser_navigate', payload, { url }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'tool', tool: TOOL_BROWSER_NAVIGATE, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_BROWSER_NAVIGATE, with: withParams }, draft)
    }
    case 'browser_click': {
      const bc = p.browserCapture
      if (!bc || !bc.selector) return null
      const selector = bc.selector
      const withParams = { selector }
      const draft: RecDraft = {
        ...baseDraft('browser_click', payload, {
          selector,
          tag: bc.tag,
          text: bc.text ?? '',
          href: bc.href ?? null,
          url: bc.url ?? null,
        }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'tool', tool: TOOL_BROWSER_CLICK, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_BROWSER_CLICK, with: withParams }, draft)
    }
    case 'browser_extract': {
      const contentNote = (payload.contentNote ?? '').trim()
      const bc = p.browserCapture
      if (bc && bc.selector) {
        // 元素级提取（C2「选择目标元素」捕获路径）：复用 rec_browser_capture_click_*
        // 捕获 selector，但不生成点击。browser_extract 仅支持整页 max_chars，无 selector
        // 参数 → 用 browser_exec 的 h.extract(sel) 表达「提取该元素文本」。
        const script = elementExtractScript(bc.selector)
        const withParams = { script }
        const draft: RecDraft = {
          ...baseDraft('browser_extract', payload, {
            content_note: contentNote || '获取该元素内容',
            selector: bc.selector,
            tag: bc.tag,
            text: bc.text ?? '',
            url: bc.url ?? null,
          }),
          evidence: { screenshot: null, rect: null },
          canvas: { kind: 'tool', tool: TOOL_BROWSER_EXEC, with: withParams },
        }
        return toBuilt('tool', { tool: TOOL_BROWSER_EXEC, with: withParams }, draft)
      }
      // 无真实捕获：如实生成 browser_extract（空 with —— 运行时取当前页文本；
      // 面板填写的获取说明记录在 draft（WorkflowAgent 可后续加 max_chars/区域细化））
      const withParams: Record<string, unknown> = {}
      const draft: RecDraft = {
        ...baseDraft('browser_extract', payload, {
          content_note: contentNote || '获取当前页面主要内容',
        }),
        evidence: { screenshot: null, rect: null },
        canvas: { kind: 'tool', tool: TOOL_BROWSER_EXTRACT, with: withParams },
      }
      return toBuilt('tool', { tool: TOOL_BROWSER_EXTRACT, with: withParams }, draft)
    }
    default:
      return null
  }
}

/** 将意图文本加工为画布节点名 / draft 意图（chatagent 前缀 + 原意；截断由 CanvasPage 负责） */
function chatDraftIntent(rawIntent: string): string {
  const intent = rawIntent.trim()
  return `${CHATAGENT_INTENT_PREFIX}${intent}`
}

/** 将 find_image 步骤的「找图后点击」意图派生为点击 draft 的意图文案 */
function clickDraftIntent(rawIntent: string, action: 'click' | 'double_click'): string {
  const intent = rawIntent.trim()
  const suffix = action === 'double_click' ? '双击' : '单击'
  return `${intent} → 找图后${suffix}匹配位置`
}

/**
 * 由 pending + 意图面板 payload 展开真实画布步骤（一个 pending 动作可能产出多个节点）：
 * - text + target=chatagent  → 1 个 chat 步骤（基础窗口参数可空，内容由 workflowAgent 填写）
 * - find_image + clickAfter  → 2 个 tool 步骤（desktop_find_image 捕获输出 + desktop_mouse 引用其坐标）
 * - 其余 → 1 个真实可执行步骤（buildOne）
 * 铁律：每个产出都是可执行 Action，禁止 custom 占位；无法映射返回空数组。
 */
export function buildSteps(p: RecPending, payload: IntentPayload): BuiltStep[] {
  // ── text → chatagent：生成 chat 步骤（由 workflowAgent 在运行前补全内容）──
  if (p.action === 'text' && payload.target === 'chatagent') {
    const intent = payload.intent.trim()
    if (!intent) return []
    const message = `${CHATAGENT_MESSAGE_PREFIX}${intent}`
    const draft: RecDraft = {
      ...baseDraft('text_chat', payload, { target: 'chatagent' }),
      evidence: { screenshot: null, rect: null },
      canvas: { kind: 'chat', with: {} },
    }
    return [
      {
        kind: 'chat',
        name: chatDraftIntent(intent),
        do: { chat: message, with: {} },
        draft,
      },
    ]
  }

  // ── find_image → 找图后点击：desktop_find_image（capture 输出）+ desktop_mouse（变量引用坐标）──
  if (p.action === 'find_image' && payload.clickAfter && payload.clickAfter !== 'none') {
    const overlay = p.overlay
    if (!overlay) return []
    const clickAction = payload.clickAfter === 'double_click' ? 'double_click' : 'click'
    const cap = findCaptureVarName(p.stepNo)
    const findWith: Record<string, unknown> = {
      template_path: overlay.path,
      threshold: 0.9,
      region: overlay.rect,
    }
    const findDraft: RecDraft = {
      ...baseDraft('find_image', payload, {
        template_path: overlay.path,
        threshold: 0.9,
        rect: overlay.rect,
        click_after: clickAction,
      }),
      evidence: { screenshot: overlay.path, rect: overlay.rect },
      canvas: { kind: 'tool', tool: TOOL_FIND_IMAGE, with: findWith, capture: cap },
    }
    // 执行期由 find_image 输出驱动点击坐标：x/y 引用该步骤 capture 的匹配框（左上角坐标）
    const xRef = `{{${cap} | json "x"}}`
    const yRef = `{{${cap} | json "y"}}`
    const clickWith: Record<string, unknown> = {
      action: clickAction,
      x: xRef,
      y: yRef,
      button: 'left',
    }
    const clickDraft: RecDraft = {
      ...baseDraft('click', payload, {
        source_capture: cap,
        click_after: clickAction,
        button: 'left',
        x: xRef,
        y: yRef,
      }),
      evidence: { screenshot: overlay.path, rect: overlay.rect },
      canvas: { kind: 'tool', tool: TOOL_MOUSE, with: clickWith },
    }
    return [
      {
        kind: 'tool',
        name: payload.intent.trim(),
        do: { tool: TOOL_FIND_IMAGE, with: findWith },
        capture: cap,
        draft: findDraft,
      },
      {
        kind: 'tool',
        name: clickDraftIntent(payload.intent, clickAction),
        do: { tool: TOOL_MOUSE, with: clickWith },
        draft: clickDraft,
      },
    ]
  }

  // ── 其余动作：单步展开 ──
  const one = buildOne(p, payload)
  return one ? [one] : []
}

/** 兼容包装：取 buildSteps 首步（常规单步动作下与旧 buildStep 行为一致） */
export function buildStep(p: RecPending, payload: IntentPayload): BuiltStep | null {
  return buildSteps(p, payload)[0] ?? null
}

/** region 步骤用 desktop_find_image 作「区域存在性锚点」的语义说明（意图面板展示） */
export const REGION_ANCHOR_SEMANTIC =
  '区域无「坐标消费」真实工具可用（画布禁止 custom 占位），因此以该 ROI 截图作 template 生成 desktop_find_image 锚点步骤：执行时在当前屏幕查找该区域作为稳定参照，供 WorkflowAgent 泛化为后续点击/断言的锚点。'

/** 每个动作的意图示例（面板快捷填充） */
export const INTENT_SUGGESTIONS: Record<RecPending['action'], string[]> = {
  click: ['点击目标元素', '聚焦输入框', '关闭弹窗', '选择菜单项'],
  scroll: ['滚动列表到目标内容', '向下翻页', '回到页面顶部'],
  text: ['输入搜索关键词', '填写表单内容', '输入用户名/账号'],
  hotkey: ['粘贴内容', '全选', '复制', '发送消息'],
  sleep: ['等待页面加载完成', '等待动画结束', '等待数据刷新'],
  region: ['区域锚点：以该 ROI 截图作稳定参照', '标记后续点击可能出现的区域'],
  find_image: ['定位按钮位置', '确认界面元素存在', '以模板查找图标'],
  browser_navigate: ['打开目标网页', '进入系统首页', '访问登录页', '打开搜索结果页'],
  browser_click: ['点击目标元素进入下一步', '点击按钮并确认', '选择页面菜单项'],
  browser_extract: ['获取当前页面主要内容', '读取页面结果/数据', '抓取该页信息供后续使用'],
}
