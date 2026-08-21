/**
 * ToolPalette.tsx — 画布左侧工具分类竖条
 *
 * 契约：
 * - 工具归属哪个分类一律来自 wf_tools 返回的 group 字段（后端 registry.workflow_tool_group
 *   唯一来源）；本文件只持有 group 键 → 图标/中文标签的展示映射，不复制归属规则
 * - 工具项主行显示中文动作名（TOOL_LABELS 纯展示映射，未命中兜底英文注册名），
 *   副行显示英文注册名小字；英文 description 收进 title tooltip 不占行
 * - hover 分类浮出工具列表；点击创建 / 拖拽到画布创建（HTML5 DnD）
 * - 只读（运行中/旧格式）时整体禁用，不产生任何编辑
 */

import { useEffect, useMemo, useState } from 'react'
import {
  Monitor, Globe, FolderOpen, Terminal, Sparkles, Puzzle,
} from 'lucide-react'
import type { ToolSchema } from '../../core/types'
import { loadToolsOnce } from './Inspector'

/** 拖拽 MIME 类型（CanvasPage onDragOver/onDrop 据此识别工具拖入） */
export const TOOL_DRAG_MIME = 'application/x-nuphus-tool'

/** group 键 → 图标/中文标签（纯展示映射；未知键由消费方归 misc） */
const GROUP_META: Record<string, { icon: typeof FolderOpen; label: string }> = {
  desktop: { icon: Monitor, label: '桌面操作' },
  browser: { icon: Globe, label: '网页操作' },
  file: { icon: FolderOpen, label: '文件操作' },
  system: { icon: Terminal, label: '系统操作' },
  generation: { icon: Sparkles, label: 'AI 生成' },
  misc: { icon: Puzzle, label: '其他' },
}

/** 分组展示顺序（固定；无工具的分组不渲染） */
const GROUP_ORDER = ['desktop', 'browser', 'file', 'system', 'generation', 'misc']

/**
 * 工具注册名 → 中文动作名（纯展示映射；key 逐一来自 wf_tools 实际返回的工具全集：
 * 注册表 register_* + desktop/browser schema，已排除 WORKFLOW_TOOL_EXCLUDE 与
 * ui_maps_/experience_ 前缀族）。未命中的工具兜底显示英文注册名，不崩溃。
 */
const TOOL_LABELS: Record<string, string> = {
  // ── 桌面操作（desktop_ 前缀族）──
  desktop_mouse: '鼠标操作',
  desktop_mouse_drag: '鼠标拖拽',
  desktop_input: '键盘输入',
  desktop_screenshot: '屏幕截图',
  desktop_screen_size: '获取屏幕尺寸',
  desktop_windows_list: '列出窗口',
  desktop_window_activate: '激活窗口',
  desktop_window_screenshot: '窗口截图',
  desktop_window_move: '移动窗口',
  desktop_window_resize: '调整窗口大小',
  desktop_window_info: '获取窗口信息',
  desktop_vision: 'AI 识别截图内容',
  desktop_perceive: '定位界面元素',
  desktop_clipboard_clean: '清空剪贴板',
  desktop_clipboard_write: '写入剪贴板',
  desktop_find_image: '屏幕查找图片',
  desktop_find_color: '屏幕查找颜色',
  desktop_find_multi_color: '屏幕多点颜色定位',
  desktop_find_text: '屏幕查找文字',
  // ── 网页操作（browser_ 前缀族 + 网络访问三件套）──
  browser_navigate: '打开网页',
  browser_snapshot: '读取页面元素',
  browser_exec: '批量执行页面操作',
  browser_click: '点击元素',
  browser_type: '输入文本',
  browser_scroll: '滚动页面',
  browser_extract: '提取当前页正文',
  browser_screenshot: '网页截图',
  browser_close: '关闭浏览器',
  browser_evaluate: '执行页面脚本',
  browser_back: '后退一页',
  browser_forward: '前进一页',
  browser_wait_for: '等待元素出现',
  browser_cookies_get: '读取网页 Cookie',
  browser_cookies_set: '设置网页 Cookie',
  browser_import_cookies: '导入 Chrome Cookie',
  browser_upload_file: '上传文件到网页',
  browser_list_downloads: '列出下载文件',
  browser_new_tab: '新建标签页',
  browser_list_tabs: '列出标签页',
  browser_switch_tab: '切换标签页',
  web_search: '搜索网络',
  web_extract: '抓取网页正文',
  http_request: '发送 HTTP 请求',
  // ── 文件操作（注册表真实名，大小写敏感）──
  Read: '读取文件',
  Write: '写入文件',
  Edit: '编辑文件',
  Delete: '删除文件',
  Rename: '重命名 / 移动',
  Copy: '复制文件',
  CreateDir: '创建目录',
  RemoveDir: '删除目录',
  ListDir: '列出目录内容',
  FilesInfo: '查看文件信息',
  Append: '追加文件内容',
  Glob: '按模式查找文件',
  Grep: '搜索文件内容',
  Diff: '对比文件差异',
  // ── 系统操作（system_ / process_ 前缀族）──
  system_info: '查看系统信息',
  system_env_get: '读取环境变量',
  system_shell: '执行命令',
  system_sleep: '延时等待',
  process_list: '列出进程',
  process_kill: '结束进程',
  // ── AI 生成 ──
  image_generate: '生成图片',
  video_generate: '生成视频',
  // ── 其他（misc 兜底组）──
  video_subtitle_extract: '提取视频字幕',
  skill_query: '查询技能',
  skill_read: '读取技能详情',
  knowledge_search: '搜索知识库',
  wf_call: '调用子工作流',
}

