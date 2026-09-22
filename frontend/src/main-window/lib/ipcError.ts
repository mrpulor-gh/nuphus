/**
 * IPC 错误 → 可展示文案。
 *
 * bridge 抛出的形态是 `IPC invoke <cmd> failed: <原因>`：直接展示会把命令名与
 * 平台实现细节泄露到界面上（例如「Command get_project_dir not found」），对用户
 * 没有任何可操作性。这里统一做三件事：
 *
 * 1. 有可读原因 → 只取原因部分（去掉 `IPC invoke xxx failed:` 前缀）；
 * 2. 命令未注册（前端已更新、后端未重启）→ 给出「重启应用」的可操作指引；
 * 3. 原因本身不可读（透传的平台错误）→ 使用调用方提供的兜底文案。
 *
 * 注：`MobilePage.tsx` 内有一个同类实现（`cleanIpcError`，仅去前缀）。后续统一
 * 到本模块时需同时改其调用点，当前保持不动以免波及该页文案。
 */
export function friendlyIpcError(e: unknown, fallback = '操作失败，请稍后重试'): string {
  const raw = e instanceof Error ? e.message : String(e)
  const reason = raw.match(/failed:\s*(.+)$/)?.[1]?.trim() ?? raw

  // 前端命令已更新、后端仍是旧进程：重启未必够（二进制本身没有该命令时，
  // 重启多少次都无效）→ 文案必须同时覆盖「先重启」与「重启无效则需重新构建」。
  //
  // ⚠️ 判定必须**限定在命令上下文**：这里历史上是裸 `/not found/`，于是后端业务
  // 错误（`model 'x' not found for provider 'y'`）也被吞成「版本过旧」，把排查
  // 方向整体带偏——归因错误的代价远大于文案难看。
  // 注意排除 HTTP 状态码：`404 Not Found` 是地址问题，不是命令未注册（两者都含
  // "not found"，若不加排除会被误判成版本问题，把用户引向错误的方向）。
  if (
    /Command\s+\S+\s+(not\s+found|not\s+allowed)|unknown command/i.test(reason) &&
    !/\b(4\d\d|5\d\d)\b/.test(reason)
  ) {
    return '应用后端版本过旧，未包含该功能：请先重启应用；若仍报错，需重新构建应用后再启动'
  }

  // 原因缺失或仍是原始 IPC 文本 → 兜底文案（细节留在 console）
  if (!reason.trim() || /IPC invoke/i.test(reason)) return fallback

  // ── 连接类失败：地址 / 网络 / 鉴权 / 协议不匹配是最常见的四种 ──
  // 原文（如 `error sending request for url (https://x/v1/models)`）只有开发者能读，
  // 对用户零可操作性 → 逐类给出「下一步该做什么」，细节仍留在 console。
  if (
    /dns error|failed to lookup address|error sending request|tcp connect error|connection refused|connection reset|connection closed|timed?\s?out/i.test(
      reason,
    )
  ) {
    return '无法连接该接口地址：请检查地址填写是否正确、网络是否可达'
  }
  if (/\b401\b|unauthorized|invalid api key|incorrect api key|authentication/i.test(reason)) {
    return 'API Key 无效或被拒绝，请检查密钥是否正确'
  }
  if (/\b403\b|forbidden/i.test(reason)) {
    return '该 API Key 无权访问此接口，请确认密钥权限'
  }
  if (/\b404\b/i.test(reason)) {
    return '接口地址不存在：请确认 URL 是否正确（常见原因是漏写或多写 /v1）'
  }
  if (/\b429\b|rate ?limit/i.test(reason)) {
    return '请求过于频繁，请稍后重试'
  }
  if (/\b50[0-9]\b|internal server error|bad gateway|service unavailable/i.test(reason)) {
    return '接口服务端异常，请稍后重试或联系服务商'
  }
  if (/expected value|expected ident|invalid json|EOF while parsing/i.test(reason)) {
    return '该地址返回的不是模型列表：请确认它提供 OpenAI 兼容的 /v1/models 接口'
  }

  return reason
}
