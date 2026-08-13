import { describe, expect, it } from 'vitest'
import { bodyById, catalog, epochFromDate, heliocentricState, searchBodies } from './model'

describe('solar-system fact adapter', () => {
  it('finds every manual-acceptance target in Chinese', () => {
    for (const query of ['地球', '谷神星', '木卫四', '海卫一', '阿罗科特']) {
      expect(searchBodies(query)).toHaveLength(1)
    }
  })

  it('keeps all required bodies and regions distinct', () => {
    expect(catalog.bodies).toHaveLength(41)
    expect(catalog.regions).toHaveLength(3)
    expect(bodyById.has('asteroid-belt')).toBe(false)
  })

  it('computes finite state without mutating catalog inputs', () => {
    const earth = bodyById.get('earth')!
    const originalAxis = earth.ephemeris!.semi_major_axis_m
    const state = heliocentricState(earth, epochFromDate(new Date('2170-01-01T00:00:00Z')))
    expect(state.position_m.every(Number.isFinite)).toBe(true)
    expect(earth.ephemeris!.semi_major_axis_m).toBe(originalAxis)
  })
})
