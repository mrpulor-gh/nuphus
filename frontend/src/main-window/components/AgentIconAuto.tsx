import { useEffect, useState } from 'react'
import {
  IconBot,
  IconTerminal,
  IconCpu,
  IconGlobe,
  IconLayers,
  IconBox,
  IconMonitor,
  IconWrench,
  IconBrain,
  IconPlug,
  IconRocket,
  IconHardDrive,
  IconAppWindow,
  IconRadio,
} from '../../ui/Icons'
import { extractAgentIcon } from '../lib/api'

/**
 * 外部 Agent 头像渲染的单一实现（共享组件）。
 * 消费方：ExternalAgentsPage（编辑区预览 + chips）、ExternalAgentsStatusBar（胶囊 + popover）。
 * 此前页面私有导致三处渲染不一致（状态栏/设置 chips 显示默认 SVG，仅编辑区显示真实头像）。
 */

/** 渲染 agent 图标：icon 字符串 → lucide 组件（未知名 fallback bot）；iconUrl 优先（应用图标 data URL） */
export function AgentIcon({
  icon,
  size = 14,
  iconUrl,
  imgSize,
  imgAvatar,
}: {
  icon: string
  size?: number
  iconUrl?: string | null
  /** img 实际渲染尺寸（用户自定义头像填满容器用）；缺省同 size */
  imgSize?: number
  /** img 按头像语义渲染：填满 + 圆形裁切（object-fit: cover），底衬色不参与 */
  imgAvatar?: boolean
}) {
  if (iconUrl) {
    const w = imgSize ?? size
    return (
      <img
        className={imgAvatar ? 'agent-icon-img agent-icon-img-avatar' : 'agent-icon-img'}
        src={iconUrl}
        width={w}
        height={w}
        alt=""
        draggable={false}
      />
    )
  }
  switch (icon) {
    case 'terminal':
      return <IconTerminal size={size} />
    case 'cpu':
      return <IconCpu size={size} />
    case 'globe':
      return <IconGlobe size={size} />
    case 'layers':
      return <IconLayers size={size} />
    case 'box':
      return <IconBox size={size} />
    case 'monitor':
      return <IconMonitor size={size} />
    case 'wrench':
      return <IconWrench size={size} />
    case 'brain':
      return <IconBrain size={size} />
    case 'plug':
      return <IconPlug size={size} />
    case 'rocket':
      return <IconRocket size={size} />
    case 'hard-drive':
      return <IconHardDrive size={size} />
    case 'app-window':
      return <IconAppWindow size={size} />
    case 'radio':
      return <IconRadio size={size} />
    default:
      return <IconBot size={size} />
  }
}

/** agent 名 → 图标类型（CLI 类终端图标 / 其余 bot；用于 is-cli / is-agent 样式类） */
export function agentKind(name: string): 'cli' | 'agent' {
  const n = name.toLowerCase()
  if (n.includes('claude') || n.includes('code') || n.includes('cli')) return 'cli'
  return 'agent'
}

/** icon 值是否为文件路径（盘符 / UNC / 含扩展名） */
export function isIconPath(v: string): boolean {
  if (!v) return false
  if (/^[a-zA-Z]:[\\/]/.test(v) || v.startsWith('\\\\')) return true
  return /\.[a-zA-Z0-9]{2,4}$/.test(v)
}

/** 从 open/process 启动串中提取可执行/图标文件路径（引号优先，其次含 .exe/.cmd/.bat/.lnk/.ico token） */
export function exePathFromOpen(s: string): string | null {
  if (!s) return null
  const quoted = s.match(/"([^"]+\.(?:exe|cmd|bat|lnk|ico|dll))"/i)
  if (quoted) return quoted[1]
  const token = s.match(/([A-Za-z]:[\\/][^"'\s]+\.(?:exe|cmd|bat|lnk|ico|dll))/i)
  if (token) return token[1]
  return null
}

/** 解析 icon 提取源路径：显式路径直接返回；auto → open/process 中的可执行路径 */
export function iconSourcePath(d: { icon: string; open: string; process: string }): string | null {
  if (isIconPath(d.icon)) return d.icon
  if (d.icon === 'auto') return exePathFromOpen(d.open) || exePathFromOpen(d.process) || null
  return null
}

/** 跨组件图标提取缓存（同一会话内同一路径只提取一次，避免重复 PowerShell 调用） */
export const iconUrlCache = new Map<string, string>()

/** 带自动提取的 AgentIcon：预设 SVG 直接渲染；auto/路径按需提取应用图标（带缓存） */
export function AgentIconAuto({
  icon,
  size = 14,
  name = '',
  open = '',
  process = '',
  avatarSize,
}: {
  icon: string
  size?: number
  name?: string
  open?: string
  process?: string
  /** 用户自定义头像（icon=显式路径）的填满尺寸 = 容器内容区尺寸；缺省同 size。
   *  设计纪律：底衬色只属于系统默认 SVG——用户头像按 100% 圆形裁切渲染 */
  avatarSize?: number
}) {
  const src = iconSourcePath({ icon, open, process })
  const [url, setUrl] = useState<string | null>(() =>
    src ? (iconUrlCache.get(src) ?? null) : null,
  )

  useEffect(() => {
    const sourceRaw = iconSourcePath({ icon, open, process })
    const source: string = sourceRaw ?? ''
    if (!source) {
      setUrl(null)
      return
    }
    if (iconUrlCache.has(source)) {
      setUrl(iconUrlCache.get(source)!)
      return
    }
    let cancelled = false
    extractAgentIcon(source)
      .then(u => {
        if (!u) return
        iconUrlCache.set(source, u)
        if (!cancelled) setUrl(u)
      })
      .catch(() => {
        /* 提取失败：保持默认 SVG 兜底 */
      })
    return () => {
      cancelled = true
    }
  }, [icon, open, process])

  // auto：优先应用图标（url）；提取不到时按 agent 名推断类型（与状态条 agentKind 一致）
  if (icon === 'auto' && !url) {
    const n = name.toLowerCase()
    if (n.includes('claude') || n.includes('code') || n.includes('cli')) {
      return <IconTerminal size={size} />
    }
    return <IconBot size={size} />
  }
  // 用户显式设置的头像（icon=路径）：img 按 avatarSize 填满容器 + 圆形裁切；
  // auto 提取的应用 logo：img 按 size 缩进，保留底衬（logo 裁圆可能切边，不按头像语义）
  const custom = isIconPath(icon)
  return (
    <AgentIcon
      icon={icon}
      size={size}
      iconUrl={url}
      imgSize={custom ? (avatarSize ?? size) : size}
      imgAvatar={custom}
    />
  )
}