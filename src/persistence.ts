import { apiRequest } from './api'

export interface BrowserSnapshot {
  schemaVersion: 2
  contentVersion: string
  epochTdbMicros: number
  selectedBodyId: string
}

const legacyStorageKeys = [
  'transfer-window.alpha-v0.2.snapshot',
  'transfer-window.alpha-v0.1.snapshot',
  'solarstorm.alpha-v0.1.snapshot',
]

export async function saveSnapshot(snapshot: BrowserSnapshot): Promise<void> {
  await apiRequest<BrowserSnapshot>('/api/v1/saves/default', {
    method: 'POST',
    body: JSON.stringify(snapshot),
  })
}

export async function loadSnapshot(): Promise<BrowserSnapshot> {
  let snapshot: BrowserSnapshot
  try {
    snapshot = await apiRequest<BrowserSnapshot>('/api/v1/saves/default')
  } catch (reason) {
    if (!(reason instanceof Error) || !reason.message.startsWith('SAVE_NOT_FOUND:')) throw reason
    const legacy = readLegacyBrowserSnapshot()
    if (!legacy) throw reason
    snapshot = { ...legacy, schemaVersion: 2 }
    await saveSnapshot(snapshot)
  }
  if (!isBrowserSnapshot(snapshot)) {
    throw new Error('SAVE_CORRUPT: 服务端存档视图字段无效，世界未被重置。')
  }
  return snapshot
}

function readLegacyBrowserSnapshot(): BrowserSnapshot | null {
  try {
    for (const key of legacyStorageKeys) {
      const raw = window.localStorage.getItem(key)
      if (!raw) continue
      const parsed = JSON.parse(raw) as Record<string, unknown>
      const migrated = { ...parsed, schemaVersion: 2 }
      if (isBrowserSnapshot(migrated)) return migrated
    }
  } catch {
    return null
  }
  return null
}

function isBrowserSnapshot(value: unknown): value is BrowserSnapshot {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Record<string, unknown>
  return candidate.schemaVersion === 2
    && typeof candidate.contentVersion === 'string'
    && Number.isFinite(candidate.epochTdbMicros)
    && typeof candidate.selectedBodyId === 'string'
}
