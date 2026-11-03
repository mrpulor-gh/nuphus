/**
 * icons.tsx — Inspector v2 组件族共享的内联 SVG 图标集。
 *
 * 铁律（设计标准 D4/S4）：
 * - 一律 inline SVG，stroke=currentColor，尺寸由 size 传入；
 * - 禁 emoji、禁位图、禁引入第二套图标体系（面板关闭按钮仍复用 ui/Icons 的 IconX，
 *   与现有 Inspector 身份头保持一致）；
 * - aria-hidden + focusable=false：图标是装饰，语义由所在控件的 role/aria 承担。
 */
import type { ReactNode } from 'react'

function IconBase({ size, children }: { size: number; children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  )
}

/** 帮助（字段行 help 图标，hover 出 title） */
export function IconHelp({ size = 12 }: { size?: number }) {
  return (
    <IconBase size={size}>
      <circle cx="12" cy="12" r="9" />
      <path d="M9.6 9a2.5 2.5 0 1 1 3.5 2.3c-.7.4-1.1 1-1.1 1.8v.4" />
      <path d="M12 17h.01" />
    </IconBase>
  )
}

/** 警示（字段行 error / JsonField 非法徽标） */
export function IconAlert({ size = 11 }: { size?: number }) {
  return (
    <IconBase size={size}>
      <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </IconBase>
  )
}

/** 对勾（JsonField 合法徽标） */
export function IconCheck({ size = 10 }: { size?: number }) {
  return (
    <IconBase size={size}>
      <path d="M20 6 9 17l-5-5" />
    </IconBase>
  )
}

/** 加号（KeyValueRows 添加一项） */
export function IconPlus({ size = 10 }: { size?: number }) {
  return (
    <IconBase size={size}>
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </IconBase>
  )
}

/** 删除（KeyValueRows 行删除，hover 染 error） */
export function IconTrash({ size = 11 }: { size?: number }) {
  return (
    <IconBase size={size}>
      <path d="M3 6h18" />
      <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
    </IconBase>
  )
}

/** 变量花括号（PathField 插变量按钮 {{ }}） */
export function IconBraces({ size = 11 }: { size?: number }) {
  return (
    <IconBase size={size}>
      <path d="M8 3H7a2 2 0 0 0-2 2v5a2 2 0 0 1-2 2 2 2 0 0 1 2 2v5a2 2 0 0 0 2 2h1" />
      <path d="M16 21h1a2 2 0 0 0 2-2v-5a2 2 0 0 1 2-2 2 2 0 0 1-2-2V5a2 2 0 0 0-2-2h-1" />
    </IconBase>
  )
}
