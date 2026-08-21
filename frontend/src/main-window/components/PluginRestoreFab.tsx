/**
 * PluginRestoreFab.tsx — 最小化插件的悬浮恢复按钮（App Plugin 运行时最小化）
 *
 * 仅当宿主处于最小化态时由 App.tsx 渲染：
 * - fixed 定位于主窗口输入框 dock 左边缘外侧（几何对齐 .chat-input-dock：
 *   dock 宽 = clamp(800px, 80%, 1280px)，左边缘 = (100% - 宽)/2；按钮退到其外侧 12px）
 * - 32px 圆角方块（对齐市场图标的圆角方形态，radius-4），内显插件图标
 *   （/plugins/{id}/{icon} 经 mobile_server 端口，加载逻辑对齐 AppShellPage：
 *   status → ensure → 取实际 port），失败 fallback IconPuzzle
 * - 点击 → 恢复宿主可见（iframe 全程未卸载，插件 JS/状态不丢）
 * - 机制通用：任意插件 id 皆可，无硬编码特例
 */

import { useEffect, useState } from 'react'
import { useLanguage } from '../../locales'
import { IconPuzzle } from '../../ui/Icons'
import { pluginAppList, type PluginAppSummary } from '../lib/plugin-apps'
import { mobileServerStatus, mobileServerEnsure } from '../lib/api'

interface PluginRestoreFabProps {
  pluginId: string
  onRestore: () => void
}

export function PluginRestoreFab({ pluginId, onRestore }: PluginRestoreFabProps) {
  const { t } = useLanguage()
  const [plugin, setPlugin] = useState<PluginAppSummary | null>(null)
  const [port, setPort] = useState<number | null>(null)

  // 插件信息 + 插件伺服端口（复用 AppShellPage 的加载顺序：ensure 非持久化启动）
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        let status = await mobileServerStatus()
        if (!status?.running) {
          status = await mobileServerEnsure()
        }
        if (cancelled || !status) return
        setPort(status.port)
        const list = (await pluginAppList()) || []
        if (cancelled) return
        setPlugin(list.find(p => p.id === pluginId) ?? null)
      } catch {
        // 图标加载失败不阻断恢复功能（fallback IconPuzzle）
      }
    })()
    return () => {
      cancelled = true
    }
  }, [pluginId])

  const iconUrl =
    port !== null && plugin?.icon
      ? `http://127.0.0.1:${port}/plugins/${plugin.id}/${plugin.icon}`
      : null

  const label = `${t('plugins.restoreApp')} · ${plugin?.name ?? pluginId}`
  return (
    <button
      type="button"
      className="plugin-restore-fab"
      onClick={onRestore}
      aria-label={label}
      title={label}
    >
      {iconUrl ? (
        <img
          src={iconUrl}
          alt=""
          onError={e => {
            ;(e.target as HTMLImageElement).style.display = 'none'
          }}
        />
      ) : null}
      <IconPuzzle size={18} className="plugin-restore-fab-fallback" />
    </button>
  )
}