interface ToolPaletteProps {
  /** 只读（运行中/旧格式）：禁用 hover 浮层、点击与拖拽 */
  disabled: boolean
  /** 点击工具创建步骤（插入位置由调用方按选中状态裁决） */
  onAdd: (tool: ToolSchema) => void
}

export function ToolPalette({ disabled, onAdd }: ToolPaletteProps) {
  // 与 Inspector 共享模块级缓存（loadToolsOnce 单例），不重复请求
  const [tools, setTools] = useState<ToolSchema[] | null | undefined>(undefined)
  const [openGroup, setOpenGroup] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    void loadToolsOnce().then(t => {
      if (alive) setTools(t)
    })
    return () => {
      alive = false
    }
  }, [])

  // 按 group 聚类；group 缺省（旧缓存/异常）或未知键一律归 misc
  const groups = useMemo(() => {
    const map = new Map<string, ToolSchema[]>()
    for (const t of tools ?? []) {
      const key = t.group && GROUP_META[t.group] ? t.group : 'misc'
      const list = map.get(key)
      if (list) list.push(t)
      else map.set(key, [t])
    }
    return GROUP_ORDER.filter(k => map.has(k)).map(k => ({ key: k, tools: map.get(k)! }))
  }, [tools])

  if (groups.length === 0) return null

  return (
    <div
      className={`wfc-palette${disabled ? ' is-disabled' : ''}`}
      onMouseLeave={() => setOpenGroup(null)}
    >
      {groups.map(g => {
        const meta = GROUP_META[g.key]
        const Icon = meta.icon
        return (
          <div
            key={g.key}
            className="wfc-palette-group"
            onMouseEnter={() => {
              if (!disabled) setOpenGroup(g.key)
            }}
          >
            <button type="button" className="wfc-palette-btn" disabled={disabled} title={meta.label}>
              <Icon size={15} />
            </button>
            {openGroup === g.key && !disabled && (
              <div className="wfc-palette-flyout">
                <div className="wfc-palette-flyout-head">
                  {meta.label} · {g.tools.length}
                </div>
                {g.tools.map(t => (
                  <button
                    key={t.name}
                    type="button"
                    className="wfc-palette-tool"
                    draggable
                    onDragStart={e => {
                      e.dataTransfer.setData(TOOL_DRAG_MIME, t.name)
                      e.dataTransfer.effectAllowed = 'copy'
                    }}
                    onClick={() => onAdd(t)}
                    title={t.description || t.name}
                  >
                    <span className="wfc-palette-tool-name">{TOOL_LABELS[t.name] ?? t.name}</span>
                    <span className="wfc-palette-tool-en">{t.name}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}
