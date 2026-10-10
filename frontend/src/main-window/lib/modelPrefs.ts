/**
 * modelPrefs — 模型界面「纯前端偏好」的统一读写口。
 *
 * 两类偏好都只落 localStorage，**绝不触发任何 IPC**（尤其不碰 switch_model：
 * 它会写盘 + 插会话系统消息 + 触发一次付费探测，误触成本高）：
 *
 * 1. 精选（眼睛）：`nuphus_visible_models` = `{ [provider]: string[] }`。
 *    某 provider 有非空数组 → 切换弹窗子列表只显示这些；无记录 / 空数组 → 显示全部。
 *    「空 = 全显示」是刻意取舍：避免用户没精选过任何模型时弹窗一片空白（476 个全隐藏），
 *    也让「移出最后一个精选」自然回到全显示，而不是退化成空列表。
 * 2. 厂商默认：`nuphus_default_model_<provider>` = model id 字符串。只记录偏好，
 *    与「点行即切换」相互独立 —— 设为默认不会当场切换模型。
 *
 * 为什么收敛到本模块：组件里散落 `localStorage.getItem('nuphus_visible_models')`
 * 会让「坏数据结构如何降级」这种口径出现多个副本，改一处漏一处。所有读写都经这里，
 * 坏数据一律安全降级为「无记录」（不抛错、不用空数组污染真实语义）。
 */

/** 精选清单的 localStorage key（值为 `{ [provider]: string[] }`）。 */
export const VISIBLE_MODELS_KEY = 'nuphus_visible_models'

/** 厂商默认模型 key 前缀（`nuphus_default_model_<provider>` = model id）。 */
export const DEFAULT_MODEL_KEY_PREFIX = 'nuphus_default_model_'

/** 精选清单：provider → 精选的 model id 列表（只含非空数组）。 */
export type VisibleModelsMap = Record<string, string[]>

/**
 * 读全部精选清单。解析失败 / 非对象 / 值为非数组 → 该项降级为「无记录」：
 * - 坏 JSON、顶层不是普通对象（数组 / 数字 / 字符串 / null）→ 整表视为空（`{}`）
 * - 某 provider 的值不是数组 → 跳过该项（保留其它 provider 的合法数据）
 * - 数组里的非字符串项被过滤；过滤后为空数组 ⇒ 不写入结果（空数组 = 无记录语义）
 */
export function getVisibleModels(storage: Storage | undefined): VisibleModelsMap {
  try {
    const raw = storage?.getItem(VISIBLE_MODELS_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: VisibleModelsMap = {}
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

/**
 * 该 provider 的精选清单；**无记录 / 空清单返回 `null`**（= 语义上的「全显示」）。
 * 调用方需要区分「没精选过」与「精选了但列表为空」时用本函数——
 * 前者应显示全部，后者在本模块里不会出现（空数组写入时即被删除）。
 */
export function getPickedModels(storage: Storage | undefined, provider: string): string[] | null {
  return getVisibleModels(storage)[provider] ?? null
}

/**
 * 该模型是否被显式精选（= 设置页眼睛的「开」态）。
 * 注意：无记录时返回 `false`（没有任何模型被显式精选），但弹窗此时仍显示全部——
 * 「眼睛开态」与「弹窗可见性」是两件事，弹窗可见性用 `getPickedModels` + 空值兜底。
 */
export function isModelVisible(
  storage: Storage | undefined,
  provider: string,
  model: string,
): boolean {
  return (getPickedModels(storage, provider) ?? []).includes(model)
}

/**
 * 设置某模型的精选态。移出最后一个精选时**删除该 provider 的 key**，
 * 使语义回到「无记录 = 全显示」（不是留下空数组让弹窗变空白）。
 */
export function setModelVisible(
  storage: Storage | undefined,
  provider: string,
  model: string,
  visible: boolean,
): void {
  try {
    const map = getVisibleModels(storage)
    const current = map[provider] ?? []
    const next = visible
      ? current.includes(model)
        ? current
        : [...current, model]
      : current.filter(m => m !== model)
    if (next.length > 0) map[provider] = next
    else delete map[provider]

    if (Object.keys(map).length > 0) {
      storage?.setItem(VISIBLE_MODELS_KEY, JSON.stringify(map))
    } else {
      storage?.removeItem(VISIBLE_MODELS_KEY)
    }
  } catch {
    /* localStorage 不可用（隐私模式 / 配额已满）不影响设置页交互 */
  }
}

/** 该 provider 的默认模型 id；未设置 → 空串。 */
export function getDefaultModel(storage: Storage | undefined, provider: string): string {
  try {
    const raw = storage?.getItem(DEFAULT_MODEL_KEY_PREFIX + provider)
    return typeof raw === 'string' ? raw : ''
  } catch {
    return ''
  }
}

/** 设置该 provider 的默认模型；传空串 = 取消默认（删除 key）。只写 localStorage。 */
export function setDefaultModel(
  storage: Storage | undefined,
  provider: string,
  model: string,
): void {
  try {
    if (model) storage?.setItem(DEFAULT_MODEL_KEY_PREFIX + provider, model)
    else storage?.removeItem(DEFAULT_MODEL_KEY_PREFIX + provider)
  } catch {
    /* 同 setModelVisible：失败静默，不影响交互 */
  }
}
