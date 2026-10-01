import { useCallback, useEffect, useState } from 'react'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { getVersion } from '@tauri-apps/api/app'
import { relaunch } from '@tauri-apps/plugin-process'
import { useLanguage } from '../../locales'
import { Button } from '../../ui/Button'
import { IconRefresh } from '../../ui/Icons'
import { MarkdownInline } from '../chat/MarkdownContent'
import { getChangelog } from '../lib/api'
import { countSectionItems, parseChangelogSection, type ChangelogSection } from '../lib/changelog'
import '../../styles/update.css'

/**
 * 把 updater 抛出的未知异常变成「一段人话 + 原始错误」。
 *
 * 2026-09-30 起因：有用户反馈换用镜像下载渠道后自动更新失效，而我们拿不到任何
 * 线索 —— 原先两处 catch 都不看错误对象，只回一句通用文案。
 * Tauri updater 内部会明确区分 Network(status) / Minisign(验签) / TargetNotFound
 * 等，这些是定位问题的唯一依据，被替换掉就只剩猜。
 *
 * 输出 = 分类建议 + 截断后的原始错误体。原始错误体必须保留：关键词匹配永远追不齐
 * 服务商/插件升级后的措辞，漏匹配时用户和维护者仍能看懂真实原因。
 */
function describeUpdateError(e: unknown, t: (k: string) => string): string {
  const raw = e instanceof Error ? e.message : String(e ?? '')
  const lower = raw.toLowerCase()
  const snippet = raw.length > 240 ? `${raw.slice(0, 240)}…` : raw

  let advice: string
  if (lower.includes('signature') || lower.includes('minisign') || lower.includes('pubkey')) {
    // 验签失败属安全问题：不能静默重试，必须让用户看到并反馈
    advice = t('update.errVerify')
  } else if (
    lower.includes('network') ||
    lower.includes('status') ||
    lower.includes('timeout') ||
    lower.includes('timed out') ||
    lower.includes('connection') ||
    lower.includes('dns') ||
    lower.includes('tls') ||
    lower.includes('redirect')
  ) {
    advice = t('update.errNetwork')
  } else if (lower.includes('target') || lower.includes('platform')) {
    advice = t('update.errPlatform')
  } else if (lower.includes('permission') || lower.includes('denied') || lower.includes('access')) {
    advice = t('update.errPermission')
  } else {
    advice = t('update.errUnknown')
  }
  return snippet ? `${advice}\n${snippet}` : advice
}

/** 本版更新内容区块的状态（四态互斥，避免多个 flag 互相打架） */
type ChangelogState =
  /** 版本号或 CHANGELOG 尚未就绪 */
  | { status: 'loading' }
  | { status: 'ready'; section: ChangelogSection }
  /** CHANGELOG 里没有当前版本段落（或段落为空） */
  | { status: 'empty' }
  /** 命令调用失败 */
  | { status: 'unavailable' }

