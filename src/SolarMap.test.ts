import { readFileSync, readdirSync } from 'node:fs'
import { resolve } from 'node:path'
import sharp from 'sharp'
import { describe, expect, it } from 'vitest'
import { atmosphereProfiles } from './atmosphere'
import {
  irregularShapeBodyIds,
  surfaceMaterialProfile,
  volumePreservingBodyScale,
} from './celestialAppearance'
import { catalog } from './model'
import { focusedSurfaceAssets } from './surfaceAssets'
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

  it('provides a high-resolution focus surface for every spherical catalog body', () => {
    const irregularBodyIds = new Set<string>(irregularShapeBodyIds)
    const sphericalBodyIds = catalog.bodies
      .map((body) => body.id)
      .filter((bodyId) => !irregularBodyIds.has(bodyId))
      .sort()

    expect(Object.keys(focusedSurfaceAssets).sort()).toEqual(sphericalBodyIds)
    expect(focusedSurfaceAssets.earth).toEqual(expect.objectContaining({
      normal: expect.any(String),
      height: expect.any(String),
      roughness: expect.any(String),
      cloud: expect.any(String),
      night: expect.any(String),
      provenance: 'observed',
    }))
    expect(focusedSurfaceAssets.moon).toEqual(expect.objectContaining({
      normal: expect.any(String),
      height: expect.any(String),
      provenance: 'observed',
    }))
  })

  it('labels completed global surfaces as artistic reconstruction', () => {
    expect([
      'ceres', 'eris', 'haumea', 'makemake', 'oberon', 'triton', 'charon',
      'vesta', 'pallas', 'hygiea', 'psyche', 'chiron', 'arrokoth',
    ].map((bodyId) => focusedSurfaceAssets[bodyId].provenance))
      .toEqual(Array.from({ length: 13 }, () => 'artistic_reconstruction'))
    expect(solarMapSource).toContain('聚焦表面：')
  })

  it('keeps every focus texture at 4K or higher', async () => {
    const textureNames = readdirSync(resolve('assets/textures/highres'))
    expect(textureNames.length).toBeGreaterThan(80)

    await Promise.all(textureNames.map(async (textureName) => {
      const metadata = await sharp(resolve('assets/textures/highres', textureName)).metadata()
      expect(metadata.width, textureName).toBeGreaterThanOrEqual(4096)
      if (textureName !== 'saturn_ring.png') {
        expect(metadata.height, textureName).toBeGreaterThanOrEqual(2048)
      }
    }))
  })

  it('keeps the LOLA normal map free of scanline-channel banding', async () => {
    const { data, info } = await sharp(resolve('assets/textures/highres/moon_normal.webp'))
      .raw()
      .toBuffer({ resolveWithObject: true })
    let horizontalDifference = 0
    let verticalDifference = 0
    let samples = 0
    for (let y = 32; y < info.height - 32; y += 31) {
      for (let x = 32; x < info.width - 32; x += 31) {
        const center = (y * info.width + x) * info.channels
        const east = center + info.channels
        const south = center + info.width * info.channels
        for (let channel = 0; channel < 3; channel += 1) {
          horizontalDifference += Math.abs(data[east + channel] - data[center + channel])
          verticalDifference += Math.abs(data[south + channel] - data[center + channel])
          samples += 1
        }
      }
    }

    const horizontalAverage = horizontalDifference / samples
    const verticalAverage = verticalDifference / samples
    expect(horizontalAverage).toBeGreaterThan(0.1)
    expect(verticalAverage / horizontalAverage).toBeLessThan(2.5)
  })

  it('renders giant planets as volume-preserving oblate spheroids', () => {
    const [equatorial, polar] = volumePreservingBodyScale('saturn')

    expect(equatorial).toBeGreaterThan(1)
    expect(polar).toBeLessThan(1)
    expect(equatorial * equatorial * polar).toBeCloseTo(1, 10)
  })
})
