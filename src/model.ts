import catalogDocument from '../data/catalog/solar-system-v1.json'

export type BodyClass = 'star' | 'planet' | 'dwarf_planet' | 'moon' | 'asteroid' | 'centaur' | 'kuiper_belt_object'

export interface OrbitalElements {
  semi_major_axis_m: number
  eccentricity: number
  inclination_deg: number
  longitude_ascending_node_deg: number
  argument_periapsis_deg: number
  mean_anomaly_at_epoch_deg: number
  orbital_period_s: number
  epoch_tdb_micros: number
}

export interface Body {
  id: string
  canonical_name: string
  localized_name_zh: string
  aliases: string[]
  parent_id: string | null
  body_class: BodyClass
  mass_kg: number
  mean_radius_m: number
  rotation_period_s: number | null
  ephemeris: OrbitalElements | null
  ephemeris_source: { name: string; url: string; kind: 'public_reference' | 'approximation' }
  data_quality: 'reference' | 'approximate'
  discovery_status: 'observed'
  development_status: 'observed'
}

export interface Region {
  id: string
  canonical_name: string
  localized_name_zh: string
  inner_radius_m: number
  outer_radius_m: number
  description: string
}

export interface StateVector {
  position_m: [number, number, number]
  velocity_mps: [number, number, number]
}

export const catalog = catalogDocument as {
  schema_version: number
  content_version: string
  epoch: string
  bodies: Body[]
  regions: Region[]
}

export const bodyById = new Map(catalog.bodies.map((body) => [body.id, body]))

export function searchBodies(query: string): Body[] {
  const normalized = query.trim().toLocaleLowerCase()
  if (!normalized) return catalog.bodies
  return catalog.bodies.filter((body) =>
    [body.id, body.canonical_name, body.localized_name_zh, ...body.aliases]
      .some((value) => value.toLocaleLowerCase().includes(normalized)),
  )
}

function rotate(elements: OrbitalElements, vector: [number, number, number]): [number, number, number] {
  const node = elements.longitude_ascending_node_deg * Math.PI / 180
  const inclination = elements.inclination_deg * Math.PI / 180
  const periapsis = elements.argument_periapsis_deg * Math.PI / 180
  const [sn, cn] = [Math.sin(node), Math.cos(node)]
  const [si, ci] = [Math.sin(inclination), Math.cos(inclination)]
  const [sp, cp] = [Math.sin(periapsis), Math.cos(periapsis)]
  const matrix = [
    [cn * cp - sn * sp * ci, -cn * sp - sn * cp * ci, sn * si],
    [sn * cp + cn * sp * ci, -sn * sp + cn * cp * ci, -cn * si],
    [sp * si, cp * si, ci],
  ]
  return matrix.map((row) => row.reduce((sum, value, index) => sum + value * vector[index], 0)) as [number, number, number]
}

export function localState(body: Body, epochTdbMicros: number): StateVector {
  const elements = body.ephemeris
  if (!elements) return { position_m: [0, 0, 0], velocity_mps: [0, 0, 0] }
  const deltaSeconds = (epochTdbMicros - elements.epoch_tdb_micros) / 1e6
  const meanMotion = Math.PI * 2 / elements.orbital_period_s
  const meanAnomaly = (elements.mean_anomaly_at_epoch_deg * Math.PI / 180 + meanMotion * deltaSeconds) % (Math.PI * 2)
  let eccentricAnomaly = meanAnomaly
  for (let iteration = 0; iteration < 12; iteration += 1) {
    eccentricAnomaly -= (eccentricAnomaly - elements.eccentricity * Math.sin(eccentricAnomaly) - meanAnomaly)
      / (1 - elements.eccentricity * Math.cos(eccentricAnomaly))
  }
  const a = elements.semi_major_axis_m
  const e = elements.eccentricity
  const denominator = 1 - e * Math.cos(eccentricAnomaly)
  return {
    position_m: rotate(elements, [a * (Math.cos(eccentricAnomaly) - e), a * Math.sqrt(1 - e * e) * Math.sin(eccentricAnomaly), 0]),
    velocity_mps: rotate(elements, [-a * meanMotion * Math.sin(eccentricAnomaly) / denominator, a * meanMotion * Math.sqrt(1 - e * e) * Math.cos(eccentricAnomaly) / denominator, 0]),
  }
}

export function heliocentricState(body: Body, epochTdbMicros: number): StateVector {
  const local = localState(body, epochTdbMicros)
  if (!body.parent_id) return local
  const parent = bodyById.get(body.parent_id)
  if (!parent) throw new Error(`CATALOG_INVALID: missing parent ${body.parent_id}`)
  const origin = heliocentricState(parent, epochTdbMicros)
  return {
    position_m: local.position_m.map((value, index) => value + origin.position_m[index]) as [number, number, number],
    velocity_mps: local.velocity_mps.map((value, index) => value + origin.velocity_mps[index]) as [number, number, number],
  }
}

export const j2000UtcMs = Date.UTC(2000, 0, 1, 11, 58, 55, 816)

export function epochFromDate(date: Date): number {
  // The browser display adapter is UTC-like; authoritative leap-second conversion remains in sim-time.
  return (date.getTime() - j2000UtcMs) * 1000
}

export function dateFromEpoch(epochTdbMicros: number): Date {
  return new Date(j2000UtcMs + epochTdbMicros / 1000)
}

export function childrenOf(parentId: string | null): Body[] {
  return catalog.bodies.filter((body) => body.parent_id === parentId)
}

