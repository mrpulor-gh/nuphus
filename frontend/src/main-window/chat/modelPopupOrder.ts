/**
 * modelPopupOrder — 模型列表「子列表 / 卡片网格」共用的展示排序口径。
 *
 * 痛点：provider 模型多时，每次切换都要在长列表里找模型。这里把最近切换过的
 * 模型排到前面，没有历史的模型保持 `list_models`（= providers.toml）原序接在后面。
 *
 * 两处消费方共用本口径（**同一套语义，不是两份实现**）：
 * - 输入框 Models 弹窗 hover 出的 provider 子列表（ChatPanel）；
 * - 设置页模型卡片网格（ModelsPage），第三参传收藏偏好——
 *   排序即「收藏 > 用过 > 未使用」。
 *
 * 铁律：只影响展示顺序。模型解析、切换、生效判定走 `switch_model` 与
 * `get_provider_context`，均与排序无关——本模块不参与那三条链路，也不读任何
 * 后端状态。
 *
 * 为什么不按「模型设置页的顺序」排：设置页那份列表是 localStorage 的
 * `nuphus_models_<provider>`，而 `addModel`（唯一写入者）仅服务 local 段
 * （见 ModelsPage 注释「仅 local 列表使用」），basic 模型走「连接后点击模型」。
 * 两者不同源，设置页顺序对多数 provider 根本不存在，照它排等于多数情况下空转。
 */

/** localStorage key：某 provider 的最近切换模型序（最近用的在最前）。 */
export const RECENT_MODELS_KEY_PREFIX = 'nuphus_model_recent_'

/** 只留最近 N 个，避免 key 无限增长。 */
export const RECENT_MODELS_MAX = 8

/** 参与排序的最小模型形状（`ModelInfo` 满足）。 */
export type OrderableModel = { id: string }

/**
 * 读最近切换序。解析失败 / 非数组 / 混入非字符串 → 空序（安全降级为原顺序），
 * 绝不让坏数据把弹窗搞崩。
 */
export function readRecentModels(storage: Storage | undefined, provider: string): string[] {
  try {
    const raw = storage?.getItem(RECENT_MODELS_KEY_PREFIX + provider)
    const parsed = raw ? JSON.parse(raw) : []
    if (!Array.isArray(parsed)) return []
    return parsed.filter((v): v is string => typeof v === 'string')
  } catch {
    return []
  }
}

/** 记一次切换：目标模型置顶、去掉重复出现、截断到 RECENT_MODELS_MAX。失败静默。 */
export function rememberRecentModel(
  storage: Storage | undefined,
  provider: string,
  model: string,
): void {
  try {
    const prev = readRecentModels(storage, provider).filter(m => m !== model)
    prev.unshift(model)
    storage?.setItem(
      RECENT_MODELS_KEY_PREFIX + provider,
      JSON.stringify(prev.slice(0, RECENT_MODELS_MAX)),
    )
  } catch {
    /* localStorage 不可用（隐私模式 / 配额已满）不影响切换流程 */
  }
}

/**
 * 子列表排序：**收藏的模型恒定置顶**，其后有历史的按历史位次升序，
 * 无历史的统一排在其后。收藏之间保持入参（= `list_models`）相对顺序。
 *
 * `favorites` 与 `recent` 是两套独立口径：前者是用户在设置页卡片上显式收藏的
 * 模型（`nuphus_favorite_models`），后者是使用痕迹。显式意图优先——否则「收藏」
 * 在弹窗里除了没有作用。收藏**只影响顺序，不过滤**：无收藏时行为与本函数加
 * 参数前完全一致。
 *
 * `Array#sort` 稳定（ES2019 起规范保证），同位次模型保持入参相对顺序，
 * 因此没有收藏也没有历史时，结果与调用方原顺序逐项一致——不是巧合，是排序稳定性的直接推论。
 */
export function orderProviderModels<T extends OrderableModel>(
  models: readonly T[],
  recent: readonly string[],
  favorites?: readonly string[] | null,
): T[] {
  if (recent.length === 0 && !favorites?.length) return [...models]
  const favSet = new Set(favorites ?? [])
  const recentRank = new Map(recent.map((id, i) => [id, i]))
  const rankOf = (m: T) => {
    if (favSet.has(m.id)) return -1
    return recentRank.get(m.id) ?? Number.MAX_SAFE_INTEGER
  }
  return [...models].sort((a, b) => rankOf(a) - rankOf(b))
}
