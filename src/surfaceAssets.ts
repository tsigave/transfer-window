export type SurfaceProvenance = 'observed' | 'observed_with_artistic_relief' | 'artistic_reconstruction'

export interface FocusedSurfaceAsset {
  albedo: string
  normal?: string
  height?: string
  roughness?: string
  cloud?: string
  night?: string
  normalScale?: number
  reliefScaleRatio?: number
  reliefBiasRatio?: number
  provenance: SurfaceProvenance
}

const bundledHighresAssets = import.meta.glob<string>(
  '../assets/textures/highres/*.webp',
  { eager: true, query: '?url', import: 'default' },
)

const highres = (name: string) => {
  const url = bundledHighresAssets[`../assets/textures/highres/${name}`]
  if (!url) throw new Error(`Missing bundled high-resolution surface asset: ${name}`)
  return url
}

// The eager glob gives Vite a finite asset set to fingerprint while the
// renderer still loads image bytes only when the corresponding body is focused.
export const focusedSurfaceAssets: Readonly<Record<string, FocusedSurfaceAsset>> = {
  sun: {
    albedo: new URL('../assets/textures/highres/sun.jpg', import.meta.url).href,
    provenance: 'observed',
  },
  mercury: {
    albedo: new URL('../assets/textures/highres/mercury.jpg', import.meta.url).href,
    normal: new URL('../assets/textures/highres/mercury_normal.webp', import.meta.url).href,
    height: new URL('../assets/textures/highres/mercury_height.webp', import.meta.url).href,
    normalScale: 0.82,
    reliefScaleRatio: 0.0045,
    provenance: 'observed',
  },
  venus: {
    albedo: new URL('../assets/textures/highres/venus.jpg', import.meta.url).href,
    provenance: 'observed',
  },
  earth: {
    albedo: new URL('../assets/textures/highres/earth.jpg', import.meta.url).href,
    normal: new URL('../assets/textures/highres/earth_normal.webp', import.meta.url).href,
    height: new URL('../assets/textures/highres/earth_height.webp', import.meta.url).href,
    roughness: new URL('../assets/textures/highres/earth_roughness.webp', import.meta.url).href,
    cloud: new URL('../assets/textures/highres/earth_clouds.jpg', import.meta.url).href,
    night: new URL('../assets/textures/highres/earth_night.jpg', import.meta.url).href,
    normalScale: 0.62,
    reliefScaleRatio: 0.0014,
    reliefBiasRatio: 0,
    provenance: 'observed',
  },
  mars: {
    albedo: new URL('../assets/textures/highres/mars.jpg', import.meta.url).href,
    normal: new URL('../assets/textures/highres/mars_normal.webp', import.meta.url).href,
    height: new URL('../assets/textures/highres/mars_height.webp', import.meta.url).href,
    normalScale: 0.8,
    reliefScaleRatio: 0.0065,
    provenance: 'observed',
  },
  jupiter: {
    albedo: new URL('../assets/textures/highres/jupiter.jpg', import.meta.url).href,
    provenance: 'observed',
  },
  saturn: {
    albedo: new URL('../assets/textures/highres/saturn.jpg', import.meta.url).href,
    provenance: 'observed',
  },
  uranus: {
    albedo: new URL('../assets/textures/highres/uranus.webp', import.meta.url).href,
    provenance: 'observed',
  },
  neptune: {
    albedo: new URL('../assets/textures/highres/neptune.webp', import.meta.url).href,
    provenance: 'observed',
  },
  moon: {
    albedo: new URL('../assets/textures/highres/moon.jpg', import.meta.url).href,
    normal: new URL('../assets/textures/highres/moon_normal.webp', import.meta.url).href,
    height: new URL('../assets/textures/highres/moon_height.webp', import.meta.url).href,
    normalScale: 0.9,
    reliefScaleRatio: 0.009,
    provenance: 'observed',
  },
  io: rockyObserved('io', 0.0034, 0.66),
  europa: rockyObserved('europa', 0.0028, 0.58),
  ganymede: rockyObserved('ganymede', 0.0048, 0.76),
  callisto: rockyObserved('callisto', 0.006, 0.86),
  mimas: rockyObserved('mimas', 0.016, 0.92),
  enceladus: rockyObserved('enceladus', 0.012, 0.72),
  tethys: rockyObserved('tethys', 0.014, 0.88),
  dione: rockyObserved('dione', 0.01, 0.82),
  rhea: rockyObserved('rhea', 0.012, 0.88),
  titan: rockyObserved('titan', 0.002, 0.35),
  iapetus: rockyObserved('iapetus', 0.02, 0.9),
  ariel: rockyObserved('ariel', 0.013, 0.82),
  umbriel: rockyObserved('umbriel', 0.012, 0.86),
  titania: rockyObserved('titania', 0.011, 0.84),
  miranda: rockyObserved('miranda', 0.025, 0.95),
  pluto: rockyObserved('pluto', 0.006, 0.72),
  ceres: rockyArtistic('ceres', 0.012, 0.88),
  eris: rockyArtistic('eris', 0.006, 0.65),
  haumea: rockyArtistic('haumea', 0.008, 0.7),
  makemake: rockyArtistic('makemake', 0.007, 0.68),
  oberon: rockyArtistic('oberon', 0.014, 0.88),
  triton: rockyArtistic('triton', 0.006, 0.7),
  charon: rockyArtistic('charon', 0.012, 0.84),
  vesta: rockyArtistic('vesta', 0.028, 0.95),
  pallas: rockyArtistic('pallas', 0.02, 0.9),
  hygiea: rockyArtistic('hygiea', 0.014, 0.82),
  psyche: rockyArtistic('psyche', 0.02, 0.88),
  chiron: rockyArtistic('chiron', 0.025, 0.92),
  arrokoth: rockyArtistic('arrokoth', 0.04, 0.95),
}

function rockyObserved(bodyId: string, reliefScaleRatio: number, normalScale: number): FocusedSurfaceAsset {
  return {
    albedo: highres(`${bodyId}.webp`),
    normal: highres(`${bodyId}_normal.webp`),
    height: highres(`${bodyId}_height.webp`),
    reliefScaleRatio,
    normalScale,
    provenance: 'observed_with_artistic_relief',
  }
}

function rockyArtistic(bodyId: string, reliefScaleRatio: number, normalScale: number): FocusedSurfaceAsset {
  return {
    albedo: highres(`${bodyId}.webp`),
    normal: highres(`${bodyId}_normal.webp`),
    height: highres(`${bodyId}_height.webp`),
    reliefScaleRatio,
    normalScale,
    provenance: 'artistic_reconstruction',
  }
}
