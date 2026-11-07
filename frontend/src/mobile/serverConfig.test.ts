import { beforeEach, describe, expect, it } from 'vitest'
import {
  getPreferredMobileServerUrl,
  normalizeMobileServerUrl,
  prepareMobileServerNavigation,
  saveMobileServerUrl,
} from './serverConfig'

describe('mobile server configuration', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('defaults bare LAN addresses to HTTP and public addresses to HTTPS', () => {
    expect(normalizeMobileServerUrl('192.168.1.23:18772')).toBe('http://192.168.1.23:18772/')
    expect(normalizeMobileServerUrl('relay.example.com')).toBe('https://relay.example.com/')
  })

  it('converts desktop websocket addresses to page URLs', () => {
    expect(normalizeMobileServerUrl('ws://192.168.1.23:18081')).toBe('http://192.168.1.23:18081/')
    expect(normalizeMobileServerUrl('wss://relay.example.com')).toBe('https://relay.example.com/')
  })

  it('preserves device routing but never stores a token', () => {
    const url = normalizeMobileServerUrl('https://relay.example.com/?device=desktop-1&token=secret')
    expect(url).toBe('https://relay.example.com/?device=desktop-1')

    saveMobileServerUrl(url)
    expect(getPreferredMobileServerUrl()).toBe('https://relay.example.com/?device=desktop-1')
    expect(localStorage.getItem('nuphus_mobile_server_url')).not.toContain('secret')
  })

  it('rejects empty, credential-bearing, and unsupported addresses', () => {
    expect(() => normalizeMobileServerUrl('')).toThrow('empty')
    expect(() => normalizeMobileServerUrl('ftp://example.com')).toThrow('invalid')
    expect(() => normalizeMobileServerUrl('https://user:pass@example.com')).toThrow('invalid')
  })

  it('reports whether a selected endpoint differs from the current page', () => {
    const prepared = prepareMobileServerNavigation('https://relay.example.com')
    expect(prepared.url).toBe('https://relay.example.com/')
    expect(prepared.changed).toBe(true)
  })
})
