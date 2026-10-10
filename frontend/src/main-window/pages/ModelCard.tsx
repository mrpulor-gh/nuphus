/**
 * ModelCard — 模型列表的卡片单元（设置页「模型」区，custom 段与 local 段共用）。
 *
 * 卡片三行，语义从左到右、从上到下固定：
 *   1. 首行：模型名（粗体）+ 手动/GO 徽章 + 收藏星形（右下角落删除）
 *   2. 能力行：svg + 文字 chip，**支持的能力在前（重色），不支持的在后（浅色）**；
 *      chip 分两种态度——可切换（vision / 图像视频生成，用户可手动纠正探测结论）
 *      与只读（自动探测写入，暂无手改入口），affordance 必须能区分。
 *   3. 底行：上下文容量（可编辑）+ 「启用」按钮（当前模型显示「使用中」）。
 *
 * 铁律（数据客观性）：
 * - chip 只呈现后端事实（supports_* / reasoning_efforts），可切换项也只写
 *   `set_model_supports_*`（user 来源）——不新增、不猜测任何能力字段；
 * - 「启用」是显式动作：切换模型会写盘 + 往会话插系统消息 + 触发一次付费探测，
 *   误触成本高，因此**整卡不承担点击切换**，只有这一个按钮触发。
 */
import { IconStar, IconEye, IconImage, IconMic, IconTrash2 } from '../../ui/Icons'
import { RowCtxEditor } from './RowCtxEditor'

/** 能力 chip 的呈现数据（label / hint 由调用方按当前语言备好）。 */
export type ModelCardCap = {
  key: string
  label: string
  /** 是否具备该能力（后端事实；未知能力由调用方直接不进数组） */
  on: boolean
  /** 是否可点击切换（false = 只读 chip，无按钮语义） */
  toggleable: boolean
  /** hover 说明（含数据来源 / 切换后果） */
  hint: string
  /** 附加数值说明（如「3 档」）；未具备时不展示 */
  detail?: string
  /** 切换中（按钮 busy 态） */
  busy?: boolean
}

const CAP_ICON: Record<string, typeof IconEye> = {
  vision: IconEye,
  imageVideo: IconImage,
  audio: IconMic,
  reasoning: IconEye,
}

/** 卡片所需文案（由调用方按当前语言备好，组件本身不识 i18n）。 */
export type ModelCardTxt = {
  favAria: (name: string) => string
  favHintOn: string
  favHintOff: string
  goTitle: string
  manualTitle: string
  removeTitle: string
  /** 非当前模型的动作文案：「切换」——动作是切换，不是「启用」 */
  switchLabel: string
  /** 当前生效模型的态文案（按钮仍可点：改了连接参数后点它=用新参数重新落盘） */
  usingLabel: string
  usingHint: string
  switchingLabel: string
}

export function ModelCard({
  name,
  active,
  switching,
  favorite,
  manual,
  go,
  caps,
  ctx,
  showRemove,
  txt,
  onToggleFavorite,
  onToggleCap,
  onUse,
  onRemove,
  ctxEdit,
}: {
  name: string
  active: boolean
  switching: boolean
  favorite: boolean
  manual?: boolean
  go?: boolean
  caps: ModelCardCap[]
  ctx?: number
  showRemove?: boolean
  txt: ModelCardTxt
  onToggleFavorite: () => void
  onToggleCap: (key: string) => void
  onUse: () => void
  onRemove?: () => void
  ctxEdit: {
    isEditing: boolean
    value: string
    onValueChange: (v: string) => void
    onStart: () => void
    onCommit: (raw: string) => void
    onCancel: () => void
    t: (key: string, ...args: string[]) => string
  }
}) {
  return (
    <div
      className={`model-card${active ? ' is-active' : ''}`}
      aria-busy={switching || undefined}
      data-testid={`model-card-${name}`}
    >
      <div className="model-card-head">
        <span className="model-card-name">{name}</span>
        {go && (
          <span className="model-card-go" title={txt.goTitle}>
            GO
          </span>
        )}
        {manual && (
          <span className="model-card-manual" title={txt.manualTitle}>
            手动
          </span>
        )}
        <span className="model-card-head-actions">
          <button
            type="button"
            className={`model-fav-btn${favorite ? ' is-on' : ''}`}
            aria-pressed={favorite}
            aria-label={txt.favAria(name)}
            title={favorite ? txt.favHintOn : txt.favHintOff}
            onClick={e => {
              e.stopPropagation()
              onToggleFavorite()
            }}
          >
            <IconStar size={12} />
          </button>
          {showRemove && onRemove && (
            <button
              type="button"
              className="icon-btn-ghost model-card-del"
              aria-label={txt.removeTitle}
              title={txt.removeTitle}
              onClick={e => {
                e.stopPropagation()
                onRemove()
              }}
            >
              <IconTrash2 size={12} />
            </button>
          )}
        </span>
      </div>

      <div className="model-card-caps">
        {caps.map(cap => {
          const Icon = CAP_ICON[cap.key] ?? IconEye
          const chip = (
            <>
              <Icon size={11} />
              <span className="cap-chip-label">{cap.label}</span>
              {cap.on && cap.detail && <span className="cap-chip-detail">{cap.detail}</span>}
            </>
          )
          if (!cap.toggleable) {
            return (
              <span
                key={cap.key}
                className={`cap-chip ${cap.on ? 'is-on' : 'is-off'} is-readonly`}
                title={cap.hint}
              >
                {chip}
              </span>
            )
          }
          return (
            <button
              key={cap.key}
              type="button"
              className={`cap-chip ${cap.on ? 'is-on' : 'is-off'}`}
              aria-pressed={cap.on}
              disabled={cap.busy}
              title={cap.hint}
              onClick={e => {
                e.stopPropagation()
                onToggleCap(cap.key)
              }}
            >
              {chip}
            </button>
          )
        })}
      </div>

      <div className="model-card-foot">
        <span className="model-card-ctx">
          <RowCtxEditor
            ctx={ctx}
            isEditing={ctxEdit.isEditing}
            value={ctxEdit.value}
            onValueChange={ctxEdit.onValueChange}
            onStart={ctxEdit.onStart}
            onCommit={() => ctxEdit.onCommit(name)}
            onCancel={ctxEdit.onCancel}
            t={ctxEdit.t}
          />
        </span>
        <button
          type="button"
          className="model-use-btn"
          disabled={switching}
          aria-busy={switching || undefined}
          title={active ? txt.usingHint : undefined}
          onClick={e => {
            e.stopPropagation()
            onUse()
          }}
        >
          {active ? txt.usingLabel : switching ? txt.switchingLabel : txt.switchLabel}
        </button>
      </div>
    </div>
  )
}
