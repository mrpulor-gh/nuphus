/**
 * 聊天主界面骨架：导航条 + 消息流 + 底部悬浮输入栏（三层信息阶梯封顶）
 * 危险操作确认卡悬浮于输入栏上方（最高交互优先级，不属第四层——它是模态性质的打断）
 *
 * 底部输入栏只有发送按钮（手机端不做 agent 执行控制：暂停/继续/终止在桌面端操作）；
 * 工作流执行遥控（暂停/继续/终止）由 WorkflowRunCard 提供（POST /workflow-*）。
 */

import type {
  ChatMessage,
  ActivityState,
  PendingConfirm,
  PendingRefine,
  PendingUserInput,
} from '../store'
import type { WsStatus } from '../ws'
import type { ShelfSessions } from '../api'
import type { ConnectionMode } from '../connection'
import { Loader2 } from 'lucide-react'
import { t } from '../i18n'
import NavBar from './NavBar'
import MessageList from './MessageList'
import Composer from './Composer'
import ConfirmCard from './ConfirmCard'
import RefineModal from './RefineModal'
import UserInputCard from './UserInputCard'
import AddToHomeScreen from './AddToHomeScreen'
import WorkflowRunCard, { type WorkflowRunState } from './WorkflowRunCard'

interface Props {
  messages: ChatMessage[]
  activity: ActivityState
  wsStatus: WsStatus
  historyError: string | null
  pendingConfirm: PendingConfirm | null
  pendingRefine: PendingRefine | null
  pendingUserInput: PendingUserInput | null
  refining: boolean
  token: string
  assistantName?: string
  /** 当前执行模型（session_info 事件下发，Composer 只读展示） */
  model?: string
  /** 会话累计上下文用量（token_usage 事件，Composer 模型卡展示） */
  tokenUsage?: { inputTokens: number; cacheHitTokens?: number }
  /** 当前连接渠道（lan=局域网直连 / wan=中继）；透传 NavBar header 状态 pill */
  connMode?: ConnectionMode | null
  onSend: (content: string, opts?: { images?: string[]; mode?: string }) => Promise<void>
  /** 确认卡提交成功回调（携带 approved 结果，供上层 toast 反馈） */
  onConfirmResolved: (approved: boolean) => void
  /** 输入卡处理完成回调（提交成功 / 取消） */
  onUserInputResolved: (submitted: boolean) => void
  /** 点评回调（assistant 消息「点评」按钮触发，App 层提交记忆评分） */
  onRateMessage?: (message: ChatMessage) => void
  /** 模型切换成功回调（更新 store.model） */
  onModelChanged?: (model: string) => void
  /** 提炼弹窗：确认（触发 /refine）/ 跳过 */
  onRefineConfirm: () => void
  onRefineSkip: () => void
  /** 轻量弹窗提示（追加指令受理等一句话提醒，不生成消息气泡） */
  toast?: string | null
  /** 轻量反馈回调（Composer 内图片超限/模式切换等） */
  onToast?: (text: string) => void
  /** 历史加载失败横幅点击重试（自动重试之外的主动出口） */
  onRetryHistory?: () => void
  /** 手动重新拉取历史（+ 弹窗胶囊触发，网络/应用切换后历史不显示时一键刷新） */
  onReloadHistory?: () => void
  /** 中继通道手动「切换到本地网络」（透传 NavBar 网络弹窗，仅 wan 展示） */
  onSwitchToLanManual?: () => void
  /** header wifi 图标：打开切本地网络确认弹窗（透传 NavBar，仅 wan 展示） */
  onLanSwitchRequest?: () => void
  /** 手动切本地请求进行中 */
  lanSwitching?: boolean
  /** 终止执行（执行中 NavBar 显示终止按钮，POST /stop 直接终止） */
  onStopExecution?: () => void
  /** 新会话（Composer + 弹窗触发）：清空前端消息 */
  onNewChat?: () => void
  /** 断开连接（Composer 设置弹窗触发）：清除 token 回到配对页 */
  onDisconnect?: () => void
  /** 桌面展示台会话清单镜像（null = 未加载/不可用） */
  sessions?: ShelfSessions | null
  /** 遥控切换桌面当前会话（App 层 POST /session/switch + 刷新） */
  onSwitchSession?: (id: string, mode?: string) => void
  /** 工作流执行实时状态（workflow_event 驱动，存在即渲染 WorkflowRunCard） */
  workflowRun?: WorkflowRunState
  /** 工作流遥控请求进行中（防重复提交） */
  wfControlBusy?: boolean
  onWorkflowPause?: () => void
  onWorkflowResume?: () => void
  onWorkflowTerminate?: () => void
  /** 移除工作流胶囊（X 任意时刻可点，执行中也可；不影响后端执行） */
  onWorkflowDismiss?: () => void
}

