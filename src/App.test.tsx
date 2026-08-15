import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import App from './App'

vi.mock('./SolarMap', () => ({
  SolarMap: () => <div data-testid="solar-map" />,
}))

vi.mock('./runtime', () => ({
  queryBodyState: async () => ({ position_m: [1, 2, 3], velocity_mps: [4, 5, 6] }),
  advanceSimulation: async () => undefined,
}))

vi.mock('./planner', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./planner')>()
  const solution = (id: string, departure: number, propellant: number) => ({
    id,
    departure,
    arrival: departure + 3 * 86_400 * 1e6,
    time_of_flight_s: 3 * 86_400,
    payload_mass_kg: 1_000,
    propellant_consumed_kg: propellant,
    fusion_fuel_consumed_kg: 1,
    peak_power_w: 1e9,
    peak_waste_heat_w: 6e8,
    reactor_lifetime_used_s: 100,
    engine_lifetime_used_s: 200,
    estimated_cost_credits: propellant * 2,
    margins: {
      position_error_m: 2,
      velocity_error_mps: 0.1,
      propellant_remaining_kg: 200_000,
      fusion_fuel_remaining_kg: 500,
      reactor_lifetime_remaining_s: 1e8,
      engine_lifetime_remaining_s: 1e8,
    },
    destination_services: { market: false, propellant_supply: false, repair: false },
    validation_level: 'executable',
    metadata: {
      input_hash: 'authoritative-input-hash',
      solver_version: 'transfer-window-trajectory-v1',
      lambert_iterations: 4,
      integrator_accepted_steps: 12,
      integrator_rejected_steps: 0,
      position_tolerance_m: 100,
      velocity_tolerance_mps: 1,
      termination_reason: 'CONVERGED',
    },
    segments: [{ kind: 'coast', start: departure, end: departure + 3 * 86_400 * 1e6 }],
  })
  const departure = 5_049_129_642_184_000
  const solutions = [
    solution('fast', departure, 30_000),
    solution('balanced', departure + 15 * 86_400 * 1e6, 20_000),
    solution('efficient', departure + 30 * 86_400 * 1e6, 10_000),
  ]
  return {
    ...actual,
    queryTransferPlans: vi.fn(async (_args, onProgress) => {
      onProgress({ requestId: 'test', evaluated: 15, planned: 15, executableSolutions: 3, status: 'completed' })
      return {
        report: {
          input_hash: 'authoritative-input-hash', solutions, failures: [], evaluated: 15, planned: 15,
          status: 'completed', termination_reason: 'CONVERGED',
        },
        paretoSolutionIds: ['fast', 'balanced', 'efficient'],
        representatives: { fastest: 'fast', balanced: 'balanced', efficient: 'efficient' },
        request: { origin_id: 'earth', destination_id: 'moon' },
        worldRevision: 0,
      }
    }),
    scheduleVoyagePlan: vi.fn(async () => ({
      command_id: 'command:test', object_id: 'plan:test', world_revision: 1,
    })),
  }
})

describe('alpha v0.2 authoritative web planner', () => {
  it('opens the planner, expands a server solution and schedules it', async () => {
    render(<App />)
    fireEvent.click(screen.getByRole('button', { name: '航迹规划' }))
    expect(screen.getByRole('heading', { name: '可达空间航迹规划' })).toBeInTheDocument()
    expect(screen.getByText('无市场 · 无补给 · 无维修')).toBeInTheDocument()

    const destination = screen.getByLabelText('目标天体')
    fireEvent.change(destination, { target: { value: 'callisto' } })
    expect(destination).toHaveValue('callisto')
    fireEvent.change(destination, { target: { value: 'triton' } })
    expect(destination).toHaveValue('triton')
    fireEvent.change(destination, { target: { value: 'moon' } })

    fireEvent.click(screen.getByRole('button', { name: '计算 3 × 5 窗口' }))
    await waitFor(() => expect(screen.getByText('质量预算')).toBeInTheDocument())
    expect(screen.getAllByTitle(/\d{4}-\d{2}-\d{2} ·/)).toHaveLength(3)
    expect(screen.getByText(/transfer-window-trajectory-v1/)).toBeInTheDocument()
    expect(screen.getAllByText(/最快|平衡|节能/).length).toBeGreaterThan(0)

    fireEvent.click(screen.getByRole('button', { name: '批准并排期' }))
    await waitFor(() => expect(screen.getByText(/航行计划 plan:test/)).toBeInTheDocument())
  })
})
