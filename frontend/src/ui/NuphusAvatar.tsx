/**
 * NuphusAvatar — Nuphus 虚拟形象
 *
 * 以身份 ICON「暂停键」为脸：
 *   框 = 颅，双竖 = 目，右下开口 = 气息出入的嘴部
 *
 * 单色纪律：情绪只用运动表达，永不变色（跟随主题 fg）。
 * 目形变体是结构姿态：prefers-reduced-motion 下动画关闭，状态仍可读。
 *
 * 状态机（Agent 运行时 → 形象状态）：
 *   idle      会话空闲          呼吸 + 定期眨眼
 *   thinking  LLM 调用中        眼珠左右巡视 + 头部轻摆
 *   working   工具执行中        双竖均衡器（相位错开）
 *   streaming 流式输出中        轻快起伏 + 频密眨眼
 *   confirm   等待用户确认      目睁大 + 注意力脉冲
 *   success   回合成功          跃起 + 笑眼弧
 *   error     错误/重试耗尽     目扁平 + 摇头（不变红）
 *   sleep     空闲超时          闭目 + 深呼吸
 *
 * 使用：
 *   <NuphusAvatar state="idle" size={40} />
 *   <NuphusAvatar state="idle" size={40} gaze={{ x: 0.3, y: -0.2 }} />
 */

import { useTheme } from '../hooks/useTheme'
import '../styles/avatar.css'

export type NuphusAvatarState =
  'idle' | 'thinking' | 'working' | 'streaming' | 'confirm' | 'success' | 'error' | 'sleep'

interface NuphusAvatarProps {
  state?: NuphusAvatarState
  size?: number
  className?: string
  /** 注视方向（归一化 -1~1）：x 左右偏移、y 上下偏移 + 俯仰缩短。缺省时眼睛只走状态动画。 */
  gaze?: { x: number; y: number }
}

const SHELL_1 = 'M64 20 H192 A44 44 0 0 1 236 64 V156'
const SHELL_2 = 'M200 236 H64 A44 44 0 0 1 20 192 V64 A44 44 0 0 1 64 20'

/** 目形变体（结构姿态） */
const EYES: Record<string, (x: number) => string> = {
  bar: x => `M${x} 180 L${x} 76`,
  wide: x => `M${x} 190 L${x} 66`,
  arc: x => `M${x - 16} 142 Q${x} 114 ${x + 16} 142`,
  flat: x => `M${x} 168 L${x} 112`,
  closed: x => `M${x} 134 L${x} 122`,
}

const STATE_EYE: Record<NuphusAvatarState, keyof typeof EYES> = {
  idle: 'bar',
  thinking: 'bar',
  working: 'bar',
  streaming: 'bar',
  confirm: 'wide',
  success: 'arc',
  error: 'flat',
  sleep: 'closed',
}

export function NuphusAvatar({ state = 'idle', size = 40, className, gaze }: NuphusAvatarProps) {
  const { theme } = useTheme()
  const fg = theme !== 'light' ? '#f5f5fa' : '#1a1a2e'
  const eye = EYES[STATE_EYE[state]]

  // 注视偏移（SVG 用户单位，随 viewBox 256 缩放）：x 左右 ±14、y 上下 ±10、俯仰缩短 scaleY
  const gx = gaze?.x ?? 0
  const gy = gaze?.y ?? 0
  const gazeTransform = `translate(${gx * 14}px, ${gy * 10}px) scaleY(${1 - Math.abs(gy) * 0.18})`

  return (
    <svg
      className={`nv-st-${state}${className ? ' ' + className : ''}`}
      width={size}
      height={size}
      viewBox="0 0 256 256"
      fill="none"
      role="img"
      aria-label={`Nuphus ${state}`}
      style={{ display: 'block', flexShrink: 0 }}
    >
      <g className="nv-whole">
        <path d={SHELL_1} stroke={fg} strokeWidth={24} strokeLinecap="round" fill="none" />
        <path d={SHELL_2} stroke={fg} strokeWidth={24} strokeLinecap="round" fill="none" />
        <g className="nv-gaze" style={{ transform: gazeTransform }}>
          <path
            className="nv-eye l"
            d={eye(80)}
            stroke={fg}
            strokeWidth={24}
            strokeLinecap="round"
            fill="none"
          />
          <path
            className="nv-eye r"
            d={eye(176)}
            stroke={fg}
            strokeWidth={24}
            strokeLinecap="round"
            fill="none"
          />
        </g>
      </g>
    </svg>
  )
}
