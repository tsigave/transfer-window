import { describe, expect, it } from 'vitest'
import { atmosphereProfiles } from './atmosphere'
import solarMapSource from './SolarMap.tsx?raw'

describe('planet atmosphere profiles', () => {
  it('covers every major planet with a visible atmosphere', () => {
    expect(Object.keys(atmosphereProfiles)).toEqual(expect.arrayContaining([
      'venus',
      'earth',
      'mars',
      'jupiter',
      'saturn',
      'uranus',
      'neptune',
    ]))
  })

  it('gives Earth a thin bright shell and Jupiter animated weather', () => {
    const earth = atmosphereProfiles.earth
    const jupiter = atmosphereProfiles.jupiter

    expect(earth.scale).toBeGreaterThan(1)
    expect(earth.scale).toBeLessThan(1.05)
    expect(earth.intensity).toBeGreaterThan(jupiter.intensity)
    expect(jupiter.weatherOpacity).toBeGreaterThan(0)
    expect(jupiter.weatherSpeed).toBeGreaterThan(0)
  })

  it('keeps transparent atmosphere layers in the logarithmic depth buffer', () => {
    expect(solarMapSource.match(/#include <logdepthbuf_pars_vertex>/g)).toHaveLength(4)
    expect(solarMapSource.match(/#include <logdepthbuf_vertex>/g)).toHaveLength(4)
    expect(solarMapSource.match(/#include <logdepthbuf_pars_fragment>/g)).toHaveLength(4)
    expect(solarMapSource.match(/#include <logdepthbuf_fragment>/g)).toHaveLength(4)
  })
})
