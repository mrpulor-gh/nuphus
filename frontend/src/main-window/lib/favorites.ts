/**
 * favorites — 模型「收藏」本地偏好（设置页卡片 + 输入框弹窗共用）。
 *
 * 与 PR #97 的 `nuphus_visible_models`（精选过滤）是两件事：收藏**只影响排序**，
 * 不改变任何列表的可见范围——切换弹窗 hover 某 provider 时收藏的模型置顶，
 * 其余照旧全部显示。偏好只落 localStorage，绝不触发任何 IPC。
 *
 * 坏数据安全降级为「无记录」（不抛错、不用空数组污染语义）：
 * - 坏 JSON / 顶层非对象 → 整表视为空
 * - 某 provider 的值非数组 → 跳过该项；数组内非字符串被过滤
 * - 过滤后为空 ⇒ 不写入结果（空数组 = 无记录语义）
 * - 移出最后一个收藏 ⇒ 删除该 provider 的 key
 */

/** 收藏清单的 localStorage key（值为 `{ [provider]: string[] }`）。 */
export const FAVORITE_MODELS_KEY = 'nuphus_favorite_models'

/** 收藏清单：provider → 收藏的 model id 列表（只含非空数组）。 */
export type FavoriteModelsMap = Record<string, string[]>

/**
 * 读全部收藏清单。解析失败 / 非对象 / 值为非数组 → 该项降级为「无记录」。
 */
export function getFavorites(storage: Storage | undefined): FavoriteModelsMap {
  try {
    const raw = storage?.getItem(FAVORITE_MODELS_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: FavoriteModelsMap = {}
    for (const [provider, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (!Array.isArray(value)) continue
      const ids = value.filter((v): v is string => typeof v === 'string')
      if (ids.length > 0) out[provider] = ids
    }
    return out
  } catch {
    return {}
  }
}

/** 该 provider 的收藏清单；无记录 / 空清单返回 `null`（= 无收藏，排序不置顶）。 */
export function getProviderFavorites(
  storage: Storage | undefined,
  provider: string,
): string[] | null {
  return getFavorites(storage)[provider] ?? null
}

/** 该模型是否被收藏（卡片右上角星形开态）。 */
export function isFavorite(storage: Storage | undefined, provider: string, model: string): boolean {
  return (getProviderFavorites(storage, provider) ?? []).includes(model)
}

/**
 * 设置某模型的收藏态。移出最后一个收藏时**删除该 provider 的 key**
 * （不留空数组——调用方靠「无记录」判断无需置顶）。
 */
export function setFavorite(
  storage: Storage | undefined,
  provider: string,
  model: string,
  favorite: boolean,
): void {
  try {
    const map = getFavorites(storage)
    const current = map[provider] ?? []
    const next = favorite
      ? current.includes(model)
        ? current
        : [...current, model]
      : current.filter(m => m !== model)
    if (next.length > 0) map[provider] = next
    else delete map[provider]

    if (Object.keys(map).length > 0) {
      storage?.setItem(FAVORITE_MODELS_KEY, JSON.stringify(map))
    } else {
      storage?.removeItem(FAVORITE_MODELS_KEY)
    }
  } catch {
    /* localStorage 不可用（隐私模式 / 配额已满）不影响卡片交互 */
  }
}
