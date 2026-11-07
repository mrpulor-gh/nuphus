import { describe, expect, it } from 'vitest'
import { withPairToken } from './mobilePairing'

describe('mobilePairing', () => {
  it('adds the mobile token to a LAN pairing URL', () => {
    expect(withPairToken('http://192.168.1.23:18772', 'mobile-token')).toBe(
      'http://192.168.1.23:18772/?token=mobile-token',
    )
  })

  it('preserves the device routing query on a custom server URL', () => {
    const url = new URL(
      withPairToken('https://relay.example.com/?device=desktop-123&lang=zh', 'mobile-token'),
    )
    expect(url.searchParams.get('device')).toBe('desktop-123')
    expect(url.searchParams.get('lang')).toBe('zh')
    expect(url.searchParams.get('token')).toBe('mobile-token')
  })
})
