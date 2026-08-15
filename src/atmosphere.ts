export interface AtmosphereProfile {
  color: number
  nightColor: number
  scale: number
  intensity: number
  rimPower: number
  weatherOpacity?: number
  weatherSpeed?: number
}

export const atmosphereProfiles: Readonly<Record<string, AtmosphereProfile>> = {
  venus: {
    color: 0xffc078,
    nightColor: 0x8f3f24,
    scale: 1.04,
    intensity: 0.62,
    rimPower: 2.15,
  },
  earth: {
    color: 0x72cfff,
    nightColor: 0x214f9d,
    scale: 1.038,
    intensity: 0.86,
    rimPower: 2.35,
  },
  mars: {
    color: 0xe99563,
    nightColor: 0x713421,
    scale: 1.018,
    intensity: 0.27,
    rimPower: 2.7,
  },
  jupiter: {
    color: 0xf1c7a5,
    nightColor: 0x70423a,
    scale: 1.027,
    intensity: 0.48,
    rimPower: 2.25,
    weatherOpacity: 0.2,
    weatherSpeed: 1,
  },
  saturn: {
    color: 0xf7dfa7,
    nightColor: 0x79613d,
    scale: 1.025,
    intensity: 0.4,
    rimPower: 2.3,
    weatherOpacity: 0.1,
    weatherSpeed: 0.62,
  },
  uranus: {
    color: 0xa3f3ff,
    nightColor: 0x326b80,
    scale: 1.034,
    intensity: 0.5,
    rimPower: 2.2,
    weatherOpacity: 0.075,
    weatherSpeed: 0.45,
  },
  neptune: {
    color: 0x64a6ff,
    nightColor: 0x202f84,
    scale: 1.04,
    intensity: 0.66,
    rimPower: 2.15,
    weatherOpacity: 0.14,
    weatherSpeed: 1.35,
  },
  titan: {
    color: 0xffb35c,
    nightColor: 0x804521,
    scale: 1.055,
    intensity: 0.68,
    rimPower: 2.05,
  },
}
