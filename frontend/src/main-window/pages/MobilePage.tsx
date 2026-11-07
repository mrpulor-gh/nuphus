/**
 * MobilePage — 设置页「移动端本地连接」
 *
 * 2026-11-07 本地化调整：
 * 顶部拓扑 + 四个可展开列表项：
 *   设置密码 → 密码输入 + 保存 + 二维码
 *   局域网直连 → 直连地址 + 复制
 *   P2P 内网穿透 → Tailscale 自建指南（用户可选）
 *   自定义服务端 → 用户自建 relay-server（地址/密钥/显式启用）
 *
 * 官方中继暂时下线：手机默认且优先通过无线局域网直连；桌面“移动端”总开关
 * 只负责本地 mobile_server，不再隐式启动任何官方或自建远程线路。
 * 列表标题一律纯文字名称；点击展开，再点收起。
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { useLanguage } from '../../locales'
import { Section } from '../../ui/PageLayout'
import { QrCode } from '../../ui/QrCode'
import { IconCopy, IconCheck, IconChevronDown, IconChevronRight } from '../../ui/Icons'
import {
  mobileServerStart,
  mobileServerStop,
  mobileServerStatus,
  mobileTokenRegenerate,
  mobilePasswordSet,
  relayClientStatus,
  relayClientSetEnabled,
  relayClientUpdateNode,
  relayCallerTokenRotate,
  type MobileServerStatus,
  type RelayChannelState,
  type RelayClientStatus,
} from '../lib/api'
import { withPairToken } from './mobilePairing'
import '../../styles/mobile-panel.css'

/** 提取 Tauri IPC 错误的干净信息（去掉 "Error: IPC invoke xxx failed: " 前缀） */
function cleanIpcError(e: unknown): string {
  const s = String(e)
  const m = s.match(/failed:\s*(.+)$/)
  return m ? m[1].trim() : s
}

/** 通道状态 → 连线视觉态（真实映射：connected=通 / retrying=重连中 / fault=阻断 / 其余=关） */
type LineState = 'active' | 'warn' | 'err' | 'off'
function lineStateFor(ch: RelayChannelState | undefined, running: boolean): LineState {
  if (!running) return 'off'
  if (ch?.status === 'connected') return 'active'
  if (ch?.status === 'retrying') return 'warn'
  if (ch?.status === 'fault') return 'err'
  return 'off'
}

