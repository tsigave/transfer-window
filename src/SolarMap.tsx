import { useEffect, useRef } from 'react'
import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { CSS2DObject, CSS2DRenderer } from 'three/examples/jsm/renderers/CSS2DRenderer.js'
import { bodyById, childrenOf, heliocentricState, localState, type Body } from './model'

const AU = 149_597_870_700
const palette: Record<string, number> = {
  sun: 0xffc85a,
  mercury: 0x9f9890,
  venus: 0xd9a85c,
  earth: 0x4b91bd,
  mars: 0xb75e3c,
  jupiter: 0xc69a72,
  saturn: 0xd8bd7d,
  uranus: 0x73c9d3,
  neptune: 0x4969bf,
  moon: 0xb9c1c4,
}

// Rotation-axis inclination to each body's orbital plane (degrees).
// Major-planet values follow NASA's planetary reference table.
const axialTiltDegrees: Record<string, number> = {
  sun: 7.25,
  mercury: 0.034,
  venus: 177.36,
  earth: 23.44,
  moon: 6.68,
  mars: 25.19,
  jupiter: 3.13,
  saturn: 26.73,
  uranus: 97.77,
  neptune: 28.32,
  pluto: 119.61,
}

export interface CameraAction {
  id: number
  type: 'zoom-in' | 'zoom-out' | 'reset'
}

interface Props {
  epochTdbMicros: number
  timeRate: number
  selectedId: string
  focusId: string
  viewPreset: 'perspective' | 'top'
  cameraAction: CameraAction
  onSelect: (id: string) => void
  onFocus: (id: string) => void
}

interface BodyVisual {
  body: Body
  root: THREE.Group
  axisGroup: THREE.Group
  mesh: THREE.Mesh<THREE.SphereGeometry, THREE.MeshStandardMaterial>
  material: THREE.MeshStandardMaterial
  baseRadius: number
  glow?: THREE.Sprite
  cloudMesh?: THREE.Mesh<THREE.SphereGeometry, THREE.MeshStandardMaterial>
  nightMesh?: THREE.Mesh<THREE.SphereGeometry, THREE.ShaderMaterial>
  labelObject: CSS2DObject
  label: HTMLDivElement
  selectionHalo: THREE.Sprite
}

interface ScopeModel {
  focus: Body
  maxRadius: number
  global: boolean
  bodies: Body[]
  contextExtent: number
  focusRadiusScene: number
}

interface OrbitVisual {
  root: THREE.Group
  body: Body
  flowGeometry: THREE.BufferGeometry
  flowCurve: THREE.CatmullRomCurve3
  flowTrailLength: number
  phaseLead: number
  anchorBodyId?: string
}

function bodyColor(body: Body): number {
  if (palette[body.id]) return palette[body.id]
  if (body.body_class === 'moon') return 0xaab8ba
  if (body.body_class === 'dwarf_planet') return 0xa486bd
  if (body.body_class === 'asteroid' || body.body_class === 'centaur') return 0xb87d55
  return 0x7283ad
}

function bodyRadius(body: Body, isFocus: boolean, isGlobal: boolean): number {
  if (isFocus) return body.body_class === 'star' ? 7 : 5.5
  if (body.body_class === 'star') return 4.5
  const physicalHint = Math.log10(Math.max(body.mean_radius_m, 1)) - 3.7
  if (body.body_class === 'planet') return THREE.MathUtils.clamp(physicalHint, 1.8, 3.6)
  if (body.body_class === 'dwarf_planet') return isGlobal ? 1.2 : 1.7
  return isGlobal ? 0.85 : 1.35
}

const textureUrls: Record<string, string> = {
  sun: new URL('../assets/textures/sun.jpg', import.meta.url).href,
  mercury: new URL('../assets/textures/mercury.jpg', import.meta.url).href,
  venus: new URL('../assets/textures/venus_atmosphere.jpg', import.meta.url).href,
  earth: new URL('../assets/textures/earth_daymap.jpg', import.meta.url).href,
  moon: new URL('../assets/textures/moon.jpg', import.meta.url).href,
  europa: new URL('../assets/textures/europa.jpg', import.meta.url).href,
  ganymede: new URL('../assets/textures/ganymede.jpg', import.meta.url).href,
  callisto: new URL('../assets/textures/callisto.jpg', import.meta.url).href,
  mars: new URL('../assets/textures/mars.jpg', import.meta.url).href,
  jupiter: new URL('../assets/textures/jupiter.jpg', import.meta.url).href,
  saturn: new URL('../assets/textures/saturn.jpg', import.meta.url).href,
  uranus: new URL('../assets/textures/uranus.jpg', import.meta.url).href,
  neptune: new URL('../assets/textures/neptune.jpg', import.meta.url).href,
  ceres: new URL('../assets/textures/ceres.jpg', import.meta.url).href,
  pluto: new URL('../assets/textures/pluto.jpg', import.meta.url).href,
  eris: new URL('../assets/textures/eris.jpg', import.meta.url).href,
  haumea: new URL('../assets/textures/haumea.jpg', import.meta.url).href,
  makemake: new URL('../assets/textures/makemake.jpg', import.meta.url).href,
}

