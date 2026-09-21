/**
 * 自定义中转站实例（Custom provider instance）命名契约 —— 前端侧。
 *
 * 一个自定义中转站 = providers.toml 里一个独立配置段：
 *   name = "custom-xxx"（实例身份，全局唯一）
 *   provider_type = "custom"（协议类型，固定不变）
 *
 * 同名模型（如两个中转站都提供 gpt-4o）靠「实例名 + 模型 ID」精确路由，
 * 因此实例名必须稳定、唯一、且是纯 ASCII —— 否则后端段查找会退化为
 * 「首个同名命中」，请求就打到了错误的中转站。
 * 规则与后端 `validate_custom_provider_name` 保持一致，两端必须同步修改。
 */

/** 旧版单实例段名：仍被兼容（迁移前的老配置），但不可用于新建。 */
export const LEGACY_CUSTOM_PROVIDER_ID = 'custom'

/**
 * 校验新建实例名是否合法：`custom-xxx`，小写英文/数字，段间用单连字符，
 * 长度 ≤ 64，不允许空后缀或以连字符结尾。
 */
export function isValidCustomInstanceId(id: string): boolean {
  return id.length <= 64 && /^custom-[a-z0-9]+(-[a-z0-9]+)*$/.test(id)
}

/** 判断配置段名或 ProviderInfo 是否属于自定义中转站（含旧版 `custom`）。 */
export function isCustomProviderId(id: string): boolean {
  return id === LEGACY_CUSTOM_PROVIDER_ID || id.startsWith('custom-')
}

/**
 * 把用户填写的「自定义名称」规格化为段 id：`custom-<slug>`。
 *
 * 用户填的是显示名（可以是中文），段名必须是稳定唯一的纯 ASCII —— 后端
 * `validate_custom_provider_name` 只接受 `custom-<小写英文/数字/连字符>`，
 * 且同名模型靠「实例名 + 模型 ID」精确路由。因此这里：
 *   转小写 → 非 [a-z0-9] 字符转 `-` → 合并连续 `-` → 去首尾 `-` →
 *   截断到 55 字符（`custom-` 7 字符 + 55 ≤ 后端 64 上限）
 * slug 为空（纯中文等最常见情况）或与已有实例重名时，退化为 `custom-<unix 秒>`；
 * 同一秒内重复创建再追加 `-<序号>`，保证不重名。
 *
 * 界面永远只显示用户填写的名称（段 id 只在 title 属性里作调试信息）。
 */
export function buildCustomInstanceId(displayName: string, existingIds: string[]): string {
  const taken = new Set(existingIds)
  const slug = displayName
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 55)
    .replace(/-+$/g, '')
  const fromSlug = slug ? `custom-${slug}` : ''
  if (fromSlug && isValidCustomInstanceId(fromSlug) && !taken.has(fromSlug)) {
    return fromSlug
  }
  const stamp = Math.floor(Date.now() / 1000)
  let id = `custom-${stamp}`
  let seq = 1
  while (taken.has(id)) {
    seq += 1
    id = `custom-${stamp}-${seq}`
  }
  return id
}
