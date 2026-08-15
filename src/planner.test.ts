import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  paretoSolutionIds,
  queryTransferPlans,
  selectPlannerRepresentatives,
  type PlannerSolution,
} from './planner'

afterEach(() => vi.unstubAllGlobals())

function solution(
  id: string,
  arrival: number,
  propellant: number,
  lifetime: number,
  cost: number,
): PlannerSolution {
  return {
    id,
    departure: 0,
    arrival,
    time_of_flight_s: arrival,
    payload_mass_kg: 1_000,
    propellant_consumed_kg: propellant,
    fusion_fuel_consumed_kg: 1,
    peak_power_w: 1,
    peak_waste_heat_w: 1,
    reactor_lifetime_used_s: lifetime / 2,
    engine_lifetime_used_s: lifetime / 2,
    estimated_cost_credits: cost,
    margins: {
      position_error_m: 1,
      velocity_error_mps: 1,
      propellant_remaining_kg: 1,
      fusion_fuel_remaining_kg: 1,
      reactor_lifetime_remaining_s: 1,
      engine_lifetime_remaining_s: 1,
    },
    destination_services: { market: false, propellant_supply: false, repair: false },
    validation_level: 'executable',
    metadata: {
      input_hash: 'hash', solver_version: 'test', lambert_iterations: 1,
      integrator_accepted_steps: 1, integrator_rejected_steps: 0,
      position_tolerance_m: 1, velocity_tolerance_mps: 1, termination_reason: 'CONVERGED',
    },
    segments: [],
  }
}

describe('trajectory Pareto adapter', () => {
  it('keeps non-dominated solutions and selects representatives from that set', () => {
    const solutions = [
      solution('fast', 10, 100, 100, 500),
      solution('efficient', 20, 50, 50, 200),
      solution('dominated', 25, 120, 120, 600),
    ]
    const frontier = paretoSolutionIds(solutions)
    expect(frontier).toEqual(['fast', 'efficient'])
    const representatives = selectPlannerRepresentatives(solutions, frontier)!
    expect(frontier).toContain(representatives.fastest)
    expect(frontier).toContain(representatives.balanced)
    expect(frontier).toContain(representatives.efficient)
  })

  it('returns the authoritative result from the HTTP job status path', async () => {
    const authoritativeSolution = solution('authoritative', 10, 100, 100, 500)
    authoritativeSolution.metadata.solver_version = 'transfer-window-trajectory-v1'
    const result = {
      report: {
        input_hash: 'hash', solutions: [authoritativeSolution], failures: [], evaluated: 1, planned: 1,
        status: 'completed', termination_reason: 'CONVERGED',
      },
      paretoSolutionIds: ['authoritative'],
      representatives: { fastest: 'authoritative', balanced: 'authoritative', efficient: 'authoritative' },
      request: { origin_id: 'earth', destination_id: 'moon' },
      worldRevision: 0,
    }
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({
        requestId: 'planner-test', eventsUrl: '/api/v1/trajectory/jobs/planner-test/events',
      }), { status: 202, headers: { 'content-type': 'application/json' } }))
      .mockResolvedValueOnce(new Response(JSON.stringify({
        requestId: 'planner-test', state: 'completed',
        progress: { requestId: 'planner-test', evaluated: 1, planned: 1, executableSolutions: 1, status: 'completed' },
        result, error: null,
      }), { status: 200, headers: { 'content-type': 'application/json' } }))
    vi.stubGlobal('fetch', fetchMock)
    vi.stubGlobal('EventSource', undefined)

    const progress = vi.fn()
    const response = await queryTransferPlans({
      requestId: 'planner-test', originId: 'earth', destinationId: 'moon', departureTdbMicros: 0,
      payloadMassKg: 1_000, payloadVolumeM3: 10, minimumDurationDays: 3, maximumDurationDays: 40,
    }, progress, new AbortController().signal)

    expect(response.report.solutions[0].metadata.solver_version).toBe('transfer-window-trajectory-v1')
    expect(progress).toHaveBeenCalledWith(expect.objectContaining({ executableSolutions: 1 }))
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/api/v1/trajectory/jobs', expect.objectContaining({ method: 'POST' }))
  })
})
