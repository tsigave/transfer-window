import { invoke } from '@tauri-apps/api/core'

export interface BrowserSnapshot {
  schemaVersion: 1
  contentVersion: string
  epochTdbMicros: number
  selectedBodyId: string
}

const storageKey = 'transfer-window.alpha-v0.1.snapshot'
const legacyStorageKey = 'solarstorm.alpha-v0.1.snapshot'

function isTauri(): boolean {
  return '__TAURI_INTERNALS__' in window
}

export async function saveSnapshot(snapshot: BrowserSnapshot): Promise<void> {
  if (isTauri()) {
    await invoke('save_game', { viewState: snapshot })
    return
  }
  localStorage.setItem(storageKey, JSON.stringify(snapshot))
}

export async function loadSnapshot(): Promise<BrowserSnapshot> {
  let raw: string | null
  if (isTauri()) {
    raw = await invoke<string>('load_game')
  } else {
    raw = localStorage.getItem(storageKey)
    if (!raw) {
      raw = localStorage.getItem(legacyStorageKey)
      if (raw) {
        localStorage.setItem(storageKey, raw)
        localStorage.removeItem(legacyStorageKey)
      }
    }
  }
  if (!raw) throw new Error('SAVE_NOT_FOUND: 尚未创建浏览器存档。')
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    throw new Error('SAVE_CORRUPT: 存档不是有效 JSON，世界未被重置。')
  }
  if (!isBrowserSnapshot(parsed)) {
    throw new Error('SAVE_CORRUPT: 存档字段无效，世界未被重置。')
  }
  return parsed
}

function isBrowserSnapshot(value: unknown): value is BrowserSnapshot {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Record<string, unknown>
  return candidate.schemaVersion === 1
    && typeof candidate.contentVersion === 'string'
    && Number.isFinite(candidate.epochTdbMicros)
    && typeof candidate.selectedBodyId === 'string'
}
