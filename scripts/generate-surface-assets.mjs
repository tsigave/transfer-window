import { mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'
import sharp from 'sharp'

const repositoryRoot = resolve(import.meta.dirname, '..')
const cliArguments = process.argv.slice(2)
const scientificOnly = cliArguments.includes('--scientific-only')
const earthOnly = cliArguments.includes('--earth-only')
const sourceRoot = resolve(
  cliArguments.find((argument) => !['--scientific-only', '--earth-only'].includes(argument))
    ?? '/tmp/transfer-window-pbr-source',
)
const textureRoot = resolve(repositoryRoot, 'assets/textures')
const outputRoot = resolve(textureRoot, 'highres')
const WIDTH = 4096
const HEIGHT = 2048
const TERRAIN_WIDTH = 1024
const TERRAIN_HEIGHT = 512

await mkdir(outputRoot, { recursive: true })

function hashSeed(text) {
  let hash = 2166136261
  for (const character of text) {
    hash ^= character.charCodeAt(0)
    hash = Math.imul(hash, 16777619)
  }
  return hash >>> 0
}

function randomGenerator(seed) {
  let state = seed >>> 0
  return () => {
    state += 0x6d2b79f5
    let value = state
    value = Math.imul(value ^ value >>> 15, value | 1)
    value ^= value + Math.imul(value ^ value >>> 7, value | 61)
    return ((value ^ value >>> 14) >>> 0) / 4_294_967_296
  }
}

function smoothStep(value) {
  return value * value * (3 - 2 * value)
}

function addNoiseLayer(terrain, random, cellsX, amplitude) {
  const cellsY = Math.max(2, Math.round(cellsX / 2))
  const grid = Array.from({ length: cellsY + 1 }, () => (
    Float32Array.from({ length: cellsX }, random)
  ))
  grid[cellsY] = grid[cellsY - 1]

  for (let y = 0; y < TERRAIN_HEIGHT; y += 1) {
    const gridY = y / (TERRAIN_HEIGHT - 1) * cellsY
    const y0 = Math.min(Math.floor(gridY), cellsY - 1)
    const yBlend = smoothStep(gridY - y0)
    for (let x = 0; x < TERRAIN_WIDTH; x += 1) {
      const gridX = x / TERRAIN_WIDTH * cellsX
      const x0 = Math.floor(gridX) % cellsX
      const x1 = (x0 + 1) % cellsX
      const xBlend = smoothStep(gridX - Math.floor(gridX))
      const upper = grid[y0][x0] * (1 - xBlend) + grid[y0][x1] * xBlend
      const lower = grid[y0 + 1][x0] * (1 - xBlend) + grid[y0 + 1][x1] * xBlend
      terrain[y * TERRAIN_WIDTH + x] += (upper * (1 - yBlend) + lower * yBlend - 0.5) * amplitude
    }
  }
}

function addCraters(terrain, random, count, craterStrength) {
  for (let crater = 0; crater < count; crater += 1) {
    const centerX = random() * TERRAIN_WIDTH
    const centerY = (0.08 + random() * 0.84) * TERRAIN_HEIGHT
    const radius = 2.5 + Math.pow(random(), 2.2) * 54
    const latitudeScale = Math.max(0.25, Math.cos((centerY / TERRAIN_HEIGHT - 0.5) * Math.PI))
    const reachX = Math.ceil(radius / latitudeScale * 1.45)
    const reachY = Math.ceil(radius * 1.45)
    for (let offsetY = -reachY; offsetY <= reachY; offsetY += 1) {
      const y = Math.round(centerY + offsetY)
      if (y < 0 || y >= TERRAIN_HEIGHT) continue
      for (let offsetX = -reachX; offsetX <= reachX; offsetX += 1) {
        const wrappedX = (Math.round(centerX + offsetX) + TERRAIN_WIDTH) % TERRAIN_WIDTH
        const normalizedRadius = Math.hypot(offsetX * latitudeScale, offsetY) / radius
        if (normalizedRadius > 1.45) continue
        const bowl = normalizedRadius < 1 ? -(1 - normalizedRadius * normalizedRadius) : 0
        const rim = Math.exp(-Math.pow((normalizedRadius - 1.03) * 8.5, 2)) * 0.62
        terrain[y * TERRAIN_WIDTH + wrappedX] += (bowl + rim) * craterStrength
      }
    }
  }
}

function makeTerrain(bodyId, profile) {
  const random = randomGenerator(hashSeed(bodyId))
  const terrain = new Float32Array(TERRAIN_WIDTH * TERRAIN_HEIGHT)
  terrain.fill(0.5)
  addNoiseLayer(terrain, random, 7, 0.28)
  addNoiseLayer(terrain, random, 19, 0.15)
  addNoiseLayer(terrain, random, 53, 0.075)
  addNoiseLayer(terrain, random, 127, 0.032)
  addCraters(terrain, random, profile.craters ?? 110, profile.craterStrength ?? 0.13)

  let minimum = Number.POSITIVE_INFINITY
  let maximum = Number.NEGATIVE_INFINITY
  for (const value of terrain) {
    minimum = Math.min(minimum, value)
    maximum = Math.max(maximum, value)
  }
  const pixels = Buffer.allocUnsafe(terrain.length)
  for (let index = 0; index < terrain.length; index += 1) {
    pixels[index] = Math.round((terrain[index] - minimum) / (maximum - minimum) * 255)
  }
  return pixels
}

async function heightToNormal(heightPixels, outputPath, strength) {
  const { data, info } = await sharp(heightPixels, {
    raw: { width: TERRAIN_WIDTH, height: TERRAIN_HEIGHT, channels: 1 },
  })
    .resize(WIDTH, HEIGHT, { kernel: sharp.kernel.cubic })
    // The delivered height maps are 8-bit display derivatives. A small
    // prefilter prevents quantisation steps from becoming one-pixel bands.
    .blur(1)
    // libvips promotes resized one-channel raw input to sRGB; explicitly
    // collapse it again so the gradient index remains one byte per pixel.
    .greyscale()
    .raw()
    .toBuffer({ resolveWithObject: true })
  const normal = Buffer.allocUnsafe(info.width * info.height * 3)
  const gradientRadius = 2
  for (let y = 0; y < info.height; y += 1) {
    const north = Math.max(0, y - gradientRadius)
    const south = Math.min(info.height - 1, y + gradientRadius)
    const longitudeScale = 1 / Math.max(0.18, Math.cos((y / (info.height - 1) - 0.5) * Math.PI))
    for (let x = 0; x < info.width; x += 1) {
      const west = (x + info.width - gradientRadius) % info.width
      const east = (x + gradientRadius) % info.width
      const dx = (data[y * info.width + east] - data[y * info.width + west]) / 255
        * strength / gradientRadius * longitudeScale
      const dy = (data[south * info.width + x] - data[north * info.width + x]) / 255
        * strength / gradientRadius
      const inverseLength = 1 / Math.hypot(dx, dy, 1)
      const target = (y * info.width + x) * 3
      normal[target] = Math.round((-dx * inverseLength * 0.5 + 0.5) * 255)
      normal[target + 1] = Math.round((dy * inverseLength * 0.5 + 0.5) * 255)
      normal[target + 2] = Math.round((inverseLength * 0.5 + 0.5) * 255)
    }
  }
  await sharp(normal, { raw: { width: info.width, height: info.height, channels: 3 } })
    .webp({ lossless: true, effort: 6 })
    .toFile(outputPath)
}

async function writeArtisticSurface(bodyId, profile) {
  if (profile.relief === false) {
    await sharp(resolve(textureRoot, profile.source))
      .resize(WIDTH, HEIGHT, { fit: 'fill', kernel: sharp.kernel.lanczos3 })
      .webp({ quality: 90, smartSubsample: true, effort: 6 })
      .toFile(resolve(outputRoot, `${bodyId}.webp`))
    return
  }

  const height = makeTerrain(bodyId, profile)
  const heightOutput = resolve(outputRoot, `${bodyId}_height.webp`)
  await sharp(height, { raw: { width: TERRAIN_WIDTH, height: TERRAIN_HEIGHT, channels: 1 } })
    .resize(WIDTH, HEIGHT, { kernel: sharp.kernel.cubic })
    .webp({ lossless: true, effort: 6 })
    .toFile(heightOutput)
  await heightToNormal(height, resolve(outputRoot, `${bodyId}_normal.webp`), profile.normalStrength ?? 12)

  if (profile.source) {
    await sharp(resolve(textureRoot, profile.source))
      .resize(WIDTH, HEIGHT, { fit: 'fill', kernel: sharp.kernel.lanczos3 })
      .sharpen({ sigma: 0.8, m1: 0.45, m2: 0.9 })
      .webp({ quality: 90, smartSubsample: true, effort: 6 })
      .toFile(resolve(outputRoot, `${bodyId}.webp`))
    return
  }

  const palette = profile.palette
  const random = randomGenerator(hashSeed(`${bodyId}-albedo`))
  const color = Buffer.allocUnsafe(TERRAIN_WIDTH * TERRAIN_HEIGHT * 3)
  for (let y = 0; y < TERRAIN_HEIGHT; y += 1) {
    const latitude = y / (TERRAIN_HEIGHT - 1)
    for (let x = 0; x < TERRAIN_WIDTH; x += 1) {
      const index = y * TERRAIN_WIDTH + x
      const elevation = height[index] / 255
      const materialNoise = (random() - 0.5) * 0.045
      let blend = Math.min(1, Math.max(0, elevation * 0.72 + materialNoise + 0.14))
      if (bodyId === 'charon') blend += Math.max(0, 0.22 - latitude) * 0.72
      const target = index * 3
      for (let channel = 0; channel < 3; channel += 1) {
        color[target + channel] = Math.round(
          palette[0][channel] * (1 - blend) + palette[1][channel] * blend,
        )
      }
    }
  }
  await sharp(color, { raw: { width: TERRAIN_WIDTH, height: TERRAIN_HEIGHT, channels: 3 } })
    .resize(WIDTH, HEIGHT, { kernel: sharp.kernel.cubic })
    .webp({ quality: 90, smartSubsample: true, effort: 6 })
    .toFile(resolve(outputRoot, `${bodyId}.webp`))
}

async function convertScientificHeight(sourceName, bodyId, normalStrength) {
  const source = resolve(sourceRoot, sourceName)
  const heightOutput = resolve(outputRoot, `${bodyId}_height.webp`)
  await sharp(source, { limitInputPixels: false })
    .resize(WIDTH, HEIGHT, { fit: 'fill', kernel: sharp.kernel.lanczos3 })
    .normalise()
    .greyscale()
    .webp({ lossless: true, effort: 6 })
    .toFile(heightOutput)
  // Encoded WebP files are decoded as RGB by libvips even when their source was
  // greyscale. Collapse back to one channel before passing the raw buffer to the
  // normal-map generator; otherwise RGB bytes are misread as adjacent scanlines.
  const height = await sharp(heightOutput)
    .resize(TERRAIN_WIDTH, TERRAIN_HEIGHT)
    .greyscale()
    .raw()
    .toBuffer()
  await heightToNormal(height, resolve(outputRoot, `${bodyId}_normal.webp`), normalStrength)
}

async function convertEarthDem() {
  const { data, info } = await sharp(resolve(sourceRoot, 'earth_dem.tif'), {
    limitInputPixels: false,
  })
    .resize(WIDTH, HEIGHT, { fit: 'fill', kernel: sharp.kernel.lanczos3 })
    // Keep the source's floating-point metre values. Calling greyscale here
    // would route through an 8-bit colour conversion and clip elevations.
    .raw({ depth: 'float' })
    .toBuffer({ resolveWithObject: true })
  const elevations = new Float32Array(data.buffer, data.byteOffset, data.byteLength / 4)
  let maximumLandElevation = 0
  for (let index = 0; index < elevations.length; index += info.channels) {
    const elevation = elevations[index]
    if (Number.isFinite(elevation)) maximumLandElevation = Math.max(maximumLandElevation, elevation)
  }
  const height = Buffer.allocUnsafe(info.width * info.height)
  for (let index = 0; index < height.length; index += 1) {
    const sourceElevation = elevations[index * info.channels]
    const elevation = Number.isFinite(sourceElevation) ? sourceElevation : 0
    // The visual surface represents the ocean surface rather than the sea
    // floor, so bathymetry is clamped to sea level before displacement.
    height[index] = Math.round(Math.max(0, elevation) / maximumLandElevation * 255)
  }
  await sharp(height, { raw: { width: info.width, height: info.height, channels: 1 } })
    .webp({ lossless: true, effort: 6 })
    .toFile(resolve(outputRoot, 'earth_height.webp'))
  const normalInput = await sharp(height, {
    raw: { width: info.width, height: info.height, channels: 1 },
  })
    .resize(TERRAIN_WIDTH, TERRAIN_HEIGHT, { kernel: sharp.kernel.cubic })
    .greyscale()
    .raw()
    .toBuffer()
  await heightToNormal(normalInput, resolve(outputRoot, 'earth_normal.webp'), 8)
}

await convertEarthDem()
await sharp(resolve(sourceRoot, 'earth_specular.tif'), { limitInputPixels: false })
  .greyscale()
  .negate()
  .linear(0.72, 46)
  .webp({ lossless: true, effort: 6 })
  .toFile(resolve(outputRoot, 'earth_roughness.webp'))

if (!earthOnly) {
  await sharp(resolve(sourceRoot, 'moon_color.tif'), { limitInputPixels: false })
    .jpeg({ quality: 92, chromaSubsampling: '4:4:4' })
    .toFile(resolve(outputRoot, 'moon.jpg'))

  await convertScientificHeight('moon_height.tif', 'moon', 15)
  await convertScientificHeight('mars_dem.tif', 'mars', 13)
  await convertScientificHeight('mercury_dem.tif', 'mercury', 14)
}

const observedProfiles = {
  uranus: { source: 'uranus.jpg', relief: false },
  neptune: { source: 'neptune.jpg', relief: false },
  io: { source: 'io.jpg', craters: 34, craterStrength: 0.045, normalStrength: 5 },
  europa: { source: 'europa.jpg', craters: 12, craterStrength: 0.025, normalStrength: 4 },
  ganymede: { source: 'ganymede.jpg', craters: 115, craterStrength: 0.09, normalStrength: 9 },
  callisto: { source: 'callisto.jpg', craters: 210, craterStrength: 0.14, normalStrength: 12 },
  mimas: { source: 'mimas.jpg', craters: 170, craterStrength: 0.16, normalStrength: 14 },
  enceladus: { source: 'enceladus.jpg', craters: 35, craterStrength: 0.055, normalStrength: 7 },
  tethys: { source: 'tethys.jpg', craters: 145, craterStrength: 0.13, normalStrength: 13 },
  dione: { source: 'dione.jpg', craters: 130, craterStrength: 0.12, normalStrength: 12 },
  rhea: { source: 'rhea.jpg', craters: 165, craterStrength: 0.14, normalStrength: 13 },
  titan: { source: 'titan.jpg', craters: 18, craterStrength: 0.025, normalStrength: 3 },
  iapetus: { source: 'iapetus.jpg', craters: 125, craterStrength: 0.12, normalStrength: 11 },
  pluto: { source: 'pluto.jpg', craters: 95, craterStrength: 0.085, normalStrength: 9 },
  ariel: { source: 'ariel.jpg', craters: 75, craterStrength: 0.085, normalStrength: 9 },
  umbriel: { source: 'umbriel.jpg', craters: 150, craterStrength: 0.13, normalStrength: 12 },
  titania: { source: 'titania.jpg', craters: 100, craterStrength: 0.1, normalStrength: 10 },
  miranda: { source: 'miranda.jpg', craters: 80, craterStrength: 0.095, normalStrength: 11 },
}

const reconstructedProfiles = {
  oberon: { palette: [[47, 43, 42], [145, 135, 129]], craters: 150, normalStrength: 12 },
  triton: { palette: [[92, 92, 81], [196, 180, 151]], craters: 58, normalStrength: 7 },
  charon: { palette: [[45, 43, 44], [152, 146, 143]], craters: 125, normalStrength: 11 },
  vesta: { palette: [[51, 48, 44], [157, 148, 133]], craters: 175, normalStrength: 14 },
  pallas: { palette: [[39, 44, 46], [117, 126, 126]], craters: 155, normalStrength: 13 },
  hygiea: { palette: [[25, 29, 30], [78, 85, 83]], craters: 135, normalStrength: 11 },
  psyche: { palette: [[47, 42, 38], [116, 102, 88]], craters: 120, normalStrength: 12 },
  chiron: { palette: [[40, 37, 34], [105, 97, 87]], craters: 145, normalStrength: 12 },
  arrokoth: { palette: [[53, 31, 25], [139, 79, 61]], craters: 48, normalStrength: 8 },
}

if (!scientificOnly && !earthOnly) {
  for (const [bodyId, profile] of Object.entries({ ...observedProfiles, ...reconstructedProfiles })) {
    await writeArtisticSurface(bodyId, profile)
  }

  for (const bodyId of ['ceres', 'eris', 'haumea', 'makemake']) {
    const profile = { source: `highres/${bodyId}.jpg`, craters: 125, normalStrength: 10 }
    await writeArtisticSurface(bodyId, profile)
  }
}

console.log(`Generated focused surface assets in ${outputRoot}`)
