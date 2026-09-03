/**
 * RecorderBar.tsx — 录制控制面板（画布 header「录制」入口打开；大王反馈#6 改为 ~200px 竖排「录制框」）
 *
 * 视觉/拖动复用 DesktopToolbar（Ctrl+U 浮条）模式，独立 CSS（rec-* 前缀），
 * 不侵入 DesktopToolbar 现有行为。按钮用词铁律：开始/取消/完成/保存进度/草稿/清空草稿，
 * 绝不出现「停止」。
 *
 * 布局：宽 ~200px 竖排三区——
 *  - 上栏：会话（标题「录制」/状态点/步数/将插入位置提示 + 顶部拖动把手）
 *  - 中栏：动作按钮（10 动作纵排：7 桌面 + 3 网页流程，超出自动滚动）
 *  - 下栏：操作（保存进度 / 草稿 N / 取消 / 完成）
 * 默认放画布容器左上 (60,80)，用户可拖动；位置记忆沿用 nuphus_recorder_pos（超界自动 clamp）。
 *
 * 进度持久化 + 草稿：
 * - idle 态「保存进度」：收集整体备注 → recSavePending 落盘 pending → 会话结束（下次 begin 恢复）
 * - idle 态「草稿」：打开 RecDraftPanel 查看/编辑/删除/清空本会话草稿（删除联动画布节点）
 */

import { useEffect, useRef, useState } from 'react'
import {
  MousePointerClick,
  Mouse,
  Type,
  Keyboard,
  Hourglass,
  Scan,
  FileImage,
  Globe,
  MousePointer2,
  FileText,
  GripVertical as IconGrip,
  Save,
  ListOrdered,
} from 'lucide-react'
import type { RecAction, RecDraft, RecStatus } from './recorderTypes'
import { ACTION_LABEL } from './recorderTypes'
import { RecDraftPanel } from './RecDraftPanel'
import type { RecDraftPatch } from './useWorkflowRecorder'
import { playUiSound } from '../../../ui/sound'
import './recorder.css'

interface RecorderBarProps {
  /** pending 时浮层隐藏，仅 idle/capturing 渲染 */
  status: Extract<RecStatus, 'idle' | 'capturing'>
  action: RecAction | null
  /** 本会话已确认草稿（含 canvas_step_id 绑定）；面板/计数徽标/保存进度 enabled 均依赖它 */
  drafts: RecDraft[]
  /** 下一确认步骤会插入的位置文案（「X」之后 / 层末尾）；缺省回退「层末尾」 */
  insertTargetLabel?: string | null
  onPick: (action: RecAction) => void
  onCancelCapture: () => void
  onAbort: () => void
  onComplete: (notes: string) => void
  onSaveProgress: (notes: string) => void
  onEditDraft: (index: number, patch: RecDraftPatch) => void
  onDeleteDraft: (index: number) => void
  onClearDrafts: () => void
}

interface ActionDef {
  action: RecAction
  icon: React.ElementType
  hint: string
}

const ACTIONS: ActionDef[] = [
  // ── 网页组（顶部）：目标 = Nuphus managed 浏览器（CDP 管理，可见窗口）──
  {
    action: 'browser_navigate',
    icon: Globe,
    hint: '在浏览器中打开指定网址（无需捕获，直接填 URL）',
  },
  {
    action: 'browser_extract',
    icon: FileText,
    hint: '获取浏览器当前页面内容（默认整页文本；意图面板可「选择目标元素」精确提取）',
  },
  {
    action: 'browser_click',
    icon: MousePointer2,
    hint: '自动捕获浏览器中的真实点击 → 生成稳定 CSS selector（需先打开目标网页）',
  },
  // ── 桌面组（点击类与网页点击相邻）──
  { action: 'click', icon: MousePointerClick, hint: '捕获一次桌面单击/双击（系统自动识别双击）' },
  { action: 'scroll', icon: Mouse, hint: '捕获一次滚轮滚动' },
  {
    action: 'text',
    icon: Type,
    hint: '先捕获定位点击；意图面板可选「窗口输入」或「chatagent 处理」两种目标',
  },
  { action: 'hotkey', icon: Keyboard, hint: '捕获一次按键组合（如 Ctrl+C）' },
  { action: 'sleep', icon: Hourglass, hint: '无需捕获，直接填写等待秒数' },
  { action: 'region', icon: Scan, hint: '框选屏幕区域，保存 ROI 截图 + 坐标作锚点' },
  {
    action: 'find_image',
    icon: FileImage,
    hint: '框选模板图生成 desktop_find_image；确认时可选「找图后单击/双击」，由匹配坐标驱动点击',
  },
]

/** 默认位置：画布容器左上附近 (60,80)，避开右上「添加/录制」按钮区 */
const DEFAULT_POS = { x: 60, y: 80 }
/** 面板初始大致宽高（用于 localStorage 旧坐标超界 clamp） */
const PANEL_W = 200
const PANEL_H = 430

