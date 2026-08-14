import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import App from './App'

vi.mock('./SolarMap', () => ({
  SolarMap: () => <div data-testid="solar-map" />,
}))

describe('alpha v0.2 transfer planner', () => {
  it('opens the unified planner and expands a Pareto solution', async () => {
    render(<App />)
    fireEvent.click(screen.getByRole('button', { name: '航迹规划' }))
    expect(screen.getByRole('heading', { name: '可达空间航迹规划' })).toBeInTheDocument()
    expect(screen.getByText('无市场 · 无补给 · 无维修')).toBeInTheDocument()

    const destination = screen.getByLabelText('目标天体')
    fireEvent.change(destination, { target: { value: 'callisto' } })
    expect(destination).toHaveValue('callisto')
    expect(screen.getByText('无市场 · 无补给 · 无维修')).toBeInTheDocument()
    fireEvent.change(destination, { target: { value: 'triton' } })
    expect(destination).toHaveValue('triton')
    fireEvent.change(destination, { target: { value: 'moon' } })

    fireEvent.click(screen.getByRole('button', { name: '计算 3 × 5 窗口' }))
    await waitFor(() => expect(screen.getByText('质量预算')).toBeInTheDocument(), { timeout: 3_000 })
    expect(screen.getAllByTitle(/\d{4}-\d{2}-\d{2} ·/).length).toBeGreaterThanOrEqual(3)
    expect(screen.getByText(/browser-preview-v1/)).toBeInTheDocument()
    expect(screen.getAllByText(/最快|平衡|节能/).length).toBeGreaterThan(0)
  })
})