export default function ChatScreen({
  messages,
  activity,
  wsStatus,
  historyError,
  pendingConfirm,
  pendingRefine,
  pendingUserInput,
  refining,
  token,
  assistantName,
  model,
  tokenUsage,
  connMode,
  onSend,
  onConfirmResolved,
  onUserInputResolved,
  onRateMessage,
  onModelChanged,
  onRefineConfirm,
  onRefineSkip,
  toast,
  onToast,
  onRetryHistory,
  onReloadHistory,
  onSwitchToLanManual,
  onLanSwitchRequest,
  lanSwitching,
  onStopExecution,
  onNewChat,
  onDisconnect,
  sessions,
  onSwitchSession,
  workflowRun,
  wfControlBusy,
  onWorkflowPause,
  onWorkflowResume,
  onWorkflowTerminate,
  onWorkflowDismiss,
}: Props) {
  return (
    <div className="mobile-app">
      <NavBar
        wsStatus={wsStatus}
        activity={activity}
        token={token}
        model={model}
        tokenUsage={tokenUsage}
        connMode={connMode}
        onReloadHistory={onReloadHistory}
        onSwitchToLanManual={onSwitchToLanManual}
        onLanSwitchRequest={onLanSwitchRequest}
        lanSwitching={lanSwitching}
        onNewChat={onNewChat}
        onDisconnect={onDisconnect}
        sessions={sessions}
        onSwitchSession={onSwitchSession}
      />
      {historyError && (
        <div
          className="mobile-banner is-retryable"
          role="button"
          tabIndex={0}
          onClick={onRetryHistory}
          onKeyDown={e => {
            if ((e.key === 'Enter' || e.key === ' ') && onRetryHistory) onRetryHistory()
          }}
        >
          {historyError}
          <span className="mobile-banner-retry">{t('mobile.retry')}</span>
        </div>
      )}
      <AddToHomeScreen visible={true} />
      {/* 工作流胶囊：absolute 悬浮于消息区顶部（覆盖式，滚动内容从胶囊背后经过 → 毛玻璃原生质感） */}
      {workflowRun && (
        <div className="mobile-wf-float">
          <WorkflowRunCard
            workflowRun={workflowRun}
            busy={wfControlBusy}
            onPause={onWorkflowPause ?? (() => {})}
            onResume={onWorkflowResume ?? (() => {})}
            onTerminate={onWorkflowTerminate ?? (() => {})}
            onDismiss={onWorkflowDismiss}
          />
        </div>
      )}
      <MessageList
        messages={messages}
        activity={activity}
        assistantName={assistantName}
        tokenUsage={tokenUsage}
        onRateMessage={onRateMessage}
      />
      {pendingConfirm && (
        <ConfirmCard confirm={pendingConfirm} token={token} onResolved={onConfirmResolved} />
      )}
      {pendingUserInput && (
        <UserInputCard input={pendingUserInput} token={token} onResolved={onUserInputResolved} />
      )}
      {pendingRefine && !refining && (
        <RefineModal
          refine={pendingRefine}
          refining={refining}
          onConfirm={onRefineConfirm}
          onSkip={onRefineSkip}
        />
      )}
      {refining && (
        <div className="mobile-refine-progress" role="status">
          <Loader2 size={14} aria-hidden="true" /> 正在提炼上下文…
        </div>
      )}
      {toast && <div className="mobile-toast">{toast}</div>}
      {/* 发送按钮：offline（WS 断开）才禁用；connecting（刚打开/重连中）允许发送——
          POST 走 HTTP 不依赖 WS，消息受理后由 history_merge 兜底确认，
          曾因 connecting 禁用导致用户首条消息发不出去（P1）。 */}
      <Composer
        disabled={wsStatus === 'offline'}
        token={token}
        assistantName={assistantName}
        mode={activity.mode}
        model={model}
        tokenUsage={tokenUsage}
        isProcessing={activity.running}
        onStopExecution={onStopExecution}
        onSend={onSend}
        onModelChanged={onModelChanged}
        onToast={onToast}
        onReloadHistory={onReloadHistory}
      />
    </div>
  )
}