function clampPos(p: { x: number; y: number }): { x: number; y: number } {
  const maxX = Math.max(0, window.innerWidth - PANEL_W)
  const maxY = Math.max(0, window.innerHeight - PANEL_H)
  return {
    x: Math.max(0, Math.min(maxX, Math.round(p.x))),
    y: Math.max(0, Math.min(maxY, Math.round(p.y))),
  }
}

export function RecorderBar({
  status,
  action,
  drafts,
  insertTargetLabel,
  onPick,
  onCancelCapture,
  onAbort,
  onComplete,
  onSaveProgress,
  onEditDraft,
  onDeleteDraft,
  onClearDrafts,
}: RecorderBarProps) {
  const [pos, setPos] = useState(() => {
    try {
      const saved = localStorage.getItem('nuphus_recorder_pos')
      if (saved) return clampPos(JSON.parse(saved) as { x: number; y: number })
    } catch {}
    return DEFAULT_POS
  })
  const barRef = useRef<HTMLDivElement | null>(null)
  const dragging = useRef(false)
  const dragOffset = useRef({ x: 0, y: 0 })
  const [completeOpen, setCompleteOpen] = useState(false)
  const [notes, setNotes] = useState('')
  const [saveOpen, setSaveOpen] = useState(false)
  const [saveNotes, setSaveNotes] = useState('')
  const [draftOpen, setDraftOpen] = useState(false)
  const count = drafts.length

  const savePos = (x: number, y: number) => {
    try {
      localStorage.setItem('nuphus_recorder_pos', JSON.stringify({ x, y }))
    } catch {}
  }

  // 拖动把手 = 面板头部整行（grip 区）；记录按下相对偏移
  const handleMouseDown = (e: React.MouseEvent) => {
    if (!barRef.current) return
    const rect = barRef.current.getBoundingClientRect()
    dragging.current = true
    dragOffset.current = { x: e.clientX - rect.left, y: e.clientY - rect.top }
  }

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return
      const w = barRef.current?.offsetWidth || PANEL_W
      const h = barRef.current?.offsetHeight || PANEL_H
      const maxX = Math.max(0, window.innerWidth - w)
      const maxY = Math.max(0, window.innerHeight - h)
      setPos({
        x: Math.max(0, Math.min(maxX, e.clientX - dragOffset.current.x)),
        y: Math.max(0, Math.min(maxY, e.clientY - dragOffset.current.y)),
      })
    }
    const handleMouseUp = () => {
      if (dragging.current) {
        dragging.current = false
        savePos(pos.x, pos.y)
      }
    }
    window.addEventListener('mousemove', handleMouseMove)
    window.addEventListener('mouseup', handleMouseUp)
    return () => {
      window.removeEventListener('mousemove', handleMouseMove)
      window.removeEventListener('mouseup', handleMouseUp)
    }
  }, [pos])

  const openComplete = () => {
    setNotes('')
    setCompleteOpen(true)
  }

  const submitComplete = () => {
    onComplete(notes)
    setCompleteOpen(false)
  }

  const openSave = () => {
    setSaveNotes('')
    setSaveOpen(true)
  }

  const submitSave = () => {
    onSaveProgress(saveNotes)
    setSaveOpen(false)
  }

  return (
    <>
      <div
        ref={barRef}
        className={`rec-bar${status === 'capturing' ? ' rec-bar--capturing' : ''}`}
        style={{ left: pos.x, top: pos.y }}
      >
        {/* ── 上栏：会话 + 拖动把手 ── */}
        <div className="rec-bar-head" onMouseDown={handleMouseDown} title="拖拽移动录制框">
          <span className="rec-bar-grip">
            <IconGrip size={14} />
          </span>
          <span className="rec-bar-dot" />
          <span className="rec-bar-title">录制</span>
          <span className="rec-bar-count" title="已确认并加入画布的步骤数">
            {count} 步
          </span>
        </div>

        {status === 'idle' ? (
          <>
            <div className="rec-bar-insert" title="下一步确认后将插入到该位置">
              将插入：{insertTargetLabel ?? '层末尾'}
            </div>

            {/* ── 中栏：动作按钮区（10 动作纵排） ── */}
            <div className="rec-bar-section-title">动作</div>
            <div className="rec-bar-actions">
              {ACTIONS.map(({ action: a, icon: Icon, hint }) => (
                <button
                  key={a}
                  type="button"
                  className="rec-act-btn"
                  title={`开始录制：${hint}`}
                  onClick={() => {
                    playUiSound('session')
                    onPick(a)
                  }}
                >
                  <Icon size={14} />
                  <span>{ACTION_LABEL[a]}</span>
                </button>
              ))}
            </div>

            {/* ── 下栏：操作区 ── */}
            <div className="rec-bar-section-title">操作</div>
            <div className="rec-bar-ops">
              <button
                type="button"
                className="rec-btn"
                disabled={count === 0}
                title={
                  count === 0
                    ? '还没有录制任何步骤，先点上方动作开始录制至少一步'
                    : '保存当前进度到本工作流（下次打开点录制可自动恢复继续）'
                }
                onClick={openSave}
              >
                <Save size={13} /> 保存进度
              </button>
              <button
                type="button"
                className="rec-btn"
                disabled={count === 0}
                title={count === 0 ? '暂无草稿' : '查看/编辑/删除草稿步骤（删除可联动画布节点）'}
                onClick={() => setDraftOpen(true)}
              >
                <ListOrdered size={13} /> 草稿
                {count > 0 && <span className="rec-bar-badge">{count}</span>}
              </button>
              <div className="rec-bar-op-row">
                <button
                  type="button"
                  className="rec-btn rec-btn--danger rec-btn--grow"
                  title="放弃本次录制（草稿清空，会话结束）"
                  onClick={onAbort}
                >
                  取消
                </button>
                <button
                  type="button"
                  className="rec-btn rec-btn--primary rec-btn--grow"
                  title="生成 record-draft JSON，交给 WorkflowAgent 微调"
                  onClick={openComplete}
                >
                  完成
                </button>
              </div>
            </div>
          </>
        ) : (
          <div className="rec-bar-capturing">
            <span className="rec-bar-hint">
              {action === 'browser_click'
                ? '捕获中 · 网页点击 — 请在浏览器窗口点击目标元素，然后回到本窗口继续'
                : action
                  ? `捕获中 · ${ACTION_LABEL[action]} — 主窗口已最小化，请在目标窗口执行该操作；取消：任务栏点回本窗口点「取消」`
                  : '捕获中 — 主窗口已最小化，请在目标窗口执行该操作；取消：任务栏点回本窗口点「取消」'}
            </span>
            <button
              type="button"
              className="rec-btn rec-btn--danger"
              title="取消本次捕获（会话仍保留，可继续录其它动作）"
              onClick={onCancelCapture}
            >
              取消
            </button>
          </div>
        )}
      </div>

      {completeOpen && (
        <div className="rec-modal-backdrop" onClick={() => setCompleteOpen(false)}>
          <div className="rec-modal" onClick={e => e.stopPropagation()}>
            <h3 className="rec-modal-title">完成录制</h3>
            {count > 0 ? (
              <>
                <p className="rec-modal-desc">
                  已录制 <strong>{count}</strong> 步。可选填整体备注，供 WorkflowAgent
                  微调时理解上下文（可空）。
                </p>
                <textarea
                  className="rec-input rec-input--area rec-notes-input"
                  placeholder="整体备注（可选）：例如「先登录再抓取首页列表」…"
                  value={notes}
                  onChange={e => setNotes(e.target.value)}
                  rows={3}
                />
                <div className="rec-modal-actions">
                  <button type="button" className="rec-btn" onClick={() => setCompleteOpen(false)}>
                    继续录制
                  </button>
                  <button
                    type="button"
                    className="rec-btn rec-btn--primary"
                    onClick={submitComplete}
                  >
                    生成录制草稿
                  </button>
                </div>
              </>
            ) : (
              <>
                <p className="rec-modal-desc rec-modal-error">
                  还没有录制任何步骤，无法生成草稿。请先点上方动作开始录制至少一步。
                </p>
                <div className="rec-modal-actions">
                  <button
                    type="button"
                    className="rec-btn rec-btn--primary"
                    onClick={() => setCompleteOpen(false)}
                  >
                    返回
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {saveOpen && (
        <div className="rec-modal-backdrop" onClick={() => setSaveOpen(false)}>
          <div className="rec-modal" onClick={e => e.stopPropagation()}>
            <h3 className="rec-modal-title">保存录制进度</h3>
            {count > 0 ? (
              <>
                <p className="rec-modal-desc">
                  将已录制的 <strong>{count}</strong>{' '}
                  步保存为待续进度（本工作流）。下次打开此工作流点
                  「录制」会自动恢复这些草稿继续录制。可选填整体备注（可空）。
                </p>
                <textarea
                  className="rec-input rec-input--area rec-notes-input"
                  placeholder="整体备注（可选）：记录进度到哪了，下次续录时参考…"
                  value={saveNotes}
                  onChange={e => setSaveNotes(e.target.value)}
                  rows={3}
                />
                <div className="rec-modal-actions">
                  <button type="button" className="rec-btn" onClick={() => setSaveOpen(false)}>
                    继续录制
                  </button>
                  <button type="button" className="rec-btn rec-btn--primary" onClick={submitSave}>
                    <Save size={13} /> 保存进度
                  </button>
                </div>
              </>
            ) : (
              <>
                <p className="rec-modal-desc rec-modal-error">
                  还没有录制任何步骤，无法保存进度。请先点上方动作开始录制至少一步。
                </p>
                <div className="rec-modal-actions">
                  <button
                    type="button"
                    className="rec-btn rec-btn--primary"
                    onClick={() => setSaveOpen(false)}
                  >
                    返回
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {draftOpen && (
        <RecDraftPanel
          drafts={drafts}
          onClose={() => setDraftOpen(false)}
          onEdit={onEditDraft}
          onDelete={onDeleteDraft}
          onClear={onClearDrafts}
        />
      )}
    </>
  )
}
