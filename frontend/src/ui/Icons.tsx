/// Shared SVG icons — re-exported from lucide-react

import React from 'react'

export {
  Copy as IconCopy,
  Check as IconCheck,
  Paperclip as IconPaperclip,
  Folder as IconFolder,
  Plug as IconPlug,
  Send as IconSend,
  Square as IconSquare,
  X as IconX,
  Clock as IconClock,
  Wrench as IconWrench,
  TrendingUp as IconTrendingUp,
  Settings as IconSettings,
  BarChart as IconBarChart,
  Activity as IconActivity,
  Brain as IconBrain,
  Bot as IconBot,
  PieChart as IconPieChart,
  Sprout as IconSprout,
  Search as IconSearch,
  ArrowLeft as IconArrowLeft,
  Grid as IconGrid,
  Monitor as IconMonitor,
  FileText as IconFile,
  Globe as IconGlobe,
  MousePointerClick as IconDesktop,
  Compass as IconBrowser,
  MessageCircle as IconMessageCircle,
  ChevronDown as IconChevronDown,
  ChevronUp as IconChevronUp,
  ChevronRight as IconChevronRight,
  Star as IconStar,
  Trash2 as IconTrash2,
  Edit3 as IconEdit3,
  ArrowUpFromDot as IconArrowUpFromDot,
  Camera as IconCamera,
  Crop as IconCrop,
  Crosshair as IconCrosshair,
  Image as IconImage,
  Keyboard as IconKeyboard,
  Type as IconType,
  GripVertical as IconGrip,
  Pin as IconPin,
  PinOff as IconPinOff,
  Palette as IconPalette,
  Shield as IconShield,
  Smartphone as IconSmartphone,
  Sparkles as IconSparkles,
  BrushCleaning as IconBrushCleaning,
  Store as IconStore,
  Minus as IconMinus,
  Plus as IconPlus,
  History as IconHistory,
  Play as IconPlay,
  Mic as IconMic,
  Eye as IconEye,
  TriangleAlert as IconAlertTriangle,
  Puzzle as IconPuzzle,
  Upload as IconUpload,
  Package as IconPackage,
  Code as IconCode,
  BookOpen as IconBook,
  Download as IconDownload,
  ExternalLink as IconExternalLink,
  RefreshCw as IconRefresh,
  Cpu as IconCpu,
  Layers as IconLayers,
  Box as IconBox,
  SquareTerminal as IconTerminalSquare,
  Rocket as IconRocket,
  HardDrive as IconHardDrive,
  AppWindow as IconAppWindow,
  Radio as IconRadio,
} from 'lucide-react'

export function ErrorXIcon({ size = 40 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <circle cx="12" cy="12" r="10" />
      <line x1="15" y1="9" x2="9" y2="15" />
      <line x1="9" y1="9" x2="15" y2="15" />
    </svg>
  )
}

export function IconWorkflow({
  size = 14,
  style,
  className,
}: {
  size?: number
  style?: React.CSSProperties
  className?: string
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      style={style}
      className={className}
    >
      <rect x="2" y="7" width="6" height="10" rx="2" />
      <rect x="9" y="4" width="6" height="16" rx="2" />
      <rect x="16" y="7" width="6" height="10" rx="2" />
      <line x1="8" y1="12" x2="9" y2="12" />
      <line x1="15" y1="12" x2="16" y2="12" />
    </svg>
  )
}

/** 终端图标 — 简洁窗口+提示符，适配小尺寸，与 lucide 风格一致 */
export function IconTerminal({ size = 14 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2" y="4" width="20" height="16" rx="2.5" />
      <path d="M6 9 L9 12 L6 15" />
    </svg>
  )
}
