import { invoke } from '@tauri-apps/api/core'
import { catalog, heliocentricState, type StateVector } from './model'

interface RuntimeBodyState extends StateVector {
  body_id: string
}

function isTauri(): boolean {
  return '__TAURI_INTERNALS__' in window
}

export async function queryBodyState(bodyId: string, epochTdbMicros: number): Promise<StateVector> {
  if (isTauri()) {
    return invoke<RuntimeBodyState>('body_state', { bodyId, epochTdbMicros })
  }
  const body = catalog.bodies.find((candidate) => candidate.id === bodyId)
  if (!body) throw new Error(`BODY_NOT_FOUND: ${bodyId}`)
  return heliocentricState(body, epochTdbMicros)
}

export async function queryMapSample(epochTdbMicros: number): Promise<Map<string, StateVector>> {
  const states = isTauri()
    ? await invoke<RuntimeBodyState[]>('map_sample', { epochTdbMicros })
    : catalog.bodies.map((body) => ({ body_id: body.id, ...heliocentricState(body, epochTdbMicros) }))
  return new Map(states.map((state) => [state.body_id, state]))
}

