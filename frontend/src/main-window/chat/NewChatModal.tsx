import { useEffect, useState } from 'react'
import { IconFolder, IconFolderPlus } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import { CompactModal } from '../layout/CompactModal'
import './new-chat-modal.css'

/** 会话标题长度上限：比后端 rename_session_cmd 的 60 字上限留出余量，弹窗内直接截断（不报错） */
const TITLE_MAX_LENGTH = 40

/**
 * 归属项目选项：只取渲染所需的「名称 + 目录路径」两段事实。
 * `ShelfProjectEntry`（展示台 projects[]）与 `ProjectBookmark`（书签读写返回）都可直接传入。
 */
export interface NewChatProjectOption {
  name: string
  path: string
}

interface NewChatModalProps {
  open: boolean
  /** 可选归属项目（未归档 projects[]，顺序即列表顺序；浏览新建的目录由父级追加书签后并入） */
  projects: NewChatProjectOption[]
  /** 关闭（取消 / Esc / 点遮罩三条路径共用；父级置 open=false 并把焦点还给入口行） */
  onClose: () => void
  /** 「浏览本地目录…」：父级调系统目录选择器（未知目录先追加书签）→ 可归属选项；null = 取消/失败 */
  onBrowseDir: () => Promise<NewChatProjectOption | null>
  /** 确认创建：父级按「切目录 → 记录标题并回欢迎页 → 刷新列表」执行；false = 未成功
   *  （弹窗保持打开，可改选）。会话本身在欢迎页直发首条消息那一刻才诞生，此处只是
   *  把「标题 + 归属」记录下来，因此父级成功返回时列表里**不会**立刻多出会话卡。 */
  onCreate: (title: string, project: NewChatProjectOption) => Promise<boolean>
}

/**
 * 新建对话弹窗（会话工作台入口动作行 → 弹窗）。
 *
 * 会话的创建路径**必须显式**存在，但创建过程**不得混进项目目录**——所以入口是列表首位的
 * 动作行，点开是弹窗：标题 + 归属项目。确认 = **记录**这次选择（标题 + 归属），会话本身仍
 * 在欢迎页直发首条消息那一刻诞生，随后带着该标题落到所选项目分组的会话列表。
 *
 * 三条关闭路径（Esc / 取消 / 点遮罩）统一走 `onClose`，且**重开即复位**（标题空 / 无选中 /
 * 主按钮 disabled）：弹窗常驻挂载、只在 open=false 时不渲染内容，表单状态必须显式清掉。
 *
 * 数据与副作用都在宿主（SessionRail）：本组件只负责表单状态与表单规则
 * （标题空或未选项目 → 主按钮 disabled；选中后底部 hint 显示所选目录完整路径）。
 */
export function NewChatModal({
  open,
  projects,
  onClose,
  onBrowseDir,
  onCreate,
}: NewChatModalProps) {
  const { t } = useLanguage()
  const [title, setTitle] = useState('')
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  /** 浏览返回、尚未出现在 projects[] 的目录：书签落盘与列表刷新的间隙里也要能立刻选中 */
  const [browsed, setBrowsed] = useState<NewChatProjectOption | null>(null)
  const [busy, setBusy] = useState(false)

  // 重开复位：标题空 / 无选中项 / 主按钮回到 disabled
  useEffect(() => {
    if (!open) return
    setTitle('')
    setSelectedPath(null)
    setBrowsed(null)
    setBusy(false)
  }, [open])

  // Esc 关窗：capture 阶段拦下 document 级 Esc —— 抽屉同层也有一条 Esc 监听（收起抽屉），
  // 弹窗在场时只关弹窗、不连带收起抽屉（关窗后焦点还要回到入口行）。
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.stopPropagation()
      onClose()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [open, onClose])

  const options =
    browsed && !projects.some(p => p.path === browsed.path) ? [...projects, browsed] : projects
  const selected = options.find(p => p.path === selectedPath) ?? null
  const canCreate = !!selected && title.trim().length > 0 && !busy

  const handleBrowse = async () => {
    if (busy) return
    const picked = await onBrowseDir()
    if (!picked) return
    setBrowsed(picked)
    setSelectedPath(picked.path)
  }

  const handleCreate = async () => {
    if (!canCreate || !selected) return
    setBusy(true)
    try {
      await onCreate(title.trim(), selected)
    } finally {
      setBusy(false)
    }
  }

  return (
    <CompactModal
      open={open}
      onClose={onClose}
      title={t('sessionRail.newChat')}
      icon={<IconFolderPlus size={14} />}
      size="sm"
      className="compact-modal--fit nc-modal"
      footer={
        <>
          <span className="nc-hint">{selected ? selected.path : ''}</span>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={onClose}
            disabled={busy}
            title={t('common.cancel')}
          >
            {t('common.cancel')}
          </button>
          <button
            type="button"
            className="btn nc-btn-primary"
            onClick={() => void handleCreate()}
            disabled={!canCreate}
            title={t('sessionRail.newChatCreate')}
          >
            {t('sessionRail.newChatCreate')}
          </button>
        </>
      }
    >
      <div className="nc-body">
        <label className="nc-field">
          <span className="nc-lb">{t('sessionRail.newChatTitleLabel')}</span>
          <input
            className="nc-input"
            value={title}
            autoFocus
            maxLength={TITLE_MAX_LENGTH}
            placeholder={t('sessionRail.newChatTitlePlaceholder')}
            aria-label={t('sessionRail.newChatTitleLabel')}
            autoComplete="off"
            spellCheck={false}
            onChange={e => setTitle(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter') void handleCreate()
            }}
          />
        </label>
        <div className="nc-field">
          <span className="nc-lb">{t('sessionRail.newChatProjectLabel')}</span>
          <div className="nc-opts" role="listbox" aria-label={t('sessionRail.newChatProjectLabel')}>
            {options.map(p => (
              <button
                key={p.path}
                type="button"
                className="nc-opt"
                role="option"
                aria-selected={p.path === selectedPath}
                title={p.path}
                onClick={() => setSelectedPath(p.path)}
              >
                <span className="nc-opt-ic" aria-hidden="true">
                  <IconFolder size={14} />
                </span>
                <span className="nc-opt-body">
                  <span className="nc-opt-name">{p.name}</span>
                  <span className="nc-opt-path">{p.path}</span>
                </span>
              </button>
            ))}
            {options.length > 0 && <span className="nc-sep" aria-hidden="true" />}
            <button
              type="button"
              className="nc-opt"
              role="option"
              aria-selected={false}
              onClick={() => void handleBrowse()}
            >
              <span className="nc-opt-ic" aria-hidden="true">
                <IconFolderPlus size={14} />
              </span>
              <span className="nc-opt-body">
                <span className="nc-opt-name">{t('sessionRail.newChatBrowse')}</span>
              </span>
            </button>
          </div>
        </div>
      </div>
    </CompactModal>
  )
}
