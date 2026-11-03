import type { CanvasTranslate } from './presentation'

/** Stable field identities. Legacy source labels are only lookup aliases, not storage keys. */
const fields: Record<string, [string, string]> = {
  名称: ['/name', 'name'],
  '步骤标识 ID': ['/id', 'id'],
  '步骤标识 ID（有历史记录，修改需确认）': ['/id', 'idHistory'],
  步骤说明: ['/description', 'description'],
  '超时秒数（空为执行器默认）': ['/timeout_secs', 'timeout'],
  重试次数: ['/on_error/retry/max', 'retryCount'],
  '重试间隔（毫秒）': ['/on_error/retry/backoff_ms', 'retryDelay'],
  '允许的退出码（逗号分隔）': ['/on_error/allow_codes/codes', 'exitCodes'],
  工具名: ['/do/tool', 'tool'],
  动作参数: ['/do/with', 'params'],
  '目标工作流 workflow_id': ['/do/call', 'workflow'],
  'with（参数 JSON）': ['/do/with', 'paramsJson'],
  对话内容: ['/do/chat', 'chat'],
  '提示词（任务描述）': ['/do/model', 'modelPrompt'],
  生成类型: ['/do/with/kind', 'modelKind'],
  '选择模型（留空用绑定能力）': ['/do/with/model', 'modelSelect'],
  '系统提示词': ['/do/with/system', 'modelSystem'],
  '参数透传（JSON，逃生舱）': ['/do/with/params', 'modelParamsEscape'],
  画面比例: ['/do/with/ratio', 'modelRatio'],
  生成数量: ['/do/with/count', 'modelCount'],
  时长: ['/do/with/duration', 'modelDuration'],
  首帧图: ['/do/with/first_frame', 'modelFirstFrame'],
  '图片宽度 width': ['/do/with/params', 'modelWidth'],
  '图片高度 height': ['/do/with/params', 'modelHeight'],
  '返回格式': ['/do/with/params', 'modelResponseFormat'],
  '画面清晰度': ['/do/with/params', 'modelResolution'],
  '最长等待时间 timeout_secs': ['/do/with/timeout_secs', 'modelTimeout'],
  '随机程度': ['/do/with/temperature', 'modelTemperature'],
  '最大输出长度': ['/do/with/max_tokens', 'modelMaxTokens'],
  '其他对话选项（JSON）': ['/do/with', 'chatOptions'],
  '脚本代码 code': ['/do/script/code', 'code'],
  失败消息: ['/do/assert/message', 'failureMessage'],
  等待目标: ['/do/wait', 'wait'],
  'MCP 服务器 server': ['/do/mcp/server', 'mcpServer'],
  'MCP 工具 tool': ['/do/mcp/tool', 'mcpTool'],
  '时长（秒）': ['/do/sleep', 'duration'],
  保存输出到变量: ['/capture', 'capture'],
  遍历列表: ['/do/loop/for_each/items', 'items'],
  '当前项变量 as': ['/do/loop/for_each/as', 'item'],
  重复次数: ['/do/loop/repeat', 'repeat'],
  '最多循环次数 max（默认 100）': ['/do/loop/max', 'maxLoop'],
  左侧变量: ['condition.left.var', 'leftVar'],
  右侧变量: ['condition.right.var', 'rightVar'],
  左侧固定值: ['condition.left.literal', 'leftLiteral'],
  右侧固定值: ['condition.right.literal', 'rightLiteral'],
}

export const editorFieldId = (label: string): string => fields[label]?.[0] ?? label
export const editorFieldLabel = (label: string, t: CanvasTranslate): string =>
  fields[label] ? t(`workflowEditor.field.${fields[label][1]}`) : label

export function focusEditorField(panel: Element, path: string): boolean {
  const fields = [...panel.querySelectorAll<HTMLElement>('[data-draft-field], [data-field-path]')]
  // A complex JSON editor may own the nearest enclosing path.
  const field = fields
    .filter(element => {
      const candidate = element.dataset.fieldPath ?? element.dataset.draftField
      return candidate && (path === candidate || path.startsWith(candidate + '/'))
    })
    .sort(
      (a, b) =>
        (b.dataset.fieldPath ?? b.dataset.draftField ?? '').length -
        (a.dataset.fieldPath ?? a.dataset.draftField ?? '').length,
    )[0]
  if (!field) return false
  let details = field.closest('details')
  while (details) {
    details.open = true
    details = details.parentElement?.closest('details') ?? null
  }
  field.scrollIntoView?.({ block: 'nearest' })
  field.querySelector<HTMLElement>('input, textarea, select, button')?.focus()
  return true
}
