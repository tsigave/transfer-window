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
  mesh: THREE.Mesh<THREE.SphereGeometry, THREE.MeshStandardMaterial>
  material: THREE.MeshStandardMaterial
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
  mars: new URL('../assets/textures/mars.jpg', import.meta.url).href,
  jupiter: new URL('../assets/textures/jupiter.jpg', import.meta.url).href,
  saturn: new URL('../assets/textures/saturn.jpg', import.meta.url).href,
  uranus: new URL('../assets/textures/uranus.jpg', import.meta.url).href,
  neptune: new URL('../assets/textures/neptune.jpg', import.meta.url).href,
}

const saturnRingUrl = new URL('../assets/textures/saturn_ring.png', import.meta.url).href
const earthCloudUrl = new URL('../assets/textures/earth_clouds.jpg', import.meta.url).href
const earthNightUrl = new URL('../assets/textures/earth_nightmap.jpg', import.meta.url).href

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
  const normalized = global
    ? Math.log1p(actualRadius / (0.08 * AU)) / Math.log1p(maxRadius / (0.08 * AU))
    : Math.pow(actualRadius / maxRadius, 0.62)
  const displayRadius = (global ? 178 : 72) * normalized
  const multiplier = displayRadius / actualRadius
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

function createStarField(): THREE.Points {
  const points: number[] = []
  let seed = 0x2160
  const random = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0
    return seed / 0xffffffff
  }
  for (let index = 0; index < 1_200; index += 1) {
    const radius = 280 + random() * 360
    const theta = random() * Math.PI * 2
    const phi = Math.acos(2 * random() - 1)
    points.push(
      radius * Math.sin(phi) * Math.cos(theta),
      radius * Math.cos(phi),
      radius * Math.sin(phi) * Math.sin(theta),
    )
  }
  const geometry = new THREE.BufferGeometry()
  geometry.setAttribute('position', new THREE.Float32BufferAttribute(points, 3))
  return new THREE.Points(
    geometry,
    new THREE.PointsMaterial({ color: 0xa9c6ca, size: 0.75, transparent: true, opacity: 0.58 }),
  )
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
    scene.fog = new THREE.FogExp2(0x050d10, 0.00125)

    const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 2_500)
    camera.position.set(0, 118, 228)
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' })
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
    controls.minDistance = 9
    controls.maxDistance = 520
    controls.target.set(0, 0, 0)

    scene.add(new THREE.AmbientLight(0x71808a, 0.22))
    const sunlight = new THREE.PointLight(0xfff1d1, 9.5, 0, 0)
    scene.add(sunlight)
    scene.add(createStarField())

    const scopeGroup = new THREE.Group()
    scene.add(scopeGroup)
    const textureLoader = new THREE.TextureLoader()
    const maxAnisotropy = renderer.capabilities.getMaxAnisotropy()
    const loadedTextures: THREE.Texture[] = []
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
    let scope: ScopeModel = {
      focus: bodyById.get('sun')!,
      maxRadius: 70 * AU,
      global: true,
      bodies: [],
    }
    let cameraGoal = camera.position.clone()
    const targetGoal = new THREE.Vector3()
    let cameraTransition = false

    const setCameraPreset = (preset: 'perspective' | 'top') => {
      const distance = scope.global ? 248 : 220
      cameraGoal = preset === 'top'
        ? new THREE.Vector3(0.01, distance, 0.01)
        : new THREE.Vector3(distance * 0.52, distance * 0.46, distance * 0.78)
      targetGoal.set(0, 0, 0)
      cameraTransition = true
    }

    const buildOrbit = (body: Body) => {
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
        color: body.body_class === 'planet' ? 0x41666c : 0x31494e,
        transparent: true,
        opacity: scope.global ? 0.34 : 0.48,
      })
      scopeGroup.add(new THREE.Line(geometry, material))
    }

    const buildScope = (nextFocusId: string) => {
      disposeGroup(scopeGroup)
      visuals = []
      const focus = bodyById.get(nextFocusId) ?? bodyById.get('sun')!
      const children = childrenOf(focus.id)
      const global = focus.id === 'sun'
      const maxRadius = Math.max(
        ...children.map((body) => body.ephemeris
          ? body.ephemeris.semi_major_axis_m * (1 + body.ephemeris.eccentricity)
          : 1),
        global ? 70 * AU : 1,
      )
      const sun = bodyById.get('sun')!
      scope = { focus, maxRadius, global, bodies: global ? [focus, ...children] : [sun, focus, ...children] }

      children.forEach(buildOrbit)

      for (const body of scope.bodies) {
        const isFocus = body.id === focus.id
        const radius = bodyRadius(body, isFocus, global)
        const surfaceTexture = planetTextures.get(body.id) ?? null
        const material = new THREE.MeshStandardMaterial({
          color: surfaceTexture ? 0xffffff : bodyColor(body),
          map: surfaceTexture,
          roughness: body.body_class === 'star' ? 0.42 : 0.78,
          metalness: body.body_class === 'asteroid' ? 0.18 : 0.03,
          emissive: body.body_class === 'star' ? 0xff8a20 : 0x000000,
          emissiveIntensity: body.body_class === 'star' ? 2.15 : 0,
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

        if (body.id === 'sun') {
          const glow = new THREE.Sprite(new THREE.SpriteMaterial({
            map: glowTexture,
            color: 0xffb044,
            transparent: true,
            depthWrite: false,
            blending: THREE.AdditiveBlending,
          }))
          const glowSize = global ? 30 : 22
          glow.scale.set(glowSize, glowSize, 1)
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
        visuals.push({ body, root, mesh, material, cloudMesh, nightMesh, labelObject, label, selectionHalo })
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
    controls.addEventListener('start', () => { cameraTransition = false })

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

      for (const visual of visuals) {
        const relative = relativePosition(visual.body, scope.focus, epochRef.current)
        const nextPosition = !scope.global && visual.body.id === 'sun'
          ? displayPosition(relative, Math.hypot(...relative), false).normalize().multiplyScalar(900)
          : displayPosition(relative, scope.maxRadius, scope.global)
        visual.root.position.copy(nextPosition)
        visual.labelObject.position.copy(nextPosition)
        if (visual.body.id === 'sun') {
          sunWorldPosition.copy(nextPosition)
          sunlight.position.copy(nextPosition)
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
        visual.mesh.scale.setScalar(1)
        visual.selectionHalo.visible = selected
        visual.material.emissive.setHex(visual.body.body_class === 'star' ? 0xff8a20 : 0x000000)
        visual.material.emissiveIntensity = visual.body.body_class === 'star' ? 2.15 : 0
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
      scene.traverse((object) => {
        if (object instanceof THREE.Points) {
          object.geometry.dispose()
          object.material.dispose()
        }
      })
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
        <strong>轨道距离与天体半径均为视觉放大；权威 SI 状态保持不变</strong>
        <small className="texture-credit">
          材质 <a href="https://www.solarsystemscope.com/textures/" target="_blank" rel="noreferrer">Solar System Scope / INOVE</a>
          {' · '}<a href="https://creativecommons.org/licenses/by/4.0/" target="_blank" rel="noreferrer">CC BY 4.0</a>
        </small>
      </div>
      <div className="map-help">滚轮缩放 · 左键旋转 · 右键平移 · 双击进入层级</div>
      {focusId !== 'sun' && (
        <div className="sun-distance-badge">☀ 太阳 · 远场光源 · {solarDistanceAu.toFixed(3)} AU</div>
      )}
      <div className="fps-meter" aria-label="当前 WebGL 帧率">-- FPS</div>
    </div>
  )
}
