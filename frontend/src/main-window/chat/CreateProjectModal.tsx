import { useEffect, useState } from 'react'
import { IconFolderPlus } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import { CompactModal } from '../layout/CompactModal'
import './create-project-modal.css'

/** 项目名称长度上限（UI 稿：最多 80 个字符，输入框内直接截断，不报错） */
export const PROJECT_NAME_MAX_LENGTH = 80

interface CreateProjectModalProps {
  open: boolean
  /** 关闭（取消 / Esc / 点遮罩 / 右上角 ✕ 共用；父级置 open=false 并把焦点还给 📁+） */
  onClose: () => void
  /** 「选择项目文件夹」：父级调系统目录选择器（`@tauri-apps/plugin-dialog` 的 open）；
   *  返回所选目录绝对路径，null = 用户取消或选择器不可用（父级已提示，不重复报错） */
  onPickDir: () => Promise<string | null>
  /** 确认创建：父级按「写书签 → 切当前目录 → 生成空对话 → 刷新」执行；
   *  false = 未成功（弹窗保持打开，用户可改路径/改名字重试） */
  onSubmit: (name: string, path: string) => Promise<boolean>
}

/**
 * 「创建项目」弹窗（「项目」行右端 📁+ 的唯一入口）。
 *
 * 与「新建对话」弹窗（NewChatModal）字段与语义都不同，**不是它的分支**，只沿用其
 * 结构/样式约定：`CompactModal` 骨架 + 同前缀字段版式（本文件用 `.cp-*`）。
 * 三条关闭路径（Esc / 取消 / 点遮罩）统一走 `onClose`，且**重开即复位**
 * （名称空 / 未选目录 / 主按钮 disabled）：弹窗常驻挂载、只在 open=false 时不渲染内容，
 * 表单状态必须显式清掉。
 *
 * 数据与副作用都在宿主（SessionRail）：本组件只负责表单状态与表单规则——
 * 两个字段都是必填（UI 稿的红色星号）：**名称空或未选目录 → 「创建项目」禁用**。
 * 已选目录在按钮下方以完整路径展示（UI 稿只给了未选择态，这是选择后的可见反馈）。
 */
export function CreateProjectModal({
  open,
  onClose,
  onPickDir,
  onSubmit,
}: CreateProjectModalProps) {
  const { t } = useLanguage()
  const [name, setName] = useState('')
  const [pickedPath, setPickedPath] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  // 重开复位：名称空 / 未选目录 / 主按钮回到 disabled
  useEffect(() => {
    if (!open) return
    setName('')
    setPickedPath(null)
    setBusy(false)
  }, [open])

  // Esc 关窗：capture 阶段拦下 document 级 Esc —— 抽屉同层也有一条 Esc 监听（收起抽屉），
  // 弹窗在场时只关弹窗、不连带收起抽屉（关窗后焦点还要回到 📁+）。
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

  const canSubmit = name.trim().length > 0 && !!pickedPath && !busy

  const handlePick = async () => {
    if (busy) return
    const picked = await onPickDir()
    if (picked) setPickedPath(picked)
  }

  const handleSubmit = async () => {
    if (!canSubmit || !pickedPath) return
    setBusy(true)
    try {
      await onSubmit(name.trim(), pickedPath)
    } finally {
      setBusy(false)
    }
  }

  return (
    <CompactModal
      open={open}
      onClose={onClose}
      title={t('sessionRail.createProjectTitle')}
      icon={<IconFolderPlus size={14} />}
      size="sm"
      className="compact-modal--fit cp-modal"
      footer={
        <>
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
            className="btn cp-btn-primary"
            onClick={() => void handleSubmit()}
            disabled={!canSubmit}
            title={t('sessionRail.createProjectSubmit')}
          >
            {t('sessionRail.createProjectSubmit')}
          </button>
        </>
      }
    >
      <div className="cp-body">
        <label className="cp-field">
          <span className="cp-lb">
            {t('sessionRail.createProjectName')}
            <span className="cp-req" aria-hidden="true">
              *
            </span>
          </span>
          <input
            className="cp-input"
            value={name}
            maxLength={PROJECT_NAME_MAX_LENGTH}
            aria-label={t('sessionRail.createProjectName')}
            autoComplete="off"
            spellCheck={false}
            onChange={e => setName(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter') void handleSubmit()
            }}
          />
          <span className="cp-helper">{t('sessionRail.createProjectNameHint')}</span>
        </label>

        <div className="cp-field">
          <span className="cp-lb">
            {t('sessionRail.createProjectSource')}
            <span className="cp-req" aria-hidden="true">
              *
            </span>
          </span>
          <button
            type="button"
            className="cp-pick"
            onClick={() => void handlePick()}
            disabled={busy}
            title={t('sessionRail.createProjectPick')}
          >
            {t('sessionRail.createProjectPick')}
          </button>
          {/* 选择后的可见反馈：完整路径（UI 稿只给了未选择态） */}
          {pickedPath && (
            <span className="cp-picked" title={pickedPath}>
              {pickedPath}
            </span>
          )}
        </div>
      </div>
    </CompactModal>
  )
}