/** 可展开列表项 id */
type ListItemId = 'password' | 'lan' | 'p2p' | 'vps'

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

  // 列表展开状态：当前展开项（null=全收起）
  const [openItem, setOpenItem] = useState<ListItemId | null>('password')

  // 自定义服务端设置
  const [vpsUrl, setVpsUrl] = useState('')
  const [vpsToken, setVpsToken] = useState('')
  const [vpsPublicUrl, setVpsPublicUrl] = useState('')
  const [nodeSaving, setNodeSaving] = useState(false)
  const [nodeMsg, setNodeMsg] = useState<string | null>(null)

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
  const customServerOn = relay?.enabled ?? false
  const customServerReady =
    customServerOn &&
    relay?.state?.relay?.status === 'connected' &&
    relay?.state?.tunnel?.status === 'connected'
  const customServerRetrying =
    customServerOn &&
    (relay?.state?.relay?.status === 'retrying' ||
      relay?.state?.tunnel?.status === 'retrying')
  const customServerFault =
    customServerOn &&
    (relay?.state?.relay?.status === 'fault' || relay?.state?.tunnel?.status === 'fault')

  // 拓扑中间节点状态推导（真实状态映射，禁止伪造）
  const topo: 'off' | 'lan' | 'server' | 'retrying' | 'fault' = (() => {
    if (!running) return 'off'
    // 默认入口始终是无线局域网。只有本机没有可用 LAN 地址时，才展示
    // 用户显式启用的自建服务端状态。
    if (status?.lan_url) return 'lan'
    const relayCh = relay?.state?.relay
    const tunnelCh = relay?.state?.tunnel
    if (relayCh?.status === 'connected' || tunnelCh?.status === 'connected') return 'server'
    if (relayCh?.status === 'fault' || tunnelCh?.status === 'fault') return 'fault'
    if (relayCh?.status === 'retrying' || tunnelCh?.status === 'retrying') return 'retrying'
    return 'off'
  })()

  // 两条连线独立映射真实通道（LAN 直连时整链视为通）：
  //   PC → 中间 = 用户服务端任务通道；中间 → MOB = 隧道通道
  const lineClsServer = topo === 'lan' ? 'active' : lineStateFor(relay?.state?.relay, running)
  const lineClsTunnel = topo === 'lan' ? 'active' : lineStateFor(relay?.state?.tunnel, running)
  const midIcon = topo === 'off' ? '···' : topo === 'lan' ? 'LAN' : 'SRV'
  const midLabel =
    topo === 'lan'
      ? t('mobile.topoMidLabelLan')
      : topo === 'server'
        ? t('mobile.topoMidLabelServer')
        : topo === 'retrying'
          ? t('mobile.topoMidLabelRetrying')
          : topo === 'fault'
            ? t('mobile.topoMidLabelFault')
            : t('mobile.topoMidLabelOff')

  // 默认二维码永远是无线局域网配对入口，不等待、也不依赖任何远程线路。
  const lanPairUrl = useMemo(() => {
    if (!running || !status?.password_set || !status.lan_url || !status.token) return null
    return withPairToken(status.lan_url, status.token)
  }, [running, status])

  // 可选的自建服务端远程入口：只有用户显式启用并连接成功后才展示。
  const customPairUrl = useMemo(() => {
    if (!running || !status?.password_set || !customServerReady || !status?.token) return null
    if (!relay?.public_url) return null
    return withPairToken(relay.public_url, status.token)
  }, [running, status, customServerReady, relay?.public_url])

  // 自建服务端地址（展示用：去协议与端口，只留域名）
  const customServerHost = useMemo(() => {
    if (!relay?.public_url) return null
    try {
      return new URL(relay.public_url).hostname
    } catch {
      return relay.public_url
    }
  }, [relay])

  // 自建服务端已开但公网地址未就绪：2s 轮询直到地址出现
  useEffect(() => {
    if (!running || !customServerOn || customPairUrl) return
    const timer = setTimeout(() => void refresh(), 2000)
    return () => clearTimeout(timer)
  }, [running, customServerOn, customPairUrl, refresh])

  // 自建服务端状态实时刷新：启用期间 5s 轮询
  useEffect(() => {
    if (!running || !customServerOn) return
    const timer = window.setInterval(() => void refresh(), 5000)
    return () => window.clearInterval(timer)
  }, [running, customServerOn, refresh])

  /** 手机访问总开关：只控制本地 mobile_server；远程线路必须由用户单独启用。 */
  const handleToggle = useCallback(async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      if (running) {
        await mobileServerStop()
        if (customServerOn) {
          await relayClientSetEnabled(false)
        }
      } else {
        await mobileServerStart()
      }
      await refresh()
      window.setTimeout(() => void refresh(), 800)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }, [busy, running, customServerOn, refresh])

  /** 保存并显式启用用户自建服务端（public_url 留空 = 后端自动派生）。 */
  const handleSaveNode = useCallback(async () => {
    if (busy || nodeSaving) return
    setNodeSaving(true)
    setNodeMsg(null)
    try {
      const url = vpsUrl.trim()
      if (!url) {
        setNodeMsg(t('mobile.nodeUrlEmpty'))
        return
      }
      await relayClientUpdateNode(url, vpsToken.trim(), vpsPublicUrl.trim())
      await relayClientSetEnabled(true)
      setNodeMsg(t('mobile.nodeSaved'))
      await refresh()
      window.setTimeout(() => void refresh(), 1500)
    } catch (e) {
      setNodeMsg(cleanIpcError(e))
    } finally {
      setNodeSaving(false)
    }
  }, [busy, nodeSaving, vpsUrl, vpsToken, vpsPublicUrl, t, refresh])

  /** 断开设用户已启用的自建服务端，不影响本地无线局域网直连。 */
  const handleDisableCustomServer = useCallback(async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await relayClientSetEnabled(false)
      await refresh()
      window.setTimeout(() => void refresh(), 800)
    } catch (e) {
      setError(cleanIpcError(e))
    } finally {
      setBusy(false)
    }
  }, [busy, refresh])

  /** 轮换自建服务端调用凭据（caller_token） */
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
  }, [busy, refresh])

  /** 重置手机配对（mobile token 重新签发） */
  const handleRegenerate = useCallback(async () => {
    if (busy) return
    if (!window.confirm(t('mobile.resetPairingHint'))) return
    setBusy(true)
    setError(null)
    try {
      await mobileTokenRegenerate()
      await refresh()
    } catch (e) {
      setError(cleanIpcError(e))
    } finally {
      setBusy(false)
    }
  }, [busy, refresh])

  const handleCopyUrl = useCallback(async () => {
    if (!lanPairUrl) return
    try {
      await navigator.clipboard.writeText(lanPairUrl)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      /* 剪贴板不可用时静默失败 */
    }
  }, [lanPairUrl])

  const handleCopyLanAddress = useCallback(async () => {
    if (!status?.lan_url) return
    try {
      await navigator.clipboard.writeText(status.lan_url)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      /* 剪贴板不可用时静默失败 */
    }
  }, [status?.lan_url])

  const handleCopyRemoteUrl = useCallback(async () => {
    if (!customPairUrl) return
    try {
      await navigator.clipboard.writeText(customPairUrl)
      setCopiedRemote(true)
      window.setTimeout(() => setCopiedRemote(false), 1500)
    } catch {
      /* 剪贴板不可用时静默失败 */
    }
  }, [customPairUrl])

  /** 保存配对密码 */
  const handlePasswordSave = useCallback(async () => {
    const pwd = passwordInput.trim()
    if (!pwd || busy || passwordSaving) return
    setPasswordSaving(true)
    setPasswordError(null)
    try {
      await mobilePasswordSet(pwd)
      setPasswordInput('')
      await refresh()
      window.setTimeout(() => void refresh(), 800)
      window.setTimeout(() => void refresh(), 2500)
    } catch (e) {
      setPasswordError(cleanIpcError(e))
    } finally {
      setPasswordSaving(false)
    }
  }, [passwordInput, busy, passwordSaving, refresh])

  /** 列表项展开/收起切换 */
  const toggleItem = (id: ListItemId) => setOpenItem(v => (v === id ? null : id))

  /** 渲染列表头：纯文字标题 + 必填标记（可选） + 展开箭头 */
  const listHead = (id: ListItemId, title: string, required = false) => (
    <button type="button" className="mobile-list-head" onClick={() => toggleItem(id)}>
      <span className="mobile-list-title-wrap">
        <span className="mobile-list-title">{title}</span>
        {required && <span className="mobile-required">{t('mobile.required')}</span>}
      </span>
      {openItem === id ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />}
    </button>
  )

  return (
    <div className="mobile-panel">
      {/* ── 1. 总开关 + 当前状态 ── */}
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
        {/* 拓扑管线：PC → 中间节点 → MOB（状态直接显示在图形节点/连线颜色上） */}
        <div className="mobile-topology">
          <div className="mobile-topo-pipeline">
            <div className={`mobile-topo-node state-${running ? 'on' : 'off'}`}>
              <div
                className="mobile-topo-icon"
                title={running ? t('mobile.statusRunning') : t('mobile.statusStopped')}
              >
                PC
              </div>
              <span className="mobile-topo-label">{t('mobile.topoDeskLabel')}</span>
            </div>
            <div className={`mobile-topo-line ${lineClsServer}`} />
            <div className={`mobile-topo-node is-mid state-${topo}`}>
              <div className="mobile-topo-icon" title={midLabel}>
                {midIcon}
              </div>
              <span className="mobile-topo-label">{midLabel}</span>
            </div>
            <div className={`mobile-topo-line ${lineClsTunnel}`} />
            <div className={`mobile-topo-node state-${status?.token ? 'on' : 'off'}`}>
              <div
                className="mobile-topo-icon"
                title={status?.token ? t('mobile.paired') : t('mobile.unpaired')}
              >
                MOB
              </div>
              <span className="mobile-topo-label">{t('mobile.topoPhoneLabel')}</span>
            </div>
          </div>
        </div>
        {error && <div className="mobile-panel-error">{error}</div>}
      </Section>

      {/* ── 2. 设置与通道列表（点击展开） ── */}
      <div className="mobile-list">
        {/* ① 设置密码（配对，必填） */}
        <div className="mobile-list-item">
          {listHead('password', t('mobile.listPassword'), true)}
          {openItem === 'password' && (
            <div className="mobile-list-body">
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
              <p className="mobile-list-hint">
                {status?.password_set
                  ? t('mobile.pairPasswordSetHint')
                  : t('mobile.pairPasswordUnsetHint')}
              </p>

              {/* 扫码二维码（配对成功后出现） */}
              {!status?.password_set ? (
                <p className="mobile-pro-locked-text">{t('mobile.scanNeedPassword')}</p>
              ) : lanPairUrl ? (
                <div className="mobile-qr-split">
                  {/* 左：二维码 + 复制链接（并列紧凑块） */}
                  <div className="mobile-qr-block">
                    <QrCode value={lanPairUrl} size={150} />
                    <button
                      className="mobile-panel-url"
                      onClick={handleCopyUrl}
                      title={t('mobile.copyUrl')}
                    >
                      {copied ? <IconCheck size={13} /> : <IconCopy size={13} />}
                      <span>{t('mobile.copyLocalLink')}</span>
                    </button>
                  </div>
                  {/* 右：无线局域网连接说明 */}
                  <ol className="mobile-qr-steps">
                    <li>{t('mobile.qrStep1')}</li>
                    <li>{t('mobile.qrStep2')}</li>
                    <li>{t('mobile.qrStep3')}</li>
                  </ol>
                </div>
              ) : (
                <p className="mobile-pro-locked-text">{t('mobile.scanNeedLan')}</p>
              )}
            </div>
          )}
        </div>

        {/* ② 局域网直连 */}
        <div className="mobile-list-item">
          {listHead('lan', t('mobile.listLan'))}
          {openItem === 'lan' && (
            <div className="mobile-list-body">
              <p className="mobile-list-hint">{t('mobile.routeLanDesc')}</p>
              {status?.lan_url ? (
                <div className="mobile-lan-addr">
                  <span className="mobile-panel-address" title={status.lan_url}>
                    {status.lan_url}
                  </span>
                  <button
                    className="mobile-backup-copy"
                    onClick={handleCopyLanAddress}
                    title={t('mobile.copyAddress')}
                  >
                    {copied ? <IconCheck size={13} /> : <IconCopy size={13} />}
                  </button>
                </div>
              ) : (
                <p className="mobile-list-hint">{t('mobile.routeStateOff')}</p>
              )}
            </div>
          )}
        </div>

        {/* ③ P2P 内网穿透（用户可选自建） */}
        <div className="mobile-list-item">
          {listHead('p2p', t('mobile.listP2P'))}
          {openItem === 'p2p' && (
            <div className="mobile-list-body">
              <p className="mobile-list-hint">{t('mobile.routeP2PDesc')}</p>
              <p className="mobile-route-guide-title">{t('mobile.routeP2PGuideTitle')}</p>
              <ol className="mobile-route-guide-list">
                <li>{t('mobile.routeP2PGuideStep1')}</li>
                <li>{t('mobile.routeP2PGuideStep2')}</li>
                <li>{t('mobile.routeP2PGuideStep3')}</li>
              </ol>
              <p className="mobile-route-guide-note">{t('mobile.routeP2PGuideNote')}</p>
            </div>
          )}
        </div>

        {/* ④ 自定义服务端（可选，默认关闭） */}
        <div className="mobile-list-item">
          {listHead('vps', t('mobile.listVps'))}
          {openItem === 'vps' && (
            <div className="mobile-list-body">
              <p className="mobile-list-hint">{t('mobile.routeVpsDesc')}</p>
              <label className="mobile-node-label">{t('mobile.nodeUrlLabel')}</label>
              <input
                className="mobile-node-input"
                value={vpsUrl}
                placeholder="wss://your-server.example.com"
                onChange={e => {
                  setVpsUrl(e.target.value)
                  setNodeMsg(null)
                }}
              />
              <label className="mobile-node-label">{t('mobile.nodeTokenLabel')}</label>
              <input
                className="mobile-node-input"
                type="password"
                value={vpsToken}
                placeholder="device token"
                onChange={e => {
                  setVpsToken(e.target.value)
                  setNodeMsg(null)
                }}
              />
              <label className="mobile-node-label">{t('mobile.nodePublicUrlLabel')}</label>
              <input
                className="mobile-node-input"
                value={vpsPublicUrl}
                placeholder="https://relay.yourdomain.com"
                onChange={e => {
                  setVpsPublicUrl(e.target.value)
                  setNodeMsg(null)
                }}
              />
              <p className="mobile-list-hint">{t('mobile.nodePublicUrlHint')}</p>
              <button
                type="button"
                className="mobile-node-save"
                disabled={busy || nodeSaving}
                onClick={handleSaveNode}
              >
                {nodeSaving ? t('mobile.nodeSaving') : t('mobile.nodeSave')}
              </button>
              {customServerOn && (
                <button
                  type="button"
                  className="mobile-node-reset"
                  disabled={busy || nodeSaving}
                  onClick={handleDisableCustomServer}
                >
                  {t('mobile.nodeDisable')}
                </button>
              )}
              <div className="mobile-relay-row">
                <span className="mobile-list-hint">{t('mobile.relayServerAddr')}</span>
                <span className="mobile-panel-address" title={relay?.public_url ?? undefined}>
                  {customServerHost ?? t('mobile.routeStateOff')}
                </span>
              </div>
              {relay?.device_id && (
                <div className="mobile-relay-row">
                  <span className="mobile-list-hint">{t('mobile.deviceIdLabel')}</span>
                  <span className="mobile-panel-address" title={relay.device_id}>
                    {relay.device_id}
                  </span>
                </div>
              )}
              <p className="mobile-list-hint">
                {!customServerOn
                  ? t('mobile.routeStateOff')
                  : customServerReady
                    ? t('mobile.relayChanConnected')
                    : customServerRetrying
                      ? t('mobile.relayChanRetrying', String(relay?.state?.relay?.attempts ?? 0))
                      : customServerFault
                        ? t('mobile.relayChanFault')
                        : t('mobile.routeStateOff')}
              </p>
              {customServerOn &&
                customServerFault &&
                [relay?.state?.relay?.reason, relay?.state?.tunnel?.reason]
                  .filter(Boolean)
                  .some(r => (r ?? '').includes('403') || (r ?? '').includes('401')) && (
                  <p className="mobile-list-warn">{t('mobile.relayAuthWarn')}</p>
                )}
              {customPairUrl && (
                <div className="mobile-qr-split">
                  <div className="mobile-qr-block">
                    <QrCode value={customPairUrl} size={150} />
                    <button
                      className="mobile-panel-url"
                      onClick={handleCopyRemoteUrl}
                      title={t('mobile.copyUrl')}
                    >
                      {copiedRemote ? <IconCheck size={13} /> : <IconCopy size={13} />}
                      <span>{t('mobile.copyRemoteLink')}</span>
                    </button>
                  </div>
                  <p className="mobile-list-hint">{t('mobile.customServerQrHint')}</p>
                </div>
              )}
              {nodeMsg && <div className="mobile-node-msg">{nodeMsg}</div>}
            </div>
          )}
        </div>
      </div>

      {/* ── 3. 维护（低频操作） ── */}
      <Section title={t('mobile.maintainTitle')}>
        <div className="mobile-ops">
          <button
            type="button"
            className="mobile-panel-reset"
            disabled={busy || !customServerOn}
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
      </Section>
    </div>
  )
}
