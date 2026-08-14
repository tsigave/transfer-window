import { beforeEach, describe, expect, it } from 'vitest'
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

describe('browser save schema migration', () => {
  beforeEach(() => {
    Object.defineProperty(window, 'localStorage', { value: new MemoryStorage(), configurable: true })
  })

  it('writes schema two snapshots', async () => {
    await saveSnapshot({ schemaVersion: 2, contentVersion: 'test', epochTdbMicros: 42, selectedBodyId: 'earth' })
    expect((await loadSnapshot()).schemaVersion).toBe(2)
  })

  it('migrates alpha v0.1 while retaining the legacy entry as a backup', async () => {
    const legacyKey = 'transfer-window.alpha-v0.1.snapshot'
    window.localStorage.setItem(legacyKey, JSON.stringify({
      schemaVersion: 1, contentVersion: 'test', epochTdbMicros: 42, selectedBodyId: 'moon',
    }))
    const migrated = await loadSnapshot()
    expect(migrated.schemaVersion).toBe(2)
    expect(migrated.selectedBodyId).toBe('moon')
    expect(window.localStorage.getItem(legacyKey)).not.toBeNull()
    expect(window.localStorage.getItem('transfer-window.alpha-v0.2.snapshot')).not.toBeNull()
  })
})
