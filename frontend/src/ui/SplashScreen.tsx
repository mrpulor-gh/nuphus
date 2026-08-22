import { useEffect, useState } from 'react'
import { NuphusAvatar, type NuphusAvatarState } from './NuphusAvatar'
import '../styles/splash.css'

interface InitItem {
  key: string
  label: string
  status: 'pending' | 'loading' | 'done' | 'error'
}

interface SplashScreenProps {
  items: InitItem[]
  fadeOut: boolean
}

const STEP_LABELS: Record<string, string> = {
  memory: 'Memory',
  tools: 'Tools',
  model: 'AI Model',
  ocr: 'OCR',
}

export function SplashScreen({ items, fadeOut }: SplashScreenProps) {
  const [statusText, setStatusText] = useState('Starting…')

  useEffect(() => {
    const loading = items.find(i => i.status === 'loading')
    if (loading) {
      setStatusText(`Loading ${STEP_LABELS[loading.key] || loading.key}…`)
    }
  }, [items])

  const avatarState: NuphusAvatarState = items.some(i => i.status === 'error')
    ? 'error'
    : items.length > 0 && items.every(i => i.status === 'done')
      ? 'success'
      : 'working'

  return (
    <div className={`splash-screen${fadeOut ? ' fade-out' : ''}`}>
      <NuphusAvatar state={avatarState} size={56} />
      <div className="splash-status">{statusText}</div>
    </div>
  )
}
