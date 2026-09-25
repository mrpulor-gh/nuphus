import { useLanguage } from '../../locales'

const messages = {
  title: ['完整运行记录', 'Execution details'],
  history: ['从历史运行选择字段', 'Choose a field from a past run'],
  historical: [
    '字段来自所选历史运行，之后的运行可能不同。',
    'Fields come from the selected past run; future values may differ.',
  ],
  mode: ['运行类型', 'Run type'],
  debug: ['调试运行', 'Debug runs'],
  normal: ['正式运行', 'Normal runs'],
  run: ['运行', 'Run'],
  invocation: ['步骤调用', 'Invocation'],
  revision: ['定义版本', 'Definition revision'],
  duration: ['耗时', 'Duration'],
  pendingDuration: ['尚未结束', 'Not finished'],
  refresh: ['刷新', 'Refresh'],
  loading: ['正在读取运行记录…', 'Loading execution evidence…'],
  empty: ['暂无可查看的真实运行记录。', 'No recorded executions are available.'],
  emptyInvocations: ['该运行尚无此步骤的调用记录。', 'This run has no invocation for this step.'],
  failed: ['无法读取运行记录', 'Could not read execution evidence'],
  unavailable: ['没有可用的运行详情。', 'Execution details are unavailable.'],
  output: ['完整输出', 'Full output'],
  inputs: ['实际输入', 'Actual inputs'],
  before: ['执行前变量', 'Variables before'],
  after: ['执行后变量', 'Variables after'],
  attempts: ['重试记录', 'Attempts'],
  verification: ['效果验证', 'Effect verification'],
  definition: ['当时的步骤定义', 'Step definition at execution'],
  unverified: [
    '未记录效果验证；执行完成不代表目标效果已验证。',
    'No effect verification was recorded; completion does not establish that the intended effect occurred.',
  ],
  error: ['错误', 'Error'],
  noOutput: ['未记录输出。', 'No output was recorded.'],
  fields: ['真实字段', 'Observed fields'],
  noFields: [
    '所选快照中没有当前作用域可引用的变量。',
    'This snapshot has no variables available in the current scope.',
  ],
  choose: ['插入引用', 'Insert reference'],
  more: ['显示更多字段', 'Show more fields'],
  copy: ['复制完整内容', 'Copy full content'],
  copied: ['已复制', 'Copied'],
  copyFailed: [
    '复制失败，可在内容区选择并复制。',
    'Copy failed. Select and copy the content directly.',
  ],
  download: ['下载完整记录', 'Download full record'],
  close: ['关闭', 'Close'],
  parent: ['父调用', 'Parent invocation'],
  success: ['执行完成', 'Completed'],
  running: ['执行中', 'Running'],
  interrupted: ['已中断', 'Interrupted'],
  timeout: ['已超时', 'Timed out'],
  failedStatus: ['失败', 'Failed'],
  skipped: ['已跳过', 'Skipped'],
  paused: ['已暂停', 'Paused'],
  cancelled: ['已取消', 'Cancelled'],
  completed_with_skips: ['执行结束，存在跳过步骤', 'Completed with skipped steps'],
} as const

export function useTraceText() {
  const { lang } = useLanguage()
  return (key: keyof typeof messages) => messages[key][lang.startsWith('zh') ? 0 : 1]
}
