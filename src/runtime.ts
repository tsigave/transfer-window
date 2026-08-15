import { apiRequest } from './api'
import type { StateVector } from './model'

interface RuntimeBodyState extends StateVector {
  body_id: string
}

export async function queryBodyState(bodyId: string, epochTdbMicros: number): Promise<StateVector> {
  return apiRequest<RuntimeBodyState>(
    `/api/v1/bodies/${encodeURIComponent(bodyId)}/state?epochTdbMicros=${encodeURIComponent(epochTdbMicros)}`,
  )
}

export async function queryMapSample(epochTdbMicros: number): Promise<Map<string, StateVector>> {
  const states = await apiRequest<RuntimeBodyState[]>(
    `/api/v1/map-sample?epochTdbMicros=${encodeURIComponent(epochTdbMicros)}`,
  )
  return new Map(states.map((state) => [state.body_id, state]))
}

export async function advanceSimulation(targetTdbMicros: number): Promise<void> {
  await apiRequest('/api/v1/simulation/advance', {
    method: 'POST',
    body: JSON.stringify({ targetTdbMicros }),
  })
}
