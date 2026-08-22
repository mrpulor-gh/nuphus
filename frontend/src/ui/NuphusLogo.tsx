/**
 * Nuphus Logo — 「暂停键」
 *
 * 设计理念（大王钦定）:
 *   双竖 = 暂停键 ⏸ = 控制，智能体对桌面的接管与收束
 *   右下开口 = 工作流的确定性规则之内，留有智能介入的余地
 *   整体 = 智能体的抽象头像（框为颅、双竖为目）
 *   精确坐标与 icon-source.svg 保持一致
 *
 * 使用场景:
 *   - TitleBar:       variant="mark" size=20
 *   - WelcomeScreen:  variant="icon" size=48
 *   - SplashScreen:   variant="icon" size=96 glow
 *   - 关于页/设置页:   variant="icon" size=48
 *   - 状态栏:          variant="mark" size=16
 */

import { useTheme } from '../hooks/useTheme'

interface NuphusLogoProps {
  size?: number
  variant?: 'mark' | 'icon'
  spin?: boolean
  glow?: boolean
}

export function NuphusLogo({
  size = 24,
  variant = 'mark',
  spin = false,
  glow = false,
}: NuphusLogoProps) {
  const { theme } = useTheme()
  const vb = 256

  const isDark = theme !== 'light'
  const bg = isDark ? '#1a1a2e' : '#ffffff'
  const fg = isDark ? '#f5f5fa' : '#1a1a2e'

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${vb} ${vb}`}
      fill="none"
      style={{
        display: 'block',
        flexShrink: 0,
        ...(spin ? { overflow: 'visible' as const } : {}),
      }}
      shapeRendering="geometricPrecision"
    >
      {spin && (
        <style>{`@keyframes nSpin{0%{transform:rotate(0deg)}30%{transform:rotate(180deg)}70%{transform:rotate(180deg)}100%{transform:rotate(360deg)}}`}</style>
      )}

      {glow && (
        <>
          <defs>
            <filter id="logo-glow" x="-50%" y="-50%" width="200%" height="200%">
              <feGaussianBlur stdDeviation="20" />
            </filter>
          </defs>
          <style>{`@keyframes logoBreathe{0%,100%{opacity:0.06}50%{opacity:0.40}}`}</style>
          <rect
            x={8}
            y={8}
            width={240}
            height={240}
            rx={52}
            fill={fg}
            filter="url(#logo-glow)"
            style={{ animation: 'logoBreathe 3s ease-in-out infinite' }}
          />
        </>
      )}

      {/* 背景底板 — 与 icon-source.svg 精确一致，仅限于窗口壳内部 */}
      <rect x={20} y={20} width={216} height={216} rx={44} fill={bg} />

      {/* 桌面窗口壳 */}
      <g
        style={
          spin
            ? {
                transformOrigin: `${vb / 2}px ${vb / 2}px`,
                animation: 'nSpin 6s ease-in-out infinite',
              }
            : undefined
        }
      >
        {/* 窗口框（右下开口）— 与 icon-source.svg 精确一致 */}
        <path
          d="M64 20 H192 A44 44 0 0 1 236 64 V156"
          stroke={fg}
          strokeWidth={24}
          strokeLinecap="round"
          fill="none"
        />
        <path
          d="M200 236 H64 A44 44 0 0 1 20 192 V64 A44 44 0 0 1 64 20"
          stroke={fg}
          strokeWidth={24}
          strokeLinecap="round"
          fill="none"
        />

        {/* 双竖（暂停键）— 与 icon-source.svg 精确一致 */}
        <path
          d="M80 180 L80 76 M176 180 L176 76"
          stroke={fg}
          strokeWidth={24}
          strokeLinecap="round"
          fill="none"
        />
      </g>
    </svg>
  )
}
