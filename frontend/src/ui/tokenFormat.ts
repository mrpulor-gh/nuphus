/**
 * token 数值显示（整个前端唯一实现，桌面 / 移动共用）。
 *
 * ## 为什么不再用「1 位小数」
 * 上下文窗口 1,048,576 被压成 `1.0M`，分母凭空少 48,576 tokens：用户拿显示值
 * 手算占比（319.1k / 1.0M ≈ 31.9%）与系统按真实窗口算出的百分比（30%）对不上，
 * 界面自相矛盾。故 M 档保留两位小数并去掉尾零：1,048,576 → `1.05M`，整百万 → `1M`。
 *
 * ## 两种口径，按场景选
 * - `formatTokens`：紧凑缩写，用于状态栏 / 气泡 / 移动端等**读数位**（要一眼可比）；
 * - `formatExactTokens`：千分位真实值，用于**设置类界面**（模型页等需要精确校准的地方）。
 */

/** 去掉小数尾零：`1.50` → `1.5`、`100.00` → `100`、`1.05` 原样。 */
function trimTrailingZero(s: string): string {
  if (!s.includes('.')) return s
  return s.replace(/0+$/, '').replace(/\.$/, '')
}

/** 紧凑 token 数：1,048,576 → `1.05M`；319,100 → `319.1k`；860 → `860`。 */
export function formatTokens(n: number): string {
  const v = Number.isFinite(n) ? n : 0
  const abs = Math.abs(v)
  if (abs >= 1_000_000) return `${trimTrailingZero((v / 1_000_000).toFixed(2))}M`
  if (abs >= 1_000) {
    const k = v / 1_000
    // 999,999 这类值按两位小数会被舍成 1000.00k —— 提升到 M 档，不出 `1000k`
    if (Math.abs(k) >= 999.995) return `${trimTrailingZero((v / 1_000_000).toFixed(2))}M`
    return `${trimTrailingZero(k.toFixed(2))}k`
  }
  return String(Math.round(v))
}

/** 精确 token 数（千分位，不缩写）：1,048,576 → `1,048,576`。 */
export function formatExactTokens(n: number): string {
  const v = Number.isFinite(n) ? Math.round(n) : 0
  return v.toLocaleString()
}
