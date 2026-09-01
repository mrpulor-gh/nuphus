/**
 * MobilePage — 设置页「网络控制中心」
 *
 * 2026-08-31 重构 v8（按大王拍板：列表式点击展开，一眼看懂）：
 * 顶部拓扑 + 五个可展开列表项：
 *   设置密码 → 密码输入 + 保存 + 二维码
 *   局域网直连 → 直连地址 + 复制
 *   P2P 内网穿透 → Tailscale 自建指南（用户可选）
 *   官方中继 → 中继服务器地址 + 通道状态
 *   自建 VPS 节点 → 仅自定义配置表单（地址/密钥/保存）
 * 列表标题一律纯文字名称；点击展开，再点收起。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
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
  relayClientResetOfficial,
  relayCallerTokenRotate,
  type MobileServerStatus,
  type RelayChannelState,
  type RelayClientStatus,
} from '../lib/api'
import '../../styles/mobile-panel.css'

/** 官方中继默认地址（与 relay_client.rs DEFAULT_RELAY_URL 一致；token 由构建/运行注入，前端不持有） */
const OFFICIAL_RELAY_URL = 'wss://relay.nuphus.com'

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
type ListItemId = 'password' | 'lan' | 'p2p' | 'relay' | 'vps'

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

  // 中继节点设置
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
  const relayOn = relay?.enabled ?? false

  // 拓扑中间节点状态推导（真实状态映射，禁止伪造）
  const topo: 'off' | 'lan' | 'relay' | 'retrying' | 'fault' = (() => {
    if (!running) return 'off'
    const relayCh = relay?.state?.relay
    const tunnelCh = relay?.state?.tunnel
    if (relayCh?.status === 'connected' || tunnelCh?.status === 'connected') return 'relay'
    if (relayCh?.status === 'fault' || tunnelCh?.status === 'fault') return 'fault'
    if (relayCh?.status === 'retrying' || tunnelCh?.status === 'retrying') return 'retrying'
    if (status?.lan_url) return 'lan'
    return 'off'
  })()

  // 两条连线独立映射真实通道（LAN 直连时整链视为通）：
  //   PC → 中间 = relay 通道；中间 → MOB = tunnel 通道
  const lineClsRelay = topo === 'lan' ? 'active' : lineStateFor(relay?.state?.relay, running)
  const lineClsTunnel = topo === 'lan' ? 'active' : lineStateFor(relay?.state?.tunnel, running)
  const midIcon = topo === 'off' ? '···' : topo === 'lan' ? 'LAN' : 'REL'
  const midLabel =
    topo === 'lan'
      ? t('mobile.topoMidLabelLan')
      : topo === 'relay'
        ? t('mobile.topoMidLabelRelay')
        : topo === 'retrying'
          ? t('mobile.topoMidLabelRetrying')
          : topo === 'fault'
            ? t('mobile.topoMidLabelFault')
            : t('mobile.topoMidLabelOff')

  // 扫码入口地址：实例级稳定（不随状态轮询跳动）
  const remoteUrlRef = useRef<string | null>(null)
  useEffect(() => {
    remoteUrlRef.current = null
  }, [running, status?.password_set, relay?.public_url])
  const remoteUrl = useMemo(() => {
    if (!running || !status?.password_set) return null
    if (!relay?.public_url) return null
    if (remoteUrlRef.current === null) {
      const sep = relay.public_url.includes('?') ? '&' : '?'
      remoteUrlRef.current = `${relay.public_url}${sep}t=${Date.now()}`
    }
    return remoteUrlRef.current
  }, [running, status, relay])

  // 官方中继服务器地址（展示用：去协议与端口，只留域名）
  const relayHost = useMemo(() => {
    if (!relay?.public_url) return null
    try {
      return new URL(relay.public_url).hostname
    } catch {
      return relay.public_url
    }
  }, [relay])

  // 中继已开但公网地址未就绪：2s 轮询直到地址出现
  useEffect(() => {
    if (!running || !relayOn || remoteUrl) return
    const timer = setTimeout(() => void refresh(), 2000)
    return () => clearTimeout(timer)
  }, [running, relayOn, remoteUrl, refresh])

  // 中继通道状态实时刷新：中继启用期间 5s 轮询
  useEffect(() => {
    if (!running || !relayOn) return
    const timer = window.setInterval(() => void refresh(), 5000)
    return () => window.clearInterval(timer)
  }, [running, relayOn, refresh])

  /** 手机访问总开关：开 = 启动服务 + 接入中继；关 = 断中继 + 停服 */
  const handleToggle = useCallback(async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      if (running) {
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

  /** 保存自建 VPS 节点：调后端持久化 + 热重启连接（public_url 留空 = 后端自动派生） */
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
      setNodeMsg(t('mobile.nodeSaved'))
      await refresh()
      window.setTimeout(() => void refresh(), 1500)
    } catch (e) {
      setNodeMsg(cleanIpcError(e))
    } finally {
      setNodeSaving(false)
    }
  }, [busy, nodeSaving, vpsUrl, vpsToken, vpsPublicUrl, t, refresh])

  /** 恢复官方中继节点：url/public_url/token 重置官方默认 + 热重启 */
  const handleResetOfficial = useCallback(async () => {
    if (busy || nodeSaving) return
    if (!window.confirm(t('mobile.nodeResetOfficialConfirm'))) return
    setNodeSaving(true)
    setNodeMsg(null)
    try {
      await relayClientResetOfficial()
      setVpsUrl('')
      setVpsToken('')
      setVpsPublicUrl('')
      setNodeMsg(t('mobile.nodeResetOfficialDone'))
      await refresh()
      window.setTimeout(() => void refresh(), 1500)
    } catch (e) {
      setNodeMsg(cleanIpcError(e))
    } finally {
      setNodeSaving(false)
    }
  }, [busy, nodeSaving, t, refresh])

  /** 遗留状态修复路径：服务在跑但中继未开——扫码入口内一键接入中继 */
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

  /** 轮换中继调用凭据（caller_token） */
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
    if (!status?.lan_url) return
    try {
      await navigator.clipboard.writeText(status.lan_url)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      /* 剪贴板不可用时静默失败 */
    }
  }, [status])

  const handleCopyRemoteUrl = useCallback(async () => {
    if (!remoteUrl) return
    try {
      await navigator.clipboard.writeText(remoteUrl)
      setCopiedRemote(true)
      window.setTimeout(() => setCopiedRemote(false), 1500)
    } catch {
      /* 剪贴板不可用时静默失败 */
    }
  }, [remoteUrl])

  /** 保存配对密码 */
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
      window.setTimeout(() => void refresh(), 800)
      window.setTimeout(() => void refresh(), 2500)
    } catch (e) {
      setPasswordError(cleanIpcError(e))
    } finally {
      setPasswordSaving(false)
    }
  }, [passwordInput, busy, passwordSaving, relayOn, refresh])

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
            <div className={`mobile-topo-line ${lineClsRelay}`} />
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
                  <QrCode value={remoteUrl} size={150} />
                  <span className="mobile-qr-caption">{t('mobile.qrCaption')}</span>
                  <button
                    className="mobile-panel-url"
                    onClick={handleCopyRemoteUrl}
                    title={t('mobile.copyUrl')}
                  >
                    {copiedRemote ? <IconCheck size={13} /> : <IconCopy size={13} />}
                    <span>{t('mobile.copyRemoteLink')}</span>
                  </button>
                </div>
              ) : (
                <p className="mobile-pro-locked-text">{t('mobile.qrConnecting')}</p>
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
                    onClick={handleCopyUrl}
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

        {/* ④ 官方中继 */}
        <div className="mobile-list-item">
          {listHead('relay', t('mobile.listRelay'))}
          {openItem === 'relay' && (
            <div className="mobile-list-body">
              <div className="mobile-relay-row">
                <span className="mobile-list-hint">{t('mobile.relayServerAddr')}</span>
                <span className="mobile-panel-address" title={relay?.public_url ?? undefined}>
                  {relayHost ?? t('mobile.routeStateOff')}
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
                {!running
                  ? t('mobile.routeStateOff')
                  : topo === 'relay'
                    ? t('mobile.relayChanConnected')
                    : topo === 'retrying'
                      ? t('mobile.relayChanRetrying', String(relay?.state?.relay?.attempts ?? 0))
                      : topo === 'fault'
                        ? t('mobile.relayChanFault')
                        : t('mobile.routeStateOff')}
              </p>
              {/* 鉴权类故障自愈提示（issue #7）：403/401 = token 与中继不匹配 */}
              {topo === 'fault' &&
                [relay?.state?.relay?.reason, relay?.state?.tunnel?.reason]
                  .filter(Boolean)
                  .some(r => (r ?? '').includes('403') || (r ?? '').includes('401')) && (
                  <p className="mobile-list-warn">{t('mobile.relayAuthWarn')}</p>
                )}
            </div>
          )}
        </div>

        {/* ⑤ 自建 VPS 节点 */}
        <div className="mobile-list-item">
          {listHead('vps', t('mobile.listVps'))}
          {openItem === 'vps' && (
            <div className="mobile-list-body">
              <p className="mobile-list-hint">{t('mobile.routeVpsDesc')}</p>
              <label className="mobile-node-label">{t('mobile.nodeUrlLabel')}</label>
              <input
                className="mobile-node-input"
                value={vpsUrl}
                placeholder="wss://119.29.x.x:18080"
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
              <button
                type="button"
                className="mobile-node-reset"
                disabled={busy || nodeSaving}
                onClick={handleResetOfficial}
              >
                {t('mobile.nodeResetOfficial')}
              </button>
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
      </Section>
    </div>
  )
}