export function UpdatePage() {
  const { t } = useLanguage()
  const [status, setStatus] = useState<
    'idle' | 'checking' | 'available' | 'latest' | 'downloading' | 'error'
  >('idle')
  const [update, setUpdate] = useState<Update | null>(null)
  const [progress, setProgress] = useState(0)
  const [error, setError] = useState('')
  const [, setDownloaded] = useState(0)
  const [currentVersion, setCurrentVersion] = useState('—')
  const [changelog, setChangelog] = useState<ChangelogState>({ status: 'loading' })

  useEffect(() => {
    let cancelled = false
    void (async () => {
      // 版本号同时驱动「当前版本」展示与本版段落定位；取不到就无从定位（保持空态而非伪版本号）
      let version = ''
      try {
        version = await getVersion()
      } catch {
        // 与既有行为一致：拿不到版本号时静默（界面仍显示占位符）
      }
      if (cancelled) return
      if (version) setCurrentVersion(version)

      try {
        const raw = await getChangelog()
        if (cancelled) return
        const section = version ? parseChangelogSection(raw, version) : null
        setChangelog(
          section && countSectionItems(section) > 0
            ? { status: 'ready', section }
            : { status: 'empty' },
        )
      } catch {
        if (!cancelled) setChangelog({ status: 'unavailable' })
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const checkVersion = useCallback(async () => {
    setStatus('checking')
    setError('')
    try {
      const found = await check()
      setUpdate(found)
      setStatus(found ? 'available' : 'latest')
    } catch (e) {
      // 必须把真实错误带出来：endpoint 已配镜像 + 权威源两路，仅凭"暂时无法获取"
      // 分不清是镜像挂了、权威源也不通、还是清单本身有问题（见 describeUpdateError）
      setError(describeUpdateError(e, t))
      setStatus('error')
    }
  }, [t])

  const install = useCallback(async () => {
    if (!update) return
    setStatus('downloading')
    setProgress(0)
    setDownloaded(0)
    try {
      let contentLength: number | null = null
      let downloadedBytes = 0
      await update.download(event => {
        if (event.event === 'Started') {
          contentLength = event.data.contentLength ?? null
          // Content length is kept locally for progress calculation.
        } else if (event.event === 'Progress') {
          downloadedBytes += event.data.chunkLength
          setDownloaded(downloadedBytes)
          setProgress(contentLength ? Math.min(100, (downloadedBytes / contentLength) * 100) : 0)
        } else if (event.event === 'Finished') {
          setProgress(100)
        }
      })
      await update.install({ restartAfterInstall: true })
      await relaunch()
    } catch (e) {
      // 下载与验签的失败原因完全不同（网络不通 / 404 / 验签不过），
      // 统一文案会让用户和我们都无法判断，所以原始错误必须保留
      setError(describeUpdateError(e, t))
      setStatus('error')
    }
  }, [update, t])

  return (
    <div className="update-page">
      <div className="update-current">
        <span>{t('update.current')}</span>
        <strong>v{currentVersion}</strong>
      </div>
      {status === 'available' && update ? (
        <div className="update-available">
          <div className="update-version">{t('update.available', update.version)}</div>
          {update.body && <div className="update-notes">{update.body}</div>}
          <Button variant="primary" onClick={install}>
            {t('update.downloadInstall')}
          </Button>
        </div>
      ) : (
        <div className="update-status">
          {status === 'checking' && t('update.checking')}
          {status === 'latest' && t('update.latest')}
          {status === 'idle' && t('update.hint')}
          {status === 'error' && <span className="update-error">{t('update.failed', error)}</span>}
          {status === 'downloading' && (
            <div className="update-progress">
              <span>{t('update.downloading', progress ? `${progress.toFixed(0)}%` : '…')}</span>
              <progress max="100" value={progress} />
            </div>
          )}
        </div>
      )}
      {status !== 'downloading' && (
        <Button variant="default" onClick={checkVersion} disabled={status === 'checking'}>
          <IconRefresh size={14} />
          {status === 'error' ? t('update.retry') : t('update.check')}
        </Button>
      )}

      {/* ── 本版更新内容：当前版本在 CHANGELOG 里的段落（离线可读，不依赖网络）── */}
      <div className="update-changes">
        <div className="update-changes-title">{t('update.changesTitle')}</div>
        {changelog.status === 'loading' && (
          <div className="update-changes-empty">{t('common.loading')}</div>
        )}
        {changelog.status === 'empty' && (
          <div className="update-changes-empty">{t('update.changesEmpty')}</div>
        )}
        {changelog.status === 'unavailable' && (
          <div className="update-changes-empty">{t('update.changesUnavailable')}</div>
        )}
        {changelog.status === 'ready' && (
          <div className="update-changes-body">
            {changelog.section.groups.map(group => (
              <div className="update-changes-group" key={group.title}>
                {group.title && (
                  <div className="update-changes-group-title">
                    <MarkdownInline text={group.title} />
                  </div>
                )}
                <ul className="update-changes-list">
                  {group.items.map((item, index) => (
                    <li key={`${group.title}-${index}`}>
                      <MarkdownInline text={item} />
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