const saturnRingUrl = new URL('../assets/textures/saturn_ring.png', import.meta.url).href
const earthCloudUrl = new URL('../assets/textures/earth_clouds.jpg', import.meta.url).href
const earthNightUrl = new URL('../assets/textures/earth_nightmap.jpg', import.meta.url).href
const starfieldUrl = new URL('../assets/textures/starfield-j2000-8k.jpg', import.meta.url).href
const LOCAL_REFERENCE_RADIUS = 72
const LOCAL_MAX_CONTEXT_EXTENT = 120_000
const FOCUS_CAMERA_RADII = 16
const LOCAL_SUN_RADIUS_BOOST = 1.45
const LOCAL_SUN_GLOW_MULTIPLIER = 5.2
const VISUAL_FOCUS_OFFSET_RADII = 3.6

function sunAppearance(distanceAu: number, global: boolean) {
  if (global) {
    return { glowSize: 30, emissiveIntensity: 3.4, lightIntensity: 9.5 }
  }
  const safeDistance = Math.max(distanceAu, 0.2)
  const irradiance = THREE.MathUtils.clamp(1 / (safeDistance * safeDistance), 0.28, 5)
  return {
    glowSize: 30,
    emissiveIntensity: 3.6 + Math.sqrt(irradiance) * 1.15,
    lightIntensity: 7.5 * irradiance,
  }
}

function relativePosition(body: Body, focus: Body, epochTdbMicros: number): [number, number, number] {
  if (body.id === focus.id) return [0, 0, 0]
  if (body.parent_id === focus.id) return localState(body, epochTdbMicros).position_m
  const state = heliocentricState(body, epochTdbMicros).position_m
  const origin = heliocentricState(focus, epochTdbMicros).position_m
  return state.map((value, index) => value - origin[index]) as [number, number, number]
}

function displayPosition(
  source: [number, number, number],
  maxRadius: number,
  global: boolean,
): THREE.Vector3 {
  const actualRadius = Math.hypot(...source)
  if (actualRadius === 0) return new THREE.Vector3()
  const multiplier = global
    ? 178 * Math.log1p(actualRadius / (0.08 * AU))
      / Math.log1p(maxRadius / (0.08 * AU)) / actualRadius
    : LOCAL_REFERENCE_RADIUS / maxRadius
  // Three.js uses Y as up; the ecliptic X/Y plane becomes the scene X/Z plane.
  return new THREE.Vector3(
    source[0] * multiplier,
    source[2] * multiplier,
    source[1] * multiplier,
  )
}

function createGlowTexture(): THREE.CanvasTexture {
  const canvas = document.createElement('canvas')
  canvas.width = 128
  canvas.height = 128
  const context = canvas.getContext('2d')!
  const gradient = context.createRadialGradient(64, 64, 4, 64, 64, 64)
  gradient.addColorStop(0, 'rgba(255,224,140,.9)')
  gradient.addColorStop(.3, 'rgba(255,174,60,.42)')
  gradient.addColorStop(1, 'rgba(255,130,20,0)')
  context.fillStyle = gradient
  context.fillRect(0, 0, 128, 128)
  return new THREE.CanvasTexture(canvas)
}

function createSelectionTexture(): THREE.CanvasTexture {
  const canvas = document.createElement('canvas')
  canvas.width = 128
  canvas.height = 128
  const context = canvas.getContext('2d')!
  context.strokeStyle = 'rgba(115, 221, 211, .9)'
  context.lineWidth = 4
  context.beginPath()
  context.arc(64, 64, 50, 0, Math.PI * 2)
  context.stroke()
  return new THREE.CanvasTexture(canvas)
}

function createOrbitFlowMaterial(
  color: number,
  opacity: number,
  pointSize: number,
): THREE.ShaderMaterial {
  return new THREE.ShaderMaterial({
    uniforms: {
      flowColor: { value: new THREE.Color(color) },
      flowOpacity: { value: opacity },
      flowPointSize: { value: pointSize },
    },
    vertexShader: `
      uniform float flowPointSize;
      attribute float flowStrength;
      varying float vFlowStrength;
      void main() {
        vFlowStrength = flowStrength;
        vec4 viewPosition = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = flowPointSize * mix(0.28, 1.0, pow(flowStrength, 1.4));
        gl_Position = projectionMatrix * viewPosition;
      }
    `,
    fragmentShader: `
      uniform vec3 flowColor;
      uniform float flowOpacity;
      varying float vFlowStrength;
      void main() {
        float radius = length(gl_PointCoord - vec2(0.5));
        float softDisc = 1.0 - smoothstep(0.12, 0.5, radius);
        float alpha = softDisc * pow(vFlowStrength, 2.1) * flowOpacity;
        if (alpha < 0.018) discard;
        gl_FragColor = vec4(flowColor, alpha);
      }
    `,
    transparent: true,
    depthTest: false,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  })
}

