import { beforeEach, describe, expect, it } from 'vitest'
import {
  getCachedLanUrl,
  getCachedRelayUrl,
  isOfficialRelayUrl,
  saveRelayCache,
} from './connection'

const RELAY_STORAGE_KEY = 'nuphus_relay_cfg'

describe('local-only relay cache', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('never saves an official relay URL while preserving the LAN address', () => {
    saveRelayCache({
      lan_url: 'http://192.168.1.23:18772',
      relay_url: 'https://r.nuphus.com/?device=desktop-1',
    })

    expect(getCachedLanUrl()).toBe('http://192.168.1.23:18772')
    expect(getCachedRelayUrl()).toBeNull()
  })

  it('cleans an official relay URL left in an older cache', () => {
    localStorage.setItem(
      RELAY_STORAGE_KEY,
      JSON.stringify({
        lan_url: 'http://192.168.1.23:18772',
        relay_url: 'https://relay.nuphus.com/?device=desktop-1',
        ts: Date.now(),
      })
    )

    expect(getCachedRelayUrl()).toBeNull()
    const cleaned = JSON.parse(localStorage.getItem(RELAY_STORAGE_KEY) || '{}')
    expect(cleaned.relay_url).toBeUndefined()
    expect(cleaned.lan_url).toBe('http://192.168.1.23:18772')
  })

  it('keeps a user-configured custom server URL', () => {
    saveRelayCache({
      relay_url: 'https://relay.example.com/?device=desktop-1',
    })

    expect(getCachedRelayUrl()).toBe('https://relay.example.com/?device=desktop-1')
    expect(isOfficialRelayUrl('wss://RELAY.nuphus.com./ws/device')).toBe(true)
    expect(isOfficialRelayUrl('https://relay.example.com')).toBe(false)
  })
})
