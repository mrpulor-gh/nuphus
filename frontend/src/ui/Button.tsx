import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react'

type Variant =
  'default' | 'primary' | 'danger' | 'ghost' | 'error-primary' | 'error-secondary' | 'error-ghost'

type Size = 'sm' | 'md' | 'lg'

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant
  size?: Size
  loading?: boolean
  icon?: ReactNode
}

const variantClass: Record<Variant, string> = {
  default: 'compact-btn',
  primary: 'compact-btn compact-btn--primary',
  danger: 'compact-btn danger-btn',
  ghost: 'compact-btn ghost-btn',
  'error-primary': 'error-btn error-btn--primary',
  'error-secondary': 'error-btn error-btn--secondary',
  'error-ghost': 'error-btn error-btn--ghost',
}

const sizeClass: Record<Size, string> = {
  sm: 'btn--sm',
  md: '',
  lg: 'btn--lg',
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      variant = 'default',
      size = 'md',
      loading,
      icon,
      children,
      style,
      disabled,
      className,
      ...rest
    },
    ref,
  ) => {
    return (
      <button
        ref={ref}
        className={[variantClass[variant], sizeClass[size], className].filter(Boolean).join(' ')}
        disabled={disabled || loading}
        style={{ ...(loading ? { position: 'relative' } : {}), ...style }}
        {...rest}
      >
        {loading && (
          <span className="btn-spinner" style={icon || children ? { marginRight: 4 } : {}} />
        )}
        {!loading && icon && <span style={{ display: 'inline-flex' }}>{icon}</span>}
        {children}
      </button>
    )
  },
)

type IconVariant =
  | 'default'
  | 'danger'
  | 'ghost'
  | 'msg-action'
  | 'modal-close'
  | 'desktop-toolbar'
  | 'desktop-toolbar-active'
  | 'input-send'
  | 'input-send-active'
  | 'input-tool'
  | 'compact-header-close'
  | 'win-btn'
  | 'win-close'
  | 'raw'

const iconVariantClass: Record<string, string> = {
  default: 'compact-btn',
  danger: 'compact-btn danger-btn',
  ghost: 'compact-btn ghost-btn',
  'msg-action': 'msg-action-btn',
  'modal-close': 'modal-close',
  'desktop-toolbar': 'desktop-toolbar-btn',
  'desktop-toolbar-active': 'desktop-toolbar-btn active',
  'input-send': 'input-send-btn',
  'input-send-active': 'input-send-btn active',
  'input-tool': 'input-tool-btn',
  'compact-header-close': 'compact-header-close',
  'win-btn': 'win-btn',
  'win-close': 'win-btn win-btn-close',
  raw: '',
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: IconVariant
  label: string
  showLabel?: boolean
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(
  ({ variant = 'default', label, showLabel, children, className, ...rest }, ref) => {
    return (
      <button
        ref={ref}
        // 注意用 ?? 而非 ||：variant 'raw' 映射为空串（仅保留自定义 className），
        // 空串是合法值，不能被 || 兜底成 compact-btn（会带入 padding 把图标挤没）
        className={[iconVariantClass[variant] ?? 'compact-btn', className]
          .filter(Boolean)
          .join(' ')}
        aria-label={label}
        title={showLabel ? undefined : label}
        {...rest}
      >
        {children}
        {showLabel && <span className="icon-btn-label">{label}</span>}
      </button>
    )
  },
)
