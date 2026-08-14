import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { bodyById, heliocentricState } from './model'

const DAY_MICROS = 86_400 * 1e6

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
  request: unknown | null
  worldRevision: number
}

function isTauri(): boolean {
  return '__TAURI_INTERNALS__' in window
}

export async function queryTransferPlans(
  args: PlannerArgs,
  onProgress: (progress: PlannerProgress) => void,
  signal: AbortSignal,
): Promise<PlanTransferResult> {
  if (!isTauri()) return browserPreview(args, onProgress, signal)

  const unlisten = await listen<PlannerProgress>('trajectory-progress', ({ payload }) => {
    if (payload.requestId === args.requestId) onProgress(payload)
  })
  const cancel = () => { void invoke('cancel_transfer', { requestId: args.requestId }) }
  signal.addEventListener('abort', cancel, { once: true })
  try {
    return await invoke<PlanTransferResult>('plan_transfer', { args })
  } finally {
    signal.removeEventListener('abort', cancel)
    unlisten()
  }
}

export async function scheduleVoyagePlan(
  planning: PlanTransferResult,
  solution: PlannerSolution,
): Promise<{ command_id: string; object_id: string; world_revision: number }> {
  if (!isTauri() || !planning.request) {
    throw new Error('SUBMISSION_UNAVAILABLE: 浏览器预览不能提交航行计划；请在桌面应用中使用 Rust 执行级结果。')
  }
  return invoke('schedule_voyage', {
    commandId: `command:schedule-${Date.now().toString(36)}`,
    expectedWorldRevision: planning.worldRevision,
    request: planning.request,
    solution,
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

async function browserPreview(
  args: PlannerArgs,
  onProgress: (progress: PlannerProgress) => void,
  signal: AbortSignal,
): Promise<PlanTransferResult> {
  const origin = bodyById.get(args.originId)
  const destination = bodyById.get(args.destinationId)
  if (!origin || !destination || origin.id === destination.id) {
    throw new Error('INVALID_REQUEST: 始发与目标必须是两个已登记的不同天体。')
  }
  if (args.payloadMassKg < 0 || args.payloadMassKg > 120_000 || args.payloadVolumeM3 < 0 || args.payloadVolumeM3 > 650) {
    throw new Error('CONSTRAINT_VIOLATION: 载荷超过 Lunar Courier 的 120,000 kg / 650 m³ 舱容。')
  }

  const departureOffsets = [0, 15, 30]
  const durationDays = Array.from({ length: 5 }, (_, index) =>
    args.minimumDurationDays + (args.maximumDurationDays - args.minimumDurationDays) * index / 4)
  const planned = departureOffsets.length * durationDays.length
  const solutions: PlannerSolution[] = []
  const failures: PlannerFailure[] = []
  let evaluated = 0

  for (const offset of departureOffsets) {
    for (const days of durationDays) {
      if (signal.aborted) {
        const ids = paretoSolutionIds(solutions)
        return {
          report: { input_hash: browserHash(args), solutions, failures, evaluated, planned, status: 'cancelled', termination_reason: 'CANCELLED' },
          paretoSolutionIds: ids,
          representatives: selectPlannerRepresentatives(solutions, ids),
          request: null,
          worldRevision: 0,
        }
      }
      const departure = args.departureTdbMicros + offset * DAY_MICROS
      const arrival = departure + days * DAY_MICROS
      const originState = heliocentricState(origin, departure)
      const destinationState = heliocentricState(destination, arrival)
      const distance = Math.hypot(...destinationState.position_m.map((value, index) => value - originState.position_m[index]))
      const relativeVelocity = Math.hypot(...destinationState.velocity_mps.map((value, index) => value - originState.velocity_mps[index]))
      const requiredDeltaV = Math.max(350, distance / (days * 86_400) * 0.075 + relativeVelocity * 0.12)
      const initialMass = 420_000 + 300_000 + 600 + args.payloadMassKg
      const propellant = initialMass * (1 - Math.exp(-requiredDeltaV / 250_000))
      const massFlow = 2 * 0.72 * 1e9 / 250_000 ** 2
      const poweredDuration = propellant / massFlow
      const feasible = propellant < 285_000 && poweredDuration < days * 86_400 * 0.5
      evaluated += 1
      if (feasible) {
        const fusionFuel = (1.03e9 * poweredDuration) / 1e14
        const lifetime = poweredDuration * 1e9 / 1.2e9
        const positionError = 120 + ((offset + days) % 17) * 11
        const velocityError = 0.04 + ((offset + days) % 7) * 0.01
        const id = `preview:${offset}:${Math.round(days * 10)}`
        solutions.push({
          id,
          departure,
          arrival,
          time_of_flight_s: days * 86_400,
          payload_mass_kg: args.payloadMassKg,
          propellant_consumed_kg: propellant,
          fusion_fuel_consumed_kg: fusionFuel,
          peak_power_w: 1.03e9,
          peak_waste_heat_w: 639.6e6,
          reactor_lifetime_used_s: lifetime,
          engine_lifetime_used_s: poweredDuration,
          estimated_cost_credits: propellant * 2 + fusionFuel * 1_000 + (lifetime + poweredDuration) * 0.05,
          margins: {
            position_error_m: positionError,
            velocity_error_mps: velocityError,
            propellant_remaining_kg: 300_000 - propellant,
            fusion_fuel_remaining_kg: 600 - fusionFuel,
            reactor_lifetime_remaining_s: 252_460_800 - lifetime,
            engine_lifetime_remaining_s: 126_230_400 - poweredDuration,
          },
          destination_services: { market: false, propellant_supply: false, repair: false },
          validation_level: 'executable',
          metadata: {
            input_hash: browserHash(args), solver_version: 'browser-preview-v1', lambert_iterations: 0,
            integrator_accepted_steps: 0, integrator_rejected_steps: 0,
            position_tolerance_m: 2_000_000, velocity_tolerance_mps: 2, termination_reason: 'CONVERGED',
          },
          segments: [
            { kind: 'finite_burn', phase: 'departure', start: departure, end: departure + poweredDuration * 1e6, target_delta_v_mps: requiredDeltaV, thrust_n: 5_760, input_power_w: 1e9, powered_duration_s: poweredDuration, chunk_count: Math.max(1, Math.ceil(poweredDuration / 21_600)), initial_mass_kg: initialMass, final_mass_kg: initialMass - propellant - fusionFuel, peak_waste_heat_w: 639.6e6 },
            { kind: 'coast', start: departure + poweredDuration * 1e6, end: arrival },
            { kind: 'approach', start: arrival, end: arrival, planned_position_error_m: positionError, planned_velocity_error_mps: velocityError },
          ],
        })
      } else {
        failures.push({
          departure, duration_s: days * 86_400, kind: 'constraint_violation',
          message: '有限推力机动超过工质或航程占比限制。',
          constraints: [{ code: 'FINITE_THRUST_DURATION', field: 'powered_duration_s', required: poweredDuration, available: days * 86_400 * 0.5, unit: 's' }],
        })
      }
      onProgress({ requestId: args.requestId, evaluated, planned, executableSolutions: solutions.length, status: 'completed' })
      await new Promise((resolve) => window.setTimeout(resolve, 4))
    }
  }
  const ids = paretoSolutionIds(solutions)
  return {
    report: { input_hash: browserHash(args), solutions, failures, evaluated, planned, status: 'completed', termination_reason: solutions.length ? 'CONVERGED' : 'CONSTRAINT_VIOLATION' },
    paretoSolutionIds: ids,
    representatives: selectPlannerRepresentatives(solutions, ids),
    request: null,
    worldRevision: 0,
  }
}

function browserHash(args: PlannerArgs): string {
  return `preview-${args.originId}-${args.destinationId}-${Math.round(args.departureTdbMicros)}`
}
