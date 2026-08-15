import { beforeEach, describe, expect, it, vi } from 'vitest'
import { loadSnapshot, saveSnapshot } from './persistence'

class MemoryStorage implements Storage {
  private values = new Map<string, string>()
  get length() { return this.values.size }
  clear() { this.values.clear() }
  getItem(key: string) { return this.values.get(key) ?? null }
  key(index: number) { return [...this.values.keys()][index] ?? null }
  removeItem(key: string) { this.values.delete(key) }
  setItem(key: string, value: string) { this.values.set(key, value) }
}

const snapshot = {
  schemaVersion: 2 as const,
  contentVersion: 'test',
  epochTdbMicros: 42,
  selectedBodyId: 'earth',
}

describe('server-backed save client', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
    Object.defineProperty(window, 'localStorage', { value: new MemoryStorage(), configurable: true })
  })

  it('writes and loads schema two through the versioned API', async () => {
    const fetchMock = vi.fn().mockImplementation(async () => new Response(JSON.stringify(snapshot), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    }))
    vi.stubGlobal('fetch', fetchMock)

    await saveSnapshot(snapshot)
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/api/v1/saves/default', expect.objectContaining({ method: 'POST' }))
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual(snapshot)
    expect(await loadSnapshot()).toEqual(snapshot)
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/api/v1/saves/default', expect.objectContaining({ headers: expect.any(Headers) }))
  })

  it('rejects a malformed server view instead of resetting the world', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ schemaVersion: 1 }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    })))
    await expect(loadSnapshot()).rejects.toThrow('SAVE_CORRUPT')
  })

  it('imports a legacy browser snapshot only when the server slot is missing', async () => {
    window.localStorage.setItem('transfer-window.alpha-v0.1.snapshot', JSON.stringify({
      schemaVersion: 1, contentVersion: 'test', epochTdbMicros: 42, selectedBodyId: 'moon',
    }))
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ code: 'SAVE_NOT_FOUND', message: 'save does not exist' }), {
        status: 404,
        headers: { 'content-type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ ...snapshot, selectedBodyId: 'moon' }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }))
    vi.stubGlobal('fetch', fetchMock)

    expect(await loadSnapshot()).toEqual({ ...snapshot, selectedBodyId: 'moon' })
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/api/v1/saves/default', expect.objectContaining({ method: 'POST' }))
    expect(window.localStorage.getItem('transfer-window.alpha-v0.1.snapshot')).not.toBeNull()
  })
})
