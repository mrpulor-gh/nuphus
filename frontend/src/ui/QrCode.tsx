/**
 * QrCode — 轻量 SVG 二维码组件
 *
 * 选型：qrcode-generator@2.0.4（零依赖，11KB gzip，< 20KB 阈值）。
 * 仅用其矩阵生成能力（addData/make/isDark），SVG 方块由 React 自渲，
 * 不引 canvas/图片管线，体积小且天然适配设计 token（currentColor）。
 */

import { useMemo } from 'react'
import qrcode from 'qrcode-generator'

/** 纠错等级 M（≈15% 冗余）：URL 内容短、扫码环境为室内屏幕，足够 */
const EC_LEVEL = 'M'

interface QrCodeProps {
  value: string
  /** 整体边长（px） */
  size?: number
  className?: string
  style?: React.CSSProperties
}

export function QrCode({ value, size = 192, className, style }: QrCodeProps) {
  const modules = useMemo(() => {
    const qr = qrcode(0, EC_LEVEL)
    qr.addData(value)
    qr.make()
    const count = qr.getModuleCount()
    const dark: boolean[] = []
    for (let r = 0; r < count; r++) {
      for (let c = 0; c < count; c++) {
        dark.push(qr.isDark(r, c))
      }
    }
    return { count, dark }
  }, [value])

  const cell = size / modules.count
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className={className}
      style={style}
      role="img"
      aria-label="配对二维码"
      shapeRendering="crispEdges"
    >
      <rect width={size} height={size} fill="#ffffff" />
      {modules.dark.map((isDark, i) => {
        if (!isDark) return null
        const r = Math.floor(i / modules.count)
        const c = i % modules.count
        return (
          <rect
            key={i}
            x={c * cell}
            y={r * cell}
            width={Math.ceil(cell * 100) / 100}
            height={Math.ceil(cell * 100) / 100}
            fill="#000000"
          />
        )
      })}
    </svg>
  )
}
