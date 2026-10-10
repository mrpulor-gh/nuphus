/**
 * modelPopupOrder — 输入框 Models 弹窗「提供商子列表」的展示口径（可见范围 + 排序）。
 *
 * 可见范围：`filterProviderModels` 按设置页的「精选」清单收窄子列表（空清单 = 全显示）。
 * 排序痛点：provider 模型多时，每次切换都要在长列表里找模型。这里把最近切换过的
 * 模型排到前面，没有历史的模型保持 `list_models`（= providers.toml）原序接在后面。
 *
 * 铁律：只影响弹窗里的展示（可见范围与顺序）。模型解析、切换、生效判定走
 * `switch_model` 与 `get_provider_context`，均与展示无关——本模块不参与那三条链路，
 * 也不读任何后端状态。
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
 * 子列表排序：**厂商默认模型恒定置顶**，其后有历史的按历史位次升序，
 * 无历史的统一排在其后。
 *
 * `pinned` 与「最近使用」是两套独立口径：前者是用户在模型设置页显式指定的
 * 厂商默认（`nuphus_default_model_<provider>`），后者是使用痕迹。显式意图优先——
 * 否则「设为默认」在弹窗里除了卡片上那行小字之外没有任何作用点。
 *
 * `Array#sort` 稳定（ES2019 起规范保证），同位次模型保持入参（= `list_models`）相对顺序，
 * 因此没有 pinned 也没有历史时，结果与调用方原顺序逐项一致——不是巧合，是排序稳定性的直接推论。
 */
export function orderProviderModels<T extends OrderableModel>(
  models: readonly T[],
  recent: readonly string[],
  pinned?: string,
): T[] {
  if (recent.length === 0 && !pinned) return [...models]
  const rank = new Map(recent.map((id, i) => [id, i]))
  const rankOf = (m: T) => {
    if (pinned && m.id === pinned) return -1
    return rank.get(m.id) ?? Number.MAX_SAFE_INTEGER
  }
  return [...models].sort((a, b) => rankOf(a) - rankOf(b))
}

/**
 * 子列表可见范围：`visible` 有非空精选清单时只留清单内的模型；否则（null / 空 /
 * 未提供）原样返回全部。
 *
 * 「空 = 全显示」是锁定语义，不是兜底疏漏：用户没精选过任何模型时若按空清单过滤，
 * 弹窗会一片空白（该 provider 全被隐藏），而设置页并**不**按精选过滤，用户还能把
 * 模型加回来——但弹窗空白本身已是坏体验。清单里指向已不存在模型的项无害忽略
 * （过滤即天然忽略），不影响其余模型展示。
 */
export function filterProviderModels<T extends OrderableModel>(
  models: readonly T[],
  visible: readonly string[] | null | undefined,
): T[] {
  if (!visible || visible.length === 0) return [...models]
  const allow = new Set(visible)
  return models.filter(m => allow.has(m.id))
}
