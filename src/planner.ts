import { apiRequest, apiUrl } from './api'

export interface PlannerArgs {
  requestId: string
  originId: string
  destinationId: string
  departureTdbMicros: number
  payloadMassKg: number
  payloadVolumeM3: number
  minimumDurationDays: number
  maximumDurationDays: number
}

export interface PlannerProgress {
  requestId: string
  evaluated: number
  planned: number
  executableSolutions: number
  status: 'completed' | 'partial_budget_exhausted' | 'cancelled'
}

export interface PlannerSegment {
  kind: 'finite_burn' | 'coast' | 'approach'
  phase?: string
  start?: number
  end?: number
  target_delta_v_mps?: number
  thrust_n?: number
  input_power_w?: number
  powered_duration_s?: number
  chunk_count?: number
  initial_mass_kg?: number
  final_mass_kg?: number
  peak_waste_heat_w?: number
  planned_position_error_m?: number
  planned_velocity_error_mps?: number
}

export interface PlannerSolution {
  id: string
  departure: number
  arrival: number
  time_of_flight_s: number
  payload_mass_kg: number
  propellant_consumed_kg: number
  fusion_fuel_consumed_kg: number
  peak_power_w: number
  peak_waste_heat_w: number
  reactor_lifetime_used_s: number
  engine_lifetime_used_s: number
  estimated_cost_credits: number
  margins: {
    position_error_m: number
    velocity_error_mps: number
    propellant_remaining_kg: number
    fusion_fuel_remaining_kg: number
    reactor_lifetime_remaining_s: number
    engine_lifetime_remaining_s: number
  }
  destination_services: { market: boolean; propellant_supply: boolean; repair: boolean }
  validation_level: 'candidate' | 'executable'
  metadata: {
    input_hash: string
    solver_version: string
    lambert_iterations: number
    integrator_accepted_steps: number
    integrator_rejected_steps: number
    position_tolerance_m: number
    velocity_tolerance_mps: number
    termination_reason: string
  }
  segments: PlannerSegment[]
}

export interface PlannerFailure {
  departure: number | null
  duration_s: number | null
  kind: string
  message: string
  constraints: Array<{ code: string; field: string; required: number; available: number; unit: string }>
}

export interface PlanTransferResult {
  report: {
    input_hash: string
    solutions: PlannerSolution[]
    failures: PlannerFailure[]
    evaluated: number
    planned: number
    status: 'completed' | 'partial_budget_exhausted' | 'cancelled'
    termination_reason: string
  }
  paretoSolutionIds: string[]
  representatives: { fastest: string; balanced: string; efficient: string } | null
  request: unknown
  worldRevision: number
}

interface CreatedJob {
  requestId: string
  eventsUrl: string
}

interface PlannerJobView {
  requestId: string
  state: 'running' | 'completed' | 'cancelled' | 'failed'
  progress: PlannerProgress
  result: PlanTransferResult | null
  error: { code: string; message: string } | null
}

interface PlannerEventEnvelope {
  type: 'progress' | 'complete' | 'cancelled' | 'failed'
  payload?: PlannerProgress | { code: string; message: string }
}

export async function queryTransferPlans(
  args: PlannerArgs,
  onProgress: (progress: PlannerProgress) => void,
  signal: AbortSignal,
): Promise<PlanTransferResult> {
  const created = await apiRequest<CreatedJob>('/api/v1/trajectory/jobs', {
    method: 'POST',
    body: JSON.stringify(args),
    signal,
  })
  const cancel = () => {
    void apiRequest<void>(`/api/v1/trajectory/jobs/${encodeURIComponent(created.requestId)}`, {
      method: 'DELETE',
    }).catch(() => undefined)
  }
  signal.addEventListener('abort', cancel, { once: true })
  const source = typeof EventSource === 'undefined' ? null : new EventSource(apiUrl(created.eventsUrl))
  source?.addEventListener('progress', (event) => {
    try {
      const envelope = JSON.parse((event as MessageEvent<string>).data) as PlannerEventEnvelope
      if (envelope.type === 'progress') onProgress(envelope.payload as PlannerProgress)
    } catch {
      // Polling below remains authoritative if an intermediary corrupts an SSE event.
    }
  })
  try {
    while (true) {
      if (signal.aborted) throw new DOMException('trajectory request cancelled', 'AbortError')
      const job = await apiRequest<PlannerJobView>(
        `/api/v1/trajectory/jobs/${encodeURIComponent(created.requestId)}`,
        { signal },
      )
      onProgress(job.progress)
      if (job.state === 'completed' && job.result) return job.result
      if (job.state === 'cancelled') throw new DOMException('trajectory request cancelled', 'AbortError')
      if (job.state === 'failed') {
        throw new Error(`${job.error?.code ?? 'TRAJECTORY_FAILED'}: ${job.error?.message ?? '航迹任务失败'}`)
      }
      await wait(40, signal)
    }
  } finally {
    signal.removeEventListener('abort', cancel)
    source?.close()
  }
}