function createNightLightsMaterial(nightMap: THREE.Texture, sunPosition: THREE.Vector3): THREE.ShaderMaterial {
  return new THREE.ShaderMaterial({
    uniforms: {
      nightMap: { value: nightMap },
      sunPosition: { value: sunPosition },
    },
    vertexShader: `
      varying vec2 vUv;
      varying vec3 vWorldNormal;
      varying vec3 vWorldPosition;
      void main() {
        vUv = uv;
        vec4 worldPosition = modelMatrix * vec4(position, 1.0);
        vWorldPosition = worldPosition.xyz;
        vWorldNormal = normalize(mat3(modelMatrix) * normal);
        gl_Position = projectionMatrix * viewMatrix * worldPosition;
      }
    `,
    fragmentShader: `
      uniform sampler2D nightMap;
      uniform vec3 sunPosition;
      varying vec2 vUv;
      varying vec3 vWorldNormal;
      varying vec3 vWorldPosition;
      void main() {
        vec3 toSun = normalize(sunPosition - vWorldPosition);
        float solarFacing = dot(normalize(vWorldNormal), toSun);
        float night = 1.0 - smoothstep(-0.18, 0.05, solarFacing);
        vec3 lights = max(texture2D(nightMap, vUv).rgb - vec3(0.012), vec3(0.0));
        gl_FragColor = vec4(lights * night * 12.0, night);
      }
    `,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  })
}

function disposeGroup(group: THREE.Group) {
  group.traverse((object) => {
    if (object instanceof CSS2DObject) object.element.remove()
    if (object instanceof THREE.Mesh || object instanceof THREE.Line || object instanceof THREE.Points) {
      object.geometry.dispose()
      const materials = Array.isArray(object.material) ? object.material : [object.material]
      materials.forEach((material) => material.dispose())
    }
    if (object instanceof THREE.Sprite) object.material.dispose()
  })
  group.clear()
}

