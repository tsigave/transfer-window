import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { atmosphereProfiles } from './atmosphere'
import {
  irregularShapeBodyIds,
  surfaceMaterialProfile,
  volumePreservingBodyScale,
} from './celestialAppearance'
import solarMapSource from './SolarMap.tsx?raw'

function readGlbJson(fileName: string) {
  const bytes = readFileSync(resolve('assets/models', fileName))
  expect(bytes.toString('ascii', 0, 4)).toBe('glTF')
  const jsonLength = bytes.readUInt32LE(12)
  return JSON.parse(bytes.toString('utf8', 20, 20 + jsonLength)) as {
    accessors: Array<{ count: number, min?: number[], max?: number[] }>
    images: Array<{ bufferView?: number, mimeType?: string }>
  }
}

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

describe('celestial surface appearance', () => {
  it('uses measured irregular meshes for both Martian moons', () => {
    expect(irregularShapeBodyIds).toEqual(['phobos', 'deimos'])
    expect(solarMapSource).toContain("phobos: new URL('../assets/models/phobos.glb'")
    expect(solarMapSource).toContain("deimos: new URL('../assets/models/deimos.glb'")
    expect(solarMapSource).toContain('new GLTFLoader()')
  })

  it.each([
    ['phobos.glb', 16_449],
    ['deimos.glb', 16_649],
  ])('bundles a textured, non-spherical %s asset', (fileName, vertexCount) => {
    const model = readGlbJson(fileName)
    const positions = model.accessors[0]
    const extents = positions.max!.map((maximum, index) => maximum - positions.min![index])

    expect(positions.count).toBe(vertexCount)
    expect(Math.max(...extents) / Math.min(...extents)).toBeGreaterThan(1.2)
    expect(model.images).toContainEqual(expect.objectContaining({
      bufferView: expect.any(Number),
      mimeType: 'image/png',
    }))
  })

  it('keeps rocky surfaces non-metallic and gives mapped moons light relief', () => {
    const moon = surfaceMaterialProfile('moon', 'moon', true)
    const jupiter = surfaceMaterialProfile('jupiter', 'planet', true)

    expect(moon.roughness).toBeGreaterThan(0.9)
    expect(moon.bumpScaleRatio).toBeGreaterThan(0)
    expect(jupiter.bumpScaleRatio).toBe(0)
    expect(solarMapSource).toContain('metalness: 0')
  })

  it('renders giant planets as volume-preserving oblate spheroids', () => {
    const [equatorial, polar] = volumePreservingBodyScale('saturn')

    expect(equatorial).toBeGreaterThan(1)
    expect(polar).toBeLessThan(1)
    expect(equatorial * equatorial * polar).toBeCloseTo(1, 10)
  })
})
