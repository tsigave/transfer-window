import type { BodyClass } from './model'

export interface SurfaceMaterialProfile {
  roughness: number
  bumpScaleRatio: number
}

const gasAndCloudBodies = new Set([
  'venus',
  'jupiter',
  'saturn',
  'uranus',
  'neptune',
  'titan',
])

const gentlyRelievedBodies = new Set([
  'earth',
  'mars',
  'pluto',
  'charon',
  'triton',
])

export const irregularShapeBodyIds = ['phobos', 'deimos'] as const

// IAU/NASA reference flattening values. The scale factors preserve volume, so
// the catalog mean radius remains the visual size authority.
export const equatorialFlattening: Readonly<Record<string, number>> = {
  earth: 1 / 298.257_223_563,
  mars: 1 / 169.8,
  jupiter: 0.06487,
  saturn: 0.09796,
  uranus: 0.02293,
  neptune: 0.01708,
}

export function surfaceMaterialProfile(
  bodyId: string,
  bodyClass: BodyClass,
  hasTexture: boolean,
): SurfaceMaterialProfile {
  if (bodyClass === 'star') return { roughness: 0.48, bumpScaleRatio: 0 }
  if (!hasTexture || gasAndCloudBodies.has(bodyId)) {
    return { roughness: gasAndCloudBodies.has(bodyId) ? 0.86 : 0.94, bumpScaleRatio: 0 }
  }
  if (gentlyRelievedBodies.has(bodyId)) return { roughness: 0.9, bumpScaleRatio: 0.012 }
  if (bodyClass === 'moon') return { roughness: 0.97, bumpScaleRatio: 0.028 }
  if (bodyClass === 'planet' || bodyClass === 'dwarf_planet') {
    return { roughness: 0.94, bumpScaleRatio: 0.022 }
  }
  return { roughness: 0.98, bumpScaleRatio: 0.035 }
}

export function volumePreservingBodyScale(bodyId: string): [number, number, number] {
  const flattening = equatorialFlattening[bodyId] ?? 0
  if (flattening === 0) return [1, 1, 1]
  const equatorialScale = Math.pow(1 / (1 - flattening), 1 / 3)
  return [equatorialScale, equatorialScale * (1 - flattening), equatorialScale]
}