export function SolarMap({
  epochTdbMicros,
  timeRate,
  selectedId,
  focusId,
  viewPreset,
  cameraAction,
  onSelect,
  onFocus,
}: Props) {
  const focusBody = bodyById.get(focusId) ?? bodyById.get('sun')!
  const solarDistanceAu = focusBody.id === 'sun'
    ? 0
    : Math.hypot(...heliocentricState(focusBody, epochTdbMicros).position_m) / AU
  const containerRef = useRef<HTMLDivElement>(null)
  const epochRef = useRef(epochTdbMicros)
  const timeRateRef = useRef(timeRate)
  const selectedRef = useRef(selectedId)
  const viewPresetValueRef = useRef(viewPreset)
  const onSelectRef = useRef(onSelect)
  const onFocusRef = useRef(onFocus)
  const rebuildRef = useRef<(focus: string) => void>(() => undefined)
  const cameraPresetRef = useRef<(preset: 'perspective' | 'top') => void>(() => undefined)
  const cameraActionRef = useRef<(type: CameraAction['type']) => void>(() => undefined)

  useEffect(() => { epochRef.current = epochTdbMicros }, [epochTdbMicros])
  useEffect(() => { timeRateRef.current = timeRate }, [timeRate])
  useEffect(() => { selectedRef.current = selectedId }, [selectedId])
  useEffect(() => { onSelectRef.current = onSelect }, [onSelect])
  useEffect(() => { onFocusRef.current = onFocus }, [onFocus])
  useEffect(() => { rebuildRef.current(focusId) }, [focusId])
  useEffect(() => {
    viewPresetValueRef.current = viewPreset
    cameraPresetRef.current(viewPreset)
  }, [viewPreset])
  useEffect(() => { cameraActionRef.current(cameraAction.type) }, [cameraAction])

  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const scene = new THREE.Scene()
    scene.background = new THREE.Color(0x050d10)
    const sceneFog = new THREE.FogExp2(0x050d10, 0.00125)
    scene.fog = sceneFog

    const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 2_500)
    camera.position.set(0, 118, 228)
    const renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: false,
      powerPreference: 'high-performance',
      logarithmicDepthBuffer: true,
    })
    renderer.outputColorSpace = THREE.SRGBColorSpace
    renderer.toneMapping = THREE.ACESFilmicToneMapping
    renderer.toneMappingExposure = 1.08
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5))
    renderer.domElement.className = 'three-canvas'
    container.appendChild(renderer.domElement)

    const labelRenderer = new CSS2DRenderer()
    labelRenderer.domElement.className = 'three-label-layer'
    container.appendChild(labelRenderer.domElement)

    const controls = new OrbitControls(camera, renderer.domElement)
    controls.enableDamping = true
    controls.dampingFactor = 0.075
    controls.rotateSpeed = 0.55
    controls.zoomSpeed = 0.82
    controls.panSpeed = 0.5
    controls.minDistance = 0.1
    controls.maxDistance = 520
    controls.target.set(0, 0, 0)

    scene.add(new THREE.AmbientLight(0x71808a, 0.22))
    const sunlight = new THREE.PointLight(0xfff1d1, 9.5, 0, 0)
    scene.add(sunlight)

    const scopeGroup = new THREE.Group()
    scene.add(scopeGroup)
    const textureLoader = new THREE.TextureLoader()
    const maxAnisotropy = renderer.capabilities.getMaxAnisotropy()
    const loadedTextures: THREE.Texture[] = []
    const starfieldTexture = textureLoader.load(starfieldUrl)
    starfieldTexture.colorSpace = THREE.SRGBColorSpace
    starfieldTexture.mapping = THREE.EquirectangularReflectionMapping
    starfieldTexture.anisotropy = Math.min(maxAnisotropy, 8)
    scene.background = starfieldTexture
    scene.backgroundIntensity = 0.68
    loadedTextures.push(starfieldTexture)
    const planetTextures = new Map<string, THREE.Texture>()
    for (const [bodyId, url] of Object.entries(textureUrls)) {
      const texture = textureLoader.load(url)
      texture.colorSpace = THREE.SRGBColorSpace
      texture.anisotropy = Math.min(maxAnisotropy, 8)
      planetTextures.set(bodyId, texture)
      loadedTextures.push(texture)
    }
    const saturnRingTexture = textureLoader.load(saturnRingUrl)
    saturnRingTexture.colorSpace = THREE.SRGBColorSpace
    saturnRingTexture.anisotropy = Math.min(maxAnisotropy, 8)
    loadedTextures.push(saturnRingTexture)
    const earthCloudTexture = textureLoader.load(earthCloudUrl)
    earthCloudTexture.anisotropy = Math.min(maxAnisotropy, 8)
    loadedTextures.push(earthCloudTexture)
    const earthNightTexture = textureLoader.load(earthNightUrl)
    earthNightTexture.colorSpace = THREE.SRGBColorSpace
    earthNightTexture.anisotropy = Math.min(maxAnisotropy, 8)
    loadedTextures.push(earthNightTexture)
    const glowTexture = createGlowTexture()
    loadedTextures.push(glowTexture)
    const selectionTexture = createSelectionTexture()
    loadedTextures.push(selectionTexture)
    const sunWorldPosition = new THREE.Vector3()
    const cloudPhase = Math.random() * Math.PI * 2
    let visuals: BodyVisual[] = []
    let orbitVisuals: OrbitVisual[] = []
    let scope: ScopeModel = {
      focus: bodyById.get('sun')!,
      maxRadius: 70 * AU,
      global: true,
      bodies: [],
      contextExtent: 178,
      focusRadiusScene: 7,
    }
    let cameraGoal = camera.position.clone()
    const targetGoal = new THREE.Vector3()
    let cameraTransition = false
    let cameraFollowsOrbit = false

    const localContextBody = () => scope.focus.body_class === 'moon' && scope.focus.parent_id
      ? bodyById.get(scope.focus.parent_id)
      : bodyById.get('sun')

    const visualFocusOffset = () => {
      if (scope.global || viewPresetValueRef.current === 'top') return new THREE.Vector3()
      const contextBody = localContextBody()
      if (!contextBody) return new THREE.Vector3()
      const contextPosition = displayPosition(
        relativePosition(contextBody, scope.focus, epochRef.current),
        scope.maxRadius,
        false,
      )
      const distance = contextPosition.length()
      if (distance === 0) return contextPosition
      const offsetDistance = Math.min(
        distance * 0.42,
        scope.focusRadiusScene * VISUAL_FOCUS_OFFSET_RADII,
      )
      return contextPosition.multiplyScalar(offsetDistance / distance)
    }

    const orbitCameraDirection = () => {
      const contextBody = localContextBody()
      if (!contextBody) return new THREE.Vector3(0.52, 0.46, 0.78).normalize()
      const contextDirection = displayPosition(
        relativePosition(contextBody, scope.focus, epochRef.current),
        scope.maxRadius,
        false,
      ).normalize()
      contextDirection.multiplyScalar(-1)
      contextDirection.y += 0.12
      return contextDirection.normalize()
    }

    const isRemoteContextBody = (body: Body) => !scope.global && (
      body.id === 'sun'
      || (scope.focus.body_class === 'moon' && body.id === scope.focus.parent_id)
    )

    const setCameraPreset = (preset: 'perspective' | 'top') => {
      const fittedDistance = THREE.MathUtils.clamp(
        scope.focusRadiusScene * FOCUS_CAMERA_RADII,
        0.01,
        120,
      )
      const distance = scope.global
        ? 248
        : fittedDistance
      if (preset === 'top') {
        cameraGoal = new THREE.Vector3(0.01, distance, 0.01)
      } else if (!scope.global) {
        cameraGoal = orbitCameraDirection().multiplyScalar(distance)
      } else {
        cameraGoal = new THREE.Vector3(distance * 0.52, distance * 0.46, distance * 0.78)
      }
      targetGoal.set(0, 0, 0)
      camera.near = scope.global ? 0.1 : Math.max(0.000001, distance / 200_000)
      camera.far = Math.max(2_500, scope.contextExtent * 12, distance * 12)
      camera.updateProjectionMatrix()
      controls.minDistance = scope.global ? 9 : Math.max(scope.focusRadiusScene * 2.4, 0.0001)
      controls.maxDistance = Math.max(520, scope.contextExtent * 6)
      camera.position.copy(cameraGoal)
      controls.target.copy(targetGoal)
      cameraTransition = false
      cameraFollowsOrbit = !scope.global && preset === 'perspective'
      controls.update()
      const showRemoteContext = preset === 'perspective'
      for (const visual of visuals) {
        const visible = showRemoteContext || !isRemoteContextBody(visual.body)
        visual.root.visible = visible
        visual.labelObject.visible = visible
      }
    }

    const buildOrbit = (body: Body, anchorBodyId?: string) => {
      if (!body.ephemeris) return
      const points: THREE.Vector3[] = []
      const samples = 192
      for (let index = 0; index <= samples; index += 1) {
        const sampleEpoch = body.ephemeris.epoch_tdb_micros
          + body.ephemeris.orbital_period_s * 1e6 * index / samples
        points.push(displayPosition(localState(body, sampleEpoch).position_m, scope.maxRadius, scope.global))
      }
      const geometry = new THREE.BufferGeometry().setFromPoints(points)
      const material = new THREE.LineBasicMaterial({
        color: body.id === scope.focus.id
          ? 0x62502e
          : body.body_class === 'planet' ? 0x41666c : 0x31494e,
        transparent: true,
        opacity: body.id === scope.focus.id ? 0.34 : scope.global ? 0.28 : 0.4,
      })
      const line = new THREE.Line(geometry, material)
      const orbitRoot = new THREE.Group()
      orbitRoot.add(line)
      const flowCurve = new THREE.CatmullRomCurve3(points.slice(0, -1), true, 'centripetal')
      const isFocusedOrbit = body.id === scope.focus.id
      const flowPointCount = isFocusedOrbit ? 26 : 12
      const flowGeometry = new THREE.BufferGeometry()
      flowGeometry.setAttribute(
        'position',
        new THREE.BufferAttribute(new Float32Array(flowPointCount * 3), 3),
      )
      flowGeometry.setAttribute(
        'flowStrength',
        new THREE.BufferAttribute(
          Float32Array.from(
            { length: flowPointCount },
            (_, index) => (index + 1) / flowPointCount,
          ),
          1,
        ),
      )
      const flowMaterial = createOrbitFlowMaterial(
        isFocusedOrbit ? 0xffc96b : 0x64d4ca,
        isFocusedOrbit ? 0.92 : 0.5,
        isFocusedOrbit ? 7.2 : 4.2,
      )
      const flowTrail = new THREE.Points(flowGeometry, flowMaterial)
      // The point positions move around the full orbit every frame, so their
      // initial zero-sized bounds must not be used for frustum culling.
      flowTrail.frustumCulled = false
      flowTrail.renderOrder = 2
      orbitRoot.add(flowTrail)
      let orbitLength = 0
      for (let index = 1; index < points.length; index += 1) {
        orbitLength += points[index].distanceTo(points[index - 1])
      }
      const desiredTrailLength = scope.focusRadiusScene * (isFocusedOrbit ? 10 : 6)
      const flowTrailLength = THREE.MathUtils.clamp(
        desiredTrailLength / Math.max(orbitLength, 0.0001),
        0.00004,
        scope.global ? 0.018 : 0.03,
      )
      const phaseLead = THREE.MathUtils.clamp(
        scope.focusRadiusScene * (isFocusedOrbit ? 4 : 2.5) / Math.max(orbitLength, 0.0001),
        0.00002,
        scope.global ? 0.008 : 0.015,
      )
      scopeGroup.add(orbitRoot)
      orbitVisuals.push({
        root: orbitRoot,
        body,
        flowGeometry,
        flowCurve,
        flowTrailLength,
        phaseLead,
        anchorBodyId,
      })
    }

    const buildScope = (nextFocusId: string) => {
      disposeGroup(scopeGroup)
      visuals = []
      orbitVisuals = []
      const focus = bodyById.get(nextFocusId) ?? bodyById.get('sun')!
      const children = childrenOf(focus.id)
      const global = focus.id === 'sun'
      const focusOrbitRadius = focus.body_class === 'moon' && focus.ephemeris
        ? focus.ephemeris.semi_major_axis_m * (1 + focus.ephemeris.eccentricity)
        : 0
      const minimumLocalRadius = focus.mean_radius_m * 40
      const sun = bodyById.get('sun')!
      const parent = focus.parent_id ? bodyById.get(focus.parent_id) : undefined
      const localBodies = [sun, parent, focus, ...children]
        .filter((body): body is Body => Boolean(body))
        .filter((body, index, bodies) => bodies.findIndex((candidate) => candidate.id === body.id) === index)
      const referenceRadius = Math.max(
        ...children.map((body) => body.ephemeris
          ? body.ephemeris.semi_major_axis_m * (1 + body.ephemeris.eccentricity)
          : 1),
        global ? 70 * AU : focusOrbitRadius,
        global ? 1 : minimumLocalRadius,
      )
      const bodies = global ? [focus, ...children] : localBodies
      const physicalBodyExtent = global ? 0 : Math.max(...bodies.map((body) => (
        Math.hypot(...relativePosition(body, focus, epochRef.current)) + body.mean_radius_m
      )))
      const physicalParentDistance = !global && parent
        ? Math.hypot(...relativePosition(parent, focus, epochRef.current))
        : 0
      const physicalFocusOrbitExtent = !global && focus.ephemeris
        ? physicalParentDistance + focus.ephemeris.semi_major_axis_m * (1 + focus.ephemeris.eccentricity)
        : 0
      // One linear scale is shared by every local distance and radius. Only the
      // overall normalization is capped so very small KBOs do not create 1e9-unit scenes.
      const safeContextReference = Math.max(physicalBodyExtent, physicalFocusOrbitExtent)
        * LOCAL_REFERENCE_RADIUS / LOCAL_MAX_CONTEXT_EXTENT
      const maxRadius = global ? referenceRadius : Math.max(referenceRadius, safeContextReference)
      const localScale = LOCAL_REFERENCE_RADIUS / maxRadius
      const focusRadiusScene = global ? bodyRadius(focus, true, true) : focus.mean_radius_m * localScale
      const bodyExtent = global ? 178 : Math.max(...bodies.map((body) => {
        const distance = Math.hypot(...relativePosition(body, focus, epochRef.current)) * localScale
        return distance + body.mean_radius_m * localScale
      }))
      const parentDistance = !global && focus.parent_id
        ? Math.hypot(...relativePosition(bodyById.get(focus.parent_id)!, focus, epochRef.current)) * localScale
        : 0
      const focusOrbitExtent = !global && focus.ephemeris
        ? parentDistance + focus.ephemeris.semi_major_axis_m * (1 + focus.ephemeris.eccentricity) * localScale
        : 0
      scope = {
        focus,
        maxRadius,
        global,
        bodies,
        contextExtent: Math.max(bodyExtent, focusOrbitExtent, LOCAL_REFERENCE_RADIUS),
        focusRadiusScene,
      }
      sceneFog.density = global ? 0.00125 : 0

      if (!global && focus.ephemeris && focus.parent_id) buildOrbit(focus, focus.parent_id)
      children.forEach((body) => buildOrbit(body))

      for (const body of scope.bodies) {
        const isFocus = body.id === focus.id
        const radius = bodyRadius(body, isFocus, global)
        const surfaceTexture = planetTextures.get(body.id) ?? null
        const material = new THREE.MeshStandardMaterial({
          color: surfaceTexture ? 0xffffff : bodyColor(body),
          map: surfaceTexture,
          roughness: body.body_class === 'star' ? 0.42 : 0.78,
          metalness: body.body_class === 'asteroid' ? 0.18 : 0.03,
          emissive: body.body_class === 'star' ? 0xffffff : 0x000000,
          emissiveMap: body.body_class === 'star' ? surfaceTexture : null,
          emissiveIntensity: body.body_class === 'star' ? 3.4 : 0,
        })
        const mesh = new THREE.Mesh(new THREE.SphereGeometry(radius, 40, 28), material)
        mesh.userData.bodyId = body.id
        const root = new THREE.Group()
        const axisGroup = new THREE.Group()
        axisGroup.rotation.z = THREE.MathUtils.degToRad(axialTiltDegrees[body.id] ?? 0)
        root.add(axisGroup)
        axisGroup.add(mesh)
        scopeGroup.add(root)

        let cloudMesh: BodyVisual['cloudMesh']
        let nightMesh: BodyVisual['nightMesh']
        if (body.id === 'earth') {
          nightMesh = new THREE.Mesh(
            new THREE.SphereGeometry(radius * 1.006, 48, 32),
            createNightLightsMaterial(earthNightTexture, sunWorldPosition),
          )
          axisGroup.add(nightMesh)
          cloudMesh = new THREE.Mesh(
            new THREE.SphereGeometry(radius * 1.018, 48, 32),
            new THREE.MeshStandardMaterial({
              color: 0xf4fbff,
              alphaMap: earthCloudTexture,
              transparent: true,
              opacity: 0.68,
              roughness: 0.96,
              metalness: 0,
              depthWrite: false,
            }),
          )
          axisGroup.add(cloudMesh)
        }

        if (body.id === 'saturn') {
          const ring = new THREE.Mesh(
            new THREE.RingGeometry(radius * 1.35, radius * 2.25, 96),
            new THREE.MeshStandardMaterial({
              color: 0xc7b17a,
              map: saturnRingTexture,
              alphaMap: saturnRingTexture,
              side: THREE.DoubleSide,
              transparent: true,
              opacity: 0.82,
              roughness: 0.85,
            }),
          )
          ring.rotation.x = Math.PI / 2
          axisGroup.add(ring)
        }

        let glow: THREE.Sprite | undefined
        if (body.id === 'sun') {
          glow = new THREE.Sprite(new THREE.SpriteMaterial({
            map: glowTexture,
            color: 0xffb044,
            transparent: true,
            depthWrite: false,
            blending: THREE.AdditiveBlending,
          }))
          const appearance = sunAppearance(
            Math.hypot(...relativePosition(sun, focus, epochRef.current)) / AU,
            global,
          )
          glow.scale.set(appearance.glowSize, appearance.glowSize, 1)
          root.add(glow)
        }

        const label = document.createElement('div')
        label.className = 'three-label'
        label.innerHTML = `<b>${body.localized_name_zh}</b><span>${body.canonical_name}</span>`
        const labelObject = new CSS2DObject(label)
        scopeGroup.add(labelObject)

        const selectionHalo = new THREE.Sprite(new THREE.SpriteMaterial({
          map: selectionTexture,
          color: 0x73ddd3,
          transparent: true,
          depthWrite: false,
        }))
        selectionHalo.scale.set(radius * 3.1, radius * 3.1, 1)
        selectionHalo.visible = body.id === selectedRef.current
        root.add(selectionHalo)
        visuals.push({
          body,
          root,
          axisGroup,
          mesh,
          material,
          baseRadius: radius,
          glow,
          cloudMesh,
          nightMesh,
          labelObject,
          label,
          selectionHalo,
        })
      }
      setCameraPreset(viewPresetValueRef.current)
    }

    rebuildRef.current = buildScope
    cameraPresetRef.current = setCameraPreset
    cameraActionRef.current = (type) => {
      if (type === 'reset') {
        setCameraPreset(viewPresetValueRef.current)
        return
      }
      const direction = camera.position.clone().sub(controls.target)
      direction.multiplyScalar(type === 'zoom-in' ? 0.68 : 1.46)
      cameraGoal = controls.target.clone().add(direction)
      targetGoal.copy(controls.target)
      cameraTransition = true
    }

    const raycaster = new THREE.Raycaster()
    const pointer = new THREE.Vector2()
    let pointerDown = { x: 0, y: 0 }
    const pickedBody = (event: PointerEvent | MouseEvent): string | null => {
      const bounds = renderer.domElement.getBoundingClientRect()
      pointer.x = ((event.clientX - bounds.left) / bounds.width) * 2 - 1
      pointer.y = -((event.clientY - bounds.top) / bounds.height) * 2 + 1
      raycaster.setFromCamera(pointer, camera)
      const hit = raycaster.intersectObjects(visuals.map((visual) => visual.mesh), false)[0]
      return hit?.object.userData.bodyId as string | null ?? null
    }
    const handlePointerDown = (event: PointerEvent) => {
      pointerDown = { x: event.clientX, y: event.clientY }
    }
    const handlePointerUp = (event: PointerEvent) => {
      if (Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) > 5) return
      const id = pickedBody(event)
      if (id) onSelectRef.current(id)
    }
    const handleDoubleClick = (event: MouseEvent) => {
      const id = pickedBody(event)
      if (id) onFocusRef.current(id)
    }
    renderer.domElement.addEventListener('pointerdown', handlePointerDown)
    renderer.domElement.addEventListener('pointerup', handlePointerUp)
    renderer.domElement.addEventListener('dblclick', handleDoubleClick)
    controls.addEventListener('start', () => {
      cameraTransition = false
      cameraFollowsOrbit = false
    })

    const resize = () => {
      const width = Math.max(container.clientWidth, 1)
      const height = Math.max(container.clientHeight, 1)
      camera.aspect = width / height
      camera.updateProjectionMatrix()
      renderer.setSize(width, height, false)
      labelRenderer.setSize(width, height)
    }
    const resizeObserver = new ResizeObserver(resize)
    resizeObserver.observe(container)
    resize()
    buildScope(focusId)

    let animationFrame = 0
    let previousFrame = performance.now()
    let performanceWindowStart = previousFrame
    let performanceFrames = 0
    const performanceBadge = container.querySelector<HTMLElement>('.fps-meter')
    const animate = (now: number) => {
      const deltaSeconds = Math.min((now - previousFrame) / 1000, 0.1)
      previousFrame = now
      if (timeRateRef.current > 0) {
        epochRef.current += deltaSeconds * timeRateRef.current * 1e6
      }

      const frameOffset = visualFocusOffset()
      for (const orbit of orbitVisuals) {
        const anchor = orbit.anchorBodyId ? bodyById.get(orbit.anchorBodyId) : undefined
        const anchorPosition = anchor
          ? displayPosition(
            relativePosition(anchor, scope.focus, epochRef.current),
            scope.maxRadius,
            scope.global,
          )
          : new THREE.Vector3()
        orbit.root.position.copy(anchorPosition.sub(frameOffset))
        if (orbit.body.ephemeris) {
          const elapsedPeriods = (epochRef.current - orbit.body.ephemeris.epoch_tdb_micros)
            / (orbit.body.ephemeris.orbital_period_s * 1e6)
          const phase = ((elapsedPeriods % 1) + 1) % 1
          const positions = orbit.flowGeometry.getAttribute('position') as THREE.BufferAttribute
          const lastPoint = Math.max(positions.count - 1, 1)
          for (let index = 0; index < positions.count; index += 1) {
            const progress = index / lastPoint
            const samplePhase = (
              phase + orbit.phaseLead - orbit.flowTrailLength * (1 - progress) + 1
            ) % 1
            const point = orbit.flowCurve.getPoint(samplePhase)
            positions.setXYZ(index, point.x, point.y, point.z)
          }
          positions.needsUpdate = true
        }
      }

      for (const visual of visuals) {
        const relative = relativePosition(visual.body, scope.focus, epochRef.current)
        const nextPosition = displayPosition(relative, scope.maxRadius, scope.global).sub(frameOffset)
        visual.root.position.copy(nextPosition)
        visual.labelObject.position.copy(nextPosition)
        const physicalRadius = visual.body.mean_radius_m * LOCAL_REFERENCE_RADIUS / scope.maxRadius
        const renderedRadius = !scope.global && visual.body.id === 'sun'
          ? physicalRadius * LOCAL_SUN_RADIUS_BOOST
          : physicalRadius
        const radiusScale = scope.global ? 1 : renderedRadius / visual.baseRadius
        visual.axisGroup.scale.setScalar(radiusScale)
        const haloRadius = (scope.global ? visual.baseRadius : renderedRadius) * 3.1
        visual.selectionHalo.scale.set(haloRadius, haloRadius, 1)
        if (visual.body.id === 'sun') {
          sunWorldPosition.copy(nextPosition)
          sunlight.position.copy(nextPosition)
          const appearance = sunAppearance(Math.hypot(...relative) / AU, scope.global)
          visual.material.emissiveIntensity = appearance.emissiveIntensity
          sunlight.intensity = appearance.lightIntensity
          const glowSize = scope.global
            ? appearance.glowSize
            : renderedRadius * LOCAL_SUN_GLOW_MULTIPLIER
          visual.glow?.scale.set(glowSize, glowSize, 1)
        }
        if (visual.body.rotation_period_s) {
          const rotation = (epochRef.current / 1e6 / visual.body.rotation_period_s * Math.PI * 2) % (Math.PI * 2)
          visual.mesh.rotation.y = rotation
          if (visual.nightMesh) visual.nightMesh.rotation.y = rotation
          if (visual.cloudMesh) {
            visual.cloudMesh.rotation.y = (
              epochRef.current / 1e6 / (visual.body.rotation_period_s * 0.985) * Math.PI * 2 + cloudPhase
            ) % (Math.PI * 2)
          }
        }
        const selected = visual.body.id === selectedRef.current
        visual.selectionHalo.visible = selected
        visual.material.emissive.setHex(visual.body.body_class === 'star' ? 0xffffff : 0x000000)
        visual.label.classList.toggle('selected', selected)
        visual.label.classList.toggle('minor', scope.global
          && visual.body.body_class !== 'planet'
          && visual.body.body_class !== 'star'
          && !selected)
      }

      if (cameraTransition) {
        camera.position.lerp(cameraGoal, 0.075)
        controls.target.lerp(targetGoal, 0.075)
        if (camera.position.distanceTo(cameraGoal) < 0.12) cameraTransition = false
      } else if (cameraFollowsOrbit) {
        const cameraDistance = camera.position.distanceTo(controls.target)
        cameraGoal = orbitCameraDirection().multiplyScalar(cameraDistance)
        camera.position.lerp(cameraGoal, 0.1)
        controls.target.lerp(targetGoal, 0.1)
      }
      controls.update()
      renderer.render(scene, camera)
      labelRenderer.render(scene, camera)
      performanceFrames += 1
      if (performanceBadge && now - performanceWindowStart >= 750) {
        const fps = Math.round(performanceFrames * 1_000 / (now - performanceWindowStart))
        performanceBadge.textContent = `${fps} FPS`
        performanceBadge.dataset.fps = String(fps)
        performanceFrames = 0
        performanceWindowStart = now
      }
      animationFrame = requestAnimationFrame(animate)
    }
    animationFrame = requestAnimationFrame(animate)

    return () => {
      cancelAnimationFrame(animationFrame)
      resizeObserver.disconnect()
      renderer.domElement.removeEventListener('pointerdown', handlePointerDown)
      renderer.domElement.removeEventListener('pointerup', handlePointerUp)
      renderer.domElement.removeEventListener('dblclick', handleDoubleClick)
      controls.dispose()
      disposeGroup(scopeGroup)
      renderer.dispose()
      loadedTextures.forEach((texture) => texture.dispose())
      renderer.domElement.remove()
      labelRenderer.domElement.remove()
    }
    // Renderer lifetime is intentionally stable; changing props updates refs and rebuild callbacks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div
      ref={containerRef}
      className="map-wrap three-map"
      role="application"
      aria-label="交互式太阳系 3D 轨道地图；滚轮缩放，拖拽旋转，双击天体聚焦"
    >
      <div className="map-caption">
        <span className="status-dot" /> Three.js WebGL · J2000 黄道参考系
        <strong>太阳系总览使用视觉压缩；行星与卫星视图采用统一物理比例尺</strong>
        <small className="texture-credit">
          表面 <a href="https://www.solarsystemscope.com/textures/" target="_blank" rel="noreferrer">Solar System Scope / INOVE</a>
          {' · '}星空 <a href="https://svs.gsfc.nasa.gov/4851" target="_blank" rel="noreferrer">NASA SVS / J2000</a>
        </small>
      </div>
      <div className="map-help">滚轮缩放 · 左键旋转 · 右键平移 · 3D 透视显示远场母天体 · 双击聚焦</div>
      {focusId !== 'sun' && viewPreset === 'perspective' && (
        <div className="sun-distance-badge">☀ 太阳 · 远场光源 · {solarDistanceAu.toFixed(3)} AU</div>
      )}
      <div className="fps-meter" aria-label="当前 WebGL 帧率">-- FPS</div>
    </div>
  )
}
