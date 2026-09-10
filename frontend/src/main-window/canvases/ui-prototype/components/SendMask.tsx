import { createPortal } from 'react-dom'
import { Palette } from '../lib/tokens'

/** 遮罩层级：画布全屏壳（.canvas-hub / .canvas-page-host）= 2500，工作台宿主层 = 100，
 *  取 2600 稳压画布与工作台（含工作台 header 的 tab 切换）。
 *  仍低于应用级全局层（右键菜单 3000 / 引用条 3100 / 错误横幅 9998+ / 桌面工具条 9999）：
 *  那些不属于画布作用域，遮罩期间保持可达——盖住它们反而会在出错时堵死退路。 */
export const SEND_MASK_Z = 2600

/** 发送 / 导出重活期间的全屏遮罩：挡住画布与工作台的一切指针操作。
 *
 *  必须挂在 document.body 上（portal）：画布躺在 .canvas-workbench-host（fixed + z-index:100）
 *  这个 stacking context 里，任何 z-index 都出不去，压不住全屏画布壳。
 *  键盘另在画布的 keydown 监听里短路，遮罩挡不住 keydown。
 *  底色单独一层：opacity 只压这一层，卡片不受影响，也避开调色板取色格式（不做 hex 拼接）。 */
export function SendMask({ p, text }: { p: Palette; text: string }) {
  return createPortal(
    <div
      role="status"
      aria-busy="true"
      aria-live="polite"
      aria-label={text}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: SEND_MASK_Z,
        display: 'grid',
        placeItems: 'center',
        cursor: 'progress',
      }}
    >
      <div
        aria-hidden
        style={{
          position: 'absolute',
          inset: 0,
          background: p.surfaceContainer,
          opacity: 0.78,
        }}
      />
      <div
        style={{
          position: 'relative',
          display: 'flex',
          alignItems: 'center',
          gap: 16,
          padding: '24px 28px',
          borderRadius: 28,
          background: p.surfaceContainerHigh,
          color: p.onSurfaceVariant,
          boxShadow: '0 8px 24px rgba(0,0,0,0.18), 0 2px 6px rgba(0,0,0,0.10)',
        }}
      >
        {/* 用既有的 m3-spin（transform 旋转）：主线程被捕获占住时合成层仍能继续动 */}
        <span
          aria-hidden
          className="m3-spin"
          style={{
            display: 'inline-block',
            width: 28,
            height: 28,
            borderRadius: '50%',
            border: `3px solid ${p.surfaceContainerHighest}`,
            borderTopColor: p.primary,
          }}
        />
        <span style={{ fontSize: 14, fontWeight: 600 }}>{text}</span>
      </div>
    </div>,
    document.body,
  )
}
