/**
 * MobilePage — 设置页「移动端」面板
 *
 * 2026-08-17 重构 v2（按大王拍板）：
 * 机制事实——手机访问只有一个服务实体 mobile_server（默认 :18772，伺服移动 UI + 配对）；
 * 中继（relay_client）不独立服务，只是把官方中继的公网流量经隧道转发到 127.0.0.1:18772
 * （relay_client.rs TUNNEL_LOCAL_ADDR 硬编码）。中继就是分发主逻辑，本地直连只是备用地址。
 *
 * 设计：一个开关走全流程——开 = 启动服务 + 接入中继；关 = 停服 + 断中继。
 * 流程即设置项：每个步骤是一个块状模块（块内自带控件），不用线条分割的 FormRow。
 *   1. 手机访问 —— 总开关 + 模块①配对密码 + 模块②扫码入口（含备用地址）+ 维护操作
 *   2. 双端状态 —— 桌面端 + 手机端
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { useLanguage } from '../../locales'
import { Section } from '../../ui/PageLayout'
import { QrCode } from '../../ui/QrCode'
import { IconCopy, IconCheck, IconMonitor, IconSmartphone } from '../../ui/Icons'
import {
  mobileServerStart,
  mobileServerStop,
  mobileServerStatus,
  mobileTokenRegenerate,
  mobilePasswordSet,
  relayClientStatus,
  relayClientSetEnabled,
  relayCallerTokenRotate,
  type MobileServerStatus,
  type RelayClientStatus,
} from '../lib/api'
import '../../styles/mobile-panel.css'

/** 提取 Tauri IPC 错误的干净信息（去掉 "Error: IPC invoke xxx failed: " 前缀） */
function cleanIpcError(e: unknown): string {
  const s = String(e)
  const m = s.match(/failed:\s*(.+)$/)
  return m ? m[1].trim() : s
}