export async function scheduleVoyagePlan(
  planning: PlanTransferResult,
  solution: PlannerSolution,
): Promise<{ command_id: string; object_id: string; world_revision: number }> {
  return apiRequest('/api/v1/voyages', {
    method: 'POST',
    body: JSON.stringify({
      commandId: `command:schedule-${Date.now().toString(36)}`,
      expectedWorldRevision: planning.worldRevision,
      request: planning.request,
      solution,
    }),
  })
}

export function paretoSolutionIds(solutions: PlannerSolution[]): string[] {
  const executable = solutions.filter((solution) => solution.validation_level === 'executable')
  return executable
    .filter((candidate) => !executable.some((other) => other.id !== candidate.id && dominates(other, candidate)))
    .map((solution) => solution.id)
}

export function selectPlannerRepresentatives(
  solutions: PlannerSolution[],
  frontierIds: string[],
): PlanTransferResult['representatives'] {
  const frontier = solutions.filter((solution) => frontierIds.includes(solution.id))
  if (!frontier.length) return null
  const fastest = frontier.reduce((best, item) => item.arrival < best.arrival ? item : best)
  const efficient = frontier.reduce((best, item) => item.estimated_cost_credits < best.estimated_cost_credits ? item : best)
  const values = (select: (solution: PlannerSolution) => number) => frontier.map(select)
  const normalize = (value: number, samples: number[], reverse = false) => {
    const minimum = Math.min(...samples)
    const maximum = Math.max(...samples)
    const normalized = (value - minimum) / Math.max(1, maximum - minimum)
    return reverse ? 1 - normalized : normalized
  }
  const arrivals = values((solution) => solution.arrival)
  const propellant = values((solution) => solution.propellant_consumed_kg)
  const payloads = values((solution) => solution.payload_mass_kg)
  const lifetimes = values((solution) => solution.reactor_lifetime_used_s + solution.engine_lifetime_used_s)
  const costs = values((solution) => solution.estimated_cost_credits)
  const score = (solution: PlannerSolution) => Math.hypot(
    normalize(solution.arrival, arrivals),
    normalize(solution.propellant_consumed_kg, propellant),
    normalize(solution.payload_mass_kg, payloads, true),
    normalize(solution.reactor_lifetime_used_s + solution.engine_lifetime_used_s, lifetimes),
    normalize(solution.estimated_cost_credits, costs),
  )
  const balanced = frontier.reduce((best, item) => score(item) < score(best) ? item : best)
  return { fastest: fastest.id, balanced: balanced.id, efficient: efficient.id }
}

function dominates(left: PlannerSolution, right: PlannerSolution): boolean {
  const leftLifetime = left.reactor_lifetime_used_s + left.engine_lifetime_used_s
  const rightLifetime = right.reactor_lifetime_used_s + right.engine_lifetime_used_s
  const noWorse = left.arrival <= right.arrival
    && left.propellant_consumed_kg <= right.propellant_consumed_kg
    && left.payload_mass_kg >= right.payload_mass_kg
    && leftLifetime <= rightLifetime
    && left.estimated_cost_credits <= right.estimated_cost_credits
  const better = left.arrival < right.arrival
    || left.propellant_consumed_kg < right.propellant_consumed_kg
    || left.payload_mass_kg > right.payload_mass_kg
    || leftLifetime < rightLifetime
    || left.estimated_cost_credits < right.estimated_cost_credits
  return noWorse && better
}

function wait(milliseconds: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const abort = () => {
      window.clearTimeout(timeout)
      reject(new DOMException('trajectory request cancelled', 'AbortError'))
    }
    const timeout = window.setTimeout(() => {
      signal.removeEventListener('abort', abort)
      resolve()
    }, milliseconds)
    signal.addEventListener('abort', abort, { once: true })
  })
}