export function MobilePage() {
  const { t } = useLanguage()
  const [status, setStatus] = useState<MobileServerStatus | null>(null)
  const [relay, setRelay] = useState<RelayClientStatus | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [passwordInput, setPasswordInput] = useState('')
  const [passwordSaving, setPasswordSaving] = useState(false)
  const [passwordError, setPasswordError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [copiedRemote, setCopiedRemote] = useState(false)

  const refresh = useCallback(async () => {
    try {
      const s = await mobileServerStatus()
      if (!s) {
        setError('状态查询失败（后端未响应）')
        return
      }
      setStatus(s)
      setError(null)
    } catch (e) {
      setError(String(e))
    }
    try {
      setRelay(await relayClientStatus())
    } catch {
      setRelay(null)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const running = status?.running ?? false
  const relayOn = relay?.enabled ?? false

  // 扫码入口地址（Nuphus 官方中继公网地址；二维码只承载地址，不带 token）。
  // 附时间戳噪声 t=：保证每次启动生成的扫码 URL 全局唯一——Safari 等浏览器对
  // 无缓存头的响应可能启发式缓存（历史故障窗口曾把引导页钉死在带标记入口上，
  // 换浏览器正常、本浏览器永远异常），唯一 URL 强制真实回源。中继解析 ?device=
  // 不受额外参数影响。
  const remoteUrl = useMemo(() => {
    if (!running || !status?.password_set) return null
    if (!relay?.public_url) return null
    const sep = relay.public_url.includes('?') ? '&' : '?'
    return `${relay.public_url}${sep}t=${Date.now()}`
  }, [running, status, relay])

  // 中继地址展示（去协议与端口，只留域名）
  const relayHost = useMemo(() => {
    if (!remoteUrl) return null
    try {
      return new URL(remoteUrl).hostname
    } catch {
      return remoteUrl
    }
  }, [remoteUrl])

  // 中继已开但公网地址未就绪（隧道连接中）：2s 轮询直到地址出现
  useEffect(() => {
    if (!running || !relayOn || remoteUrl) return
    const timer = setTimeout(() => void refresh(), 2000)
    return () => clearTimeout(timer)
  }, [running, relayOn, remoteUrl, refresh])

  /** 手机访问总开关（唯一开关）：开 = 启动服务 + 接入中继；关 = 断中继 + 停服。
   *  中继接入失败（配置不完整等）不回滚服务——局域网备用地址仍可用，错误如实展示。 */
  const handleToggle = useCallback(async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      if (running) {
        try {
          await relayClientSetEnabled(false)
        } catch {
          /* 关中继失败不阻塞停服 */
        }
        await mobileServerStop()
      } else {
        await mobileServerStart()
        try {
          await relayClientSetEnabled(true)
        } catch (e) {
          setError(cleanIpcError(e))
        }
      }
      await refresh()
      window.setTimeout(() => void refresh(), 800)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }, [busy, running, refresh])

  /** 遗留状态修复路径：服务在跑但中继未开（旧版本用户）——扫码入口内一键接入中继 */
  const handleEnableRelay = useCallback(async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await relayClientSetEnabled(true)
      await refresh()
      window.setTimeout(() => void refresh(), 800)
    } catch (e) {
      setError(cleanIpcError(e))
    } finally {
      setBusy(false)
    }
  }, [busy, refresh])

  /** 轮换中继调用凭据（caller_token）：确认后调服务端管理端点，旧凭据即刻失效，
   *  已配对手机外网访问需重新扫码（对齐 mobile token 重置的交互模型）。 */
  const handleRotateCallerToken = useCallback(async () => {
    if (busy) return
    if (!window.confirm(t('mobile.relayRotateConfirm'))) return
    setBusy(true)
    setError(null)
    try {
      await relayCallerTokenRotate()
      await refresh()
      window.setTimeout(() => void refresh(), 800)
    } catch (e) {
      setError(cleanIpcError(e))
    } finally {
      setBusy(false)
    }
  }, [busy, refresh, t])

  /** 重置配对：重签 token 并持久化，已配对手机立即失效（重新输密码配对） */
  const handleRegenerate = useCallback(async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await mobileTokenRegenerate()
      await refresh()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }, [busy, refresh])

  /** 复制备用地址（纯地址，不含 token） */
  const handleCopyUrl = useCallback(async () => {
    if (!status?.lan_url) return
    try {
      await navigator.clipboard.writeText(status.lan_url)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      /* 剪贴板不可用时静默 */
    }
  }, [status])

  /** 复制扫码入口地址（纯地址，不含 token） */
  const handleCopyRemoteUrl = useCallback(async () => {
    if (!remoteUrl) return
    try {
      await navigator.clipboard.writeText(remoteUrl)
      setCopiedRemote(true)
      setTimeout(() => setCopiedRemote(false), 1500)
    } catch {
      /* 剪贴板不可用时静默 */
    }
  }, [remoteUrl])

  /** 保存配对密码（设置/修改）；成功清空输入并刷新。
   *  中继未开时自动接入——扫码二维码是唯一配对入口，用户设完密码就应看到码，
   *  不允许停留在「需要接入中继」的死角（实测投诉：设完密码无码可扫）。 */
  const handlePasswordSave = useCallback(async () => {
    const pwd = passwordInput.trim()
    if (!pwd || busy || passwordSaving) return
    setPasswordSaving(true)
    setPasswordError(null)
    try {
      await mobilePasswordSet(pwd)
      setPasswordInput('')
      if (!relayOn) {
        try {
          await relayClientSetEnabled(true)
        } catch (e) {
          setError(cleanIpcError(e))
        }
      }
      await refresh()
      // 隧道建立有延迟（公网地址就绪前轮询补拉），确保二维码尽快出现
      window.setTimeout(() => void refresh(), 800)
      window.setTimeout(() => void refresh(), 2500)
    } catch (e) {
      setPasswordError(cleanIpcError(e))
    } finally {
      setPasswordSaving(false)
    }
  }, [passwordInput, busy, passwordSaving, relayOn, refresh])

  return (
    <div className="mobile-panel">
      {/* ── 1. 手机访问（唯一开关 + 流程即模块） ── */}
      <Section
        title={t('mobile.title')}
        description={t('mobile.desc')}
        actions={
          <button
            type="button"
            role="switch"
            aria-checked={running}
            className="switch"
            disabled={busy}
            onClick={handleToggle}
          />
        }
      >
        <div className="mobile-status-row">
          <span className={`mobile-status-dot ${running ? 'mobile-status-dot-on' : ''}`} />
          <span className="mobile-status-text">
            {running ? t('mobile.runningHint') : t('mobile.stoppedHint')}
          </span>
        </div>

        {running && (
          <div className="mobile-flow">
            {/* 模块①：配对密码 */}
            <div className="mobile-block">
              <div className="mobile-block-head">
                <span className="mobile-block-num">1</span>
                <span className="mobile-block-title">{t('mobile.blockPassword')}</span>
              </div>
              <p className="mobile-block-hint">
                {status?.password_set
                  ? t('mobile.pairPasswordSetHint')
                  : t('mobile.pairPasswordUnsetHint')}
              </p>
              <div className="mobile-password-row">
                <input
                  className="mobile-panel-password"
                  type="password"
                  value={passwordInput}
                  placeholder={
                    status?.password_set
                      ? t('mobile.pairPasswordPlaceholderSet')
                      : t('mobile.pairPasswordPlaceholder')
                  }
                  disabled={busy || passwordSaving}
                  onChange={e => {
                    setPasswordInput(e.target.value)
                    setPasswordError(null)
                  }}
                  aria-label={t('mobile.pairPassword')}
                />
                <button
                  type="button"
                  className="mobile-panel-save"
                  disabled={busy || passwordSaving || !passwordInput.trim()}
                  onClick={handlePasswordSave}
                >
                  {passwordSaving ? t('mobile.pairPasswordSaving') : t('mobile.pairPasswordSave')}
                </button>
              </div>
              {passwordError && <div className="mobile-password-error">{passwordError}</div>}
            </div>

            {/* 模块②：扫码入口（中继公网地址，异地可用；内含局域网备用地址） */}
            <div className="mobile-block">
              <div className="mobile-block-head">
                <span className="mobile-block-num">2</span>
                <span className="mobile-block-title">{t('mobile.blockScan')}</span>
              </div>
              {!status?.password_set ? (
                <p className="mobile-pro-locked-text">{t('mobile.scanNeedPassword')}</p>
              ) : !relayOn ? (
                <div className="mobile-scan-repair">
                  <p className="mobile-pro-locked-text">{t('mobile.scanNeedRelay')}</p>
                  <button
                    type="button"
                    className="mobile-panel-save"
                    disabled={busy}
                    onClick={handleEnableRelay}
                  >
                    {t('mobile.relayEnableBtn')}
                  </button>
                </div>
              ) : remoteUrl ? (
                <div className="mobile-qr-block">
                  {relayHost && (
                    <div className="mobile-relay-host">
                      <span className="mobile-relay-host-label">{t('mobile.relayHost')}</span>
                      <span className="mobile-relay-host-value">{relayHost}</span>
                    </div>
                  )}
                  <QrCode value={remoteUrl} size={150} />
                  <span className="mobile-qr-caption">{t('mobile.qrCaption')}</span>
                  <p className="mobile-relay-wave-note">{t('mobile.relayWaveNote')}</p>
                  <button
                    className="mobile-panel-url"
                    onClick={handleCopyRemoteUrl}
                    title={t('mobile.copyUrl')}
                  >
                    {copiedRemote ? <IconCheck size={13} /> : <IconCopy size={13} />}
                    <span>{t('mobile.copyRemoteLink')}</span>
                  </button>
                  <p className="mobile-relay-guide-text">{t('mobile.relayShortcutGuide')}</p>
                </div>
              ) : (
                <p className="mobile-pro-locked-text">{t('mobile.qrConnecting')}</p>
              )}
            </div>

            {/* 维护操作（低频，安静收尾） */}
            <div className="mobile-ops">
              <button
                type="button"
                className="mobile-panel-reset"
                disabled={busy}
                title={t('mobile.relayRotateHint')}
                onClick={handleRotateCallerToken}
              >
                {t('mobile.relayRotate')}
              </button>
              <button
                type="button"
                className="mobile-panel-reset"
                disabled={busy}
                title={t('mobile.resetPairingHint')}
                onClick={handleRegenerate}
              >
                {t('mobile.resetPairingBtn')}
              </button>
            </div>
          </div>
        )}
        {error && <div className="mobile-panel-error">{error}</div>}
      </Section>

      {/* ── 2. 双端状态（SVG 图标 + CSS 连接线） ── */}
      <Section title={t('mobile.dualTitle')}>
        <div className="mobile-dual">
          <div className="mobile-device">
            <div className={`mobile-device-icon ${running ? 'is-on' : ''}`}>
              <IconMonitor size={20} />
            </div>
            <div className="mobile-device-name">{t('mobile.desktopLabel')}</div>
            <div className="mobile-device-meta">
              {t('mobile.localServiceLabel')}{' '}
              {running ? t('mobile.statusRunning') : t('mobile.statusStopped')}
            </div>
          </div>
          <div className={`mobile-link-line ${running && status?.token ? 'is-connected' : ''}`} />
          <div className="mobile-device">
            <div className={`mobile-device-icon ${status?.token ? 'is-on' : ''}`}>
              <IconSmartphone size={20} />
            </div>
            <div className="mobile-device-name">{t('mobile.phoneLabel')}</div>
            <div className="mobile-device-meta">
              {t('mobile.pairStatusLabel')}{' '}
              {status?.token ? t('mobile.paired') : t('mobile.unpaired')}
            </div>
          </div>
        </div>
      </Section>

      {/* ── 3. 局域网直连（最终回退，弱化展示但说明清楚） ── */}
      {status?.lan_url && (
        <Section title={t('mobile.lanFallbackTitle')}>
          <div className="mobile-lan-fallback">
            <span className="mobile-lan-fallback-desc">{t('mobile.lanFallbackDesc')}</span>
            <span className="mobile-panel-address" title={status.lan_url}>
              {status.lan_url}
            </span>
            <button
              className="mobile-backup-copy"
              onClick={handleCopyUrl}
              title={t('mobile.copyAddress')}
            >
              {copied ? <IconCheck size={13} /> : <IconCopy size={13} />}
            </button>
          </div>
          <p className="mobile-lan-fallback-note">{t('mobile.lanFallbackNote')}</p>
        </Section>
      )}
    </div>
  )
}
