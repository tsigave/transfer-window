import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { SolarMap, type CameraAction } from './SolarMap'
import { bodyById, catalog, childrenOf, dateFromEpoch, epochFromDate, heliocentricState, searchBodies, type Body } from './model'
import { loadSnapshot, saveSnapshot } from './persistence'
import { queryTransferPlans, type PlanTransferResult, type PlannerProgress } from './planner'
import { queryBodyState } from './runtime'
import './styles.css'

const initialDate = new Date('2160-01-01T00:00:00.000Z')
const AU = 149_597_870_700
const yearMicros = 365.25 * 86_400 * 1e6
const formatNumber = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 3 })
const playbackRates = [1, 10_000, 1_000_000, 100_000_000]
const planetScaleStorageKey = 'transfer-window.planet-visibility-scale'
const legacyPlanetScaleStorageKey = 'solarstorm.planet-visibility-scale'

function initialPlanetScale(): number {
  if (typeof window === 'undefined') return 1.8
  try {
    const currentValue = window.localStorage.getItem(planetScaleStorageKey)
    const rawValue = currentValue ?? window.localStorage.getItem(legacyPlanetScaleStorageKey)
    if (rawValue === null) return 1.8
    if (currentValue === null) {
      window.localStorage.setItem(planetScaleStorageKey, rawValue)
      window.localStorage.removeItem(legacyPlanetScaleStorageKey)
    }
    const stored = Number(rawValue)
    return Number.isFinite(stored) ? Math.min(6, Math.max(1, stored)) : 1.8
  } catch {
    return 1.8
  }
}

function shiftCalendarYears(epochTdbMicros: number, years: number): number {
  const date = dateFromEpoch(epochTdbMicros)
  date.setUTCFullYear(date.getUTCFullYear() + years)
  return epochFromDate(date)
}

function scopeForBody(body: Body): string {
  return body.id
}

function TreeBranch({ body, selectedId, onSelect }: { body: Body; selectedId: string; onSelect: (id: string) => void }) {
  const children = childrenOf(body.id)
  return (
    <li>
      <button className={selectedId === body.id ? 'tree-item selected' : 'tree-item'} onClick={() => onSelect(body.id)}>
        <span className={`body-icon ${body.body_class}`} />
        <span>{body.localized_name_zh}<small>{body.canonical_name}</small></span>
        {children.length > 0 && <em>{children.length}</em>}
      </button>
      {children.length > 0 && <ul>{children.map((child) => <TreeBranch key={child.id} body={child} selectedId={selectedId} onSelect={onSelect} />)}</ul>}
    </li>
  )
}

export default function App() {
  const [selectedId, setSelectedId] = useState('earth')
  const [focusId, setFocusId] = useState('sun')
  const [query, setQuery] = useState('')
  const [epochTdbMicros, setEpochTdbMicros] = useState(() => epochFromDate(initialDate))
  const [timeRate, setTimeRate] = useState(0)
  const [viewPreset, setViewPreset] = useState<'perspective' | 'top'>('perspective')
  const [planetVisibilityScale, setPlanetVisibilityScale] = useState(initialPlanetScale)
  const [cameraAction, setCameraAction] = useState<CameraAction>({ id: 0, type: 'reset' })
  const [notice, setNotice] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [plannerOpen, setPlannerOpen] = useState(false)
  const [plannerOriginId, setPlannerOriginId] = useState('earth')
  const [plannerDestinationId, setPlannerDestinationId] = useState('moon')
  const [plannerPayloadKg, setPlannerPayloadKg] = useState(1_000)
  const [plannerMinimumDays, setPlannerMinimumDays] = useState(3)
  const [plannerMaximumDays, setPlannerMaximumDays] = useState(40)
  const [plannerProgress, setPlannerProgress] = useState<PlannerProgress | null>(null)
  const [plannerResult, setPlannerResult] = useState<PlanTransferResult | null>(null)
  const [plannerSolutionId, setPlannerSolutionId] = useState<string | null>(null)
  const [plannerError, setPlannerError] = useState<string | null>(null)
  const plannerAbort = useRef<AbortController | null>(null)
  const selected = bodyById.get(selectedId) ?? bodyById.get('earth')!
  const results = useMemo(() => searchBodies(query), [query])
  const previewState = useMemo(() => heliocentricState(selected, epochTdbMicros), [selected, epochTdbMicros])
  const [pausedState, setPausedState] = useState(previewState)
  const state = timeRate === 0 ? pausedState : previewState
  const displayedDate = dateFromEpoch(epochTdbMicros)
  const plannerSolution = plannerResult?.report.solutions.find((solution) => solution.id === plannerSolutionId) ?? null
  const plannerFrontier = useMemo(() => {
    if (!plannerResult) return []
    return plannerResult.report.solutions.filter((solution) => plannerResult.paretoSolutionIds.includes(solution.id))
  }, [plannerResult])
  const heatCells = useMemo(() => {
    if (!plannerResult) return []
    const cells = [
      ...plannerResult.report.solutions.map((solution) => ({
        key: solution.id,
        departure: solution.departure,
        duration: solution.time_of_flight_s,
        solution,
        failure: null,
      })),
      ...plannerResult.report.failures.map((failure, index) => ({
        key: `failure-${index}`,
        departure: failure.departure ?? 0,
        duration: failure.duration_s ?? 0,
        solution: null,
        failure,
      })),
    ]
    return cells.sort((left, right) => left.departure - right.departure || left.duration - right.duration)
  }, [plannerResult])

  useEffect(() => {
    try {
      window.localStorage.setItem(planetScaleStorageKey, String(planetVisibilityScale))
    } catch {
      // The setting still works for this session when storage is unavailable.
    }
  }, [planetVisibilityScale])

  useEffect(() => {
    if (timeRate !== 0) return
    let current = true
    setPausedState(previewState)
    queryBodyState(selected.id, epochTdbMicros)
      .then((nextState) => { if (current) setPausedState(nextState) })
      .catch((reason: unknown) => { if (current) setError(reason instanceof Error ? reason.message : String(reason)) })
    return () => { current = false }
  }, [selected.id, epochTdbMicros, previewState, timeRate])

  useEffect(() => {
    if (timeRate === 0) return
    let animationFrame = 0
    let previousFrame = performance.now()
    let previousCommit = previousFrame
    let pendingMicros = 0
    const advance = (now: number) => {
      const elapsedMilliseconds = Math.min(now - previousFrame, 100)
      previousFrame = now
      // Playback rates are true real-time multipliers: ×10,000 = 10,000 simulated seconds/second.
      pendingMicros += elapsedMilliseconds * timeRate * 1_000
      if (now - previousCommit >= 33) {
        const committedMicros = pendingMicros
        pendingMicros = 0
        previousCommit = now
        setEpochTdbMicros((value) => value + committedMicros)
      }
      animationFrame = requestAnimationFrame(advance)
    }
    animationFrame = requestAnimationFrame(advance)
    return () => window.cancelAnimationFrame(animationFrame)
  }, [timeRate])

  function locate(id: string) {
    const body = bodyById.get(id)
    if (!body) return
    setSelectedId(id)
    setFocusId(scopeForBody(body))
    setQuery('')
  }

  function focusBody(id: string) {
    const body = bodyById.get(id)
    if (!body) return
    setSelectedId(id)
    setFocusId(scopeForBody(body))
  }

  function goUpOneLevel() {
    const focus = bodyById.get(focusId)
    setFocusId(focus?.parent_id ?? 'sun')
  }

  function moveCamera(type: CameraAction['type']) {
    setCameraAction((current) => ({ id: current.id + 1, type }))
  }

  function jumpTime(update: (value: number) => number) {
    setTimeRate(0)
    setEpochTdbMicros(update)
  }

  async function save() {
    setError(null)
    try {
      await saveSnapshot({ schemaVersion: 1, contentVersion: catalog.content_version, epochTdbMicros, selectedBodyId: selectedId })
      setNotice('存档已完整写入 alpha-v0.1 槽位')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  async function load() {
    setError(null)
    try {
      const snapshot = await loadSnapshot()
      if (snapshot.contentVersion !== catalog.content_version || !bodyById.has(snapshot.selectedBodyId)) {
        throw new Error('SAVE_UNSUPPORTED: 存档内容版本或所选天体不可用，世界未被重置。')
      }
      setEpochTdbMicros(snapshot.epochTdbMicros)
      locate(snapshot.selectedBodyId)
      setNotice('存档已载入；时间、选择与事实状态一致')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  function openPlanner() {
    const destination = selectedId === 'earth' ? 'moon' : selectedId
    setPlannerDestinationId(destination)
    setPlannerOpen(true)
  }

  async function startPlanning() {
    plannerAbort.current?.abort()
    const controller = new AbortController()
    plannerAbort.current = controller
    setPlannerResult(null)
    setPlannerSolutionId(null)
    setPlannerError(null)
    const requestId = `planner-${Date.now().toString(36)}`
    setPlannerProgress({ requestId, evaluated: 0, planned: 15, executableSolutions: 0, status: 'completed' })
    try {
      const result = await queryTransferPlans({
        requestId,
        originId: plannerOriginId,
        destinationId: plannerDestinationId,
        departureTdbMicros: epochTdbMicros,
        payloadMassKg: plannerPayloadKg,
        payloadVolumeM3: plannerPayloadKg / 180,
        minimumDurationDays: plannerMinimumDays,
        maximumDurationDays: plannerMaximumDays,
      }, setPlannerProgress, controller.signal)
      setPlannerResult(result)
      setPlannerSolutionId(result.representatives?.balanced ?? result.paretoSolutionIds[0] ?? null)
    } catch (reason) {
      setPlannerError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      if (plannerAbort.current === controller) plannerAbort.current = null
    }
  }

  function cancelPlanning() {
    plannerAbort.current?.abort()
  }

  function representativeLabel(id: string): string {
    const representatives = plannerResult?.representatives
    if (!representatives) return ''
    const labels = []
    if (id === representatives.fastest) labels.push('最快')
    if (id === representatives.balanced) labels.push('平衡')
    if (id === representatives.efficient) labels.push('节能')
    return labels.join(' / ')
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand"><span className="brand-mark">T</span><div><b>TRANSFER WINDOW</b><small>可达空间 · alpha v0.2</small></div></div>
        <div className="clock-block">
          <small>模拟时间 · TDB 内部纪元</small>
          <time>{displayedDate.toISOString().slice(0, 10)} <b>{displayedDate.toISOString().slice(11, 19)}</b></time>
        </div>
        <div className="save-actions">
          <button className="planner-button" onClick={openPlanner}>航迹规划</button>
          <button onClick={save}>保存</button><button onClick={load}>载入</button>
          <span className="content-version">内容 {catalog.content_version}</span>
        </div>
      </header>

      <aside className="catalog-panel">
        <div className="panel-heading"><span>天体目录</span><em>{catalog.bodies.length} 个独立天体</em></div>
        <label className="search-box"><span>⌕</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索名称、别名或稳定 ID" /></label>
        {query ? (
          <div className="search-results">
            <small>{results.length} 项匹配</small>
            {results.map((body) => <button key={body.id} onClick={() => locate(body.id)}><b>{body.localized_name_zh}</b><span>{body.canonical_name}</span><code>{body.id}</code></button>)}
          </div>
        ) : (
          <nav className="tree" aria-label="天体层级"><ul>{childrenOf(null).map((body) => <TreeBranch key={body.id} body={body} selectedId={selectedId} onSelect={locate} />)}</ul></nav>
        )}
        <div className="regions">
          <small>统计区域 · 非独立天体</small>
          {catalog.regions.map((region) => <div key={region.id}><span>⌁</span>{region.localized_name_zh}<em>区域</em></div>)}
        </div>
      </aside>

      <section className="map-panel">
        <div className="map-toolbar">
          <div className="toolbar-group scope-tools">
            <button className={focusId === 'sun' ? 'active' : ''} onClick={() => setFocusId('sun')}>☀ 聚焦太阳</button>
            <button onClick={() => focusBody(selected.id)}>◎ 聚焦所选天体</button>
            <button onClick={goUpOneLevel} disabled={focusId === 'sun'}>↰ 上一级</button>
            <span className="scope-readout">观察原点：{bodyById.get(focusId)?.localized_name_zh ?? '太阳'}</span>
          </div>
          <div className="toolbar-group camera-tools">
            <button className={viewPreset === 'perspective' ? 'active' : ''} onClick={() => setViewPreset('perspective')}>3D 透视</button>
            <button className={viewPreset === 'top' ? 'active' : ''} onClick={() => setViewPreset('top')}>黄道俯视</button>
            <button aria-label="放大地图" title="放大" onClick={() => moveCamera('zoom-in')}>＋</button>
            <button aria-label="缩小地图" title="缩小" onClick={() => moveCamera('zoom-out')}>−</button>
            <button aria-label="复位相机" title="复位相机" onClick={() => moveCamera('reset')}>↺</button>
          </div>
        </div>
        <label className="planet-scale-control" title="1× 保持真实比例；提高倍率可让太阳系总览中的行星更容易辨认">
          <span>行星可见性</span>
          <input
            aria-label="太阳系总览行星视觉倍率"
            type="range"
            min={1}
            max={6}
            step={0.1}
            value={planetVisibilityScale}
            onChange={(event) => setPlanetVisibilityScale(Number(event.target.value))}
          />
          <output>{planetVisibilityScale === 1 ? '拟真 1×' : `${planetVisibilityScale.toFixed(1)}×`}</output>
        </label>
        <SolarMap
          epochTdbMicros={epochTdbMicros}
          timeRate={timeRate}
          selectedId={selectedId}
          focusId={focusId}
          viewPreset={viewPreset}
          cameraAction={cameraAction}
          overviewBodyScaleCap={planetVisibilityScale}
          onSelect={setSelectedId}
          onFocus={focusBody}
        />
        <div className="time-control">
          <div className="rate-buttons" title="相对真实时间的连续播放倍率"><button className={timeRate === 0 ? 'active' : ''} onClick={() => setTimeRate(0)}>Ⅱ</button>{playbackRates.map((rate) => <button key={rate} className={timeRate === rate ? 'active' : ''} onClick={() => setTimeRate(rate)}>×{formatNumber.format(rate)}</button>)}</div>
          <button onClick={() => jumpTime((value) => shiftCalendarYears(value, -1))}>− 1 年</button>
          <input aria-label="时间轴" type="range" min={-10} max={10} step={0.1} value={(epochTdbMicros - epochFromDate(initialDate)) / yearMicros} onChange={(event) => { setTimeRate(0); setEpochTdbMicros(epochFromDate(initialDate) + Number(event.target.value) * yearMicros) }} />
          <button onClick={() => jumpTime((value) => shiftCalendarYears(value, 1))}>+ 1 年</button>
          <button className="decade" onClick={() => jumpTime((value) => shiftCalendarYears(value, 10))}>推进十年</button>
        </div>
      </section>

      <aside className="detail-panel">
        <div className="eyebrow">{selected.body_class.replaceAll('_', ' ')} · {selected.id}</div>
        <h1>{selected.localized_name_zh}<small>{selected.canonical_name}</small></h1>
        <div className="badges"><span>● 已观测</span><span className={selected.data_quality}>质量：{selected.data_quality === 'reference' ? '参考级' : '近似级'}</span></div>
        <section><h2>层级与权限</h2><dl><dt>父天体</dt><dd>{selected.parent_id ? `${bodyById.get(selected.parent_id)?.localized_name_zh} · ${selected.parent_id}` : '无 · 系统根'}</dd><dt>开发状态</dt><dd>Observed · 仅观测</dd><dt>目录可见性</dt><dd>不受开发权限影响</dd></dl></section>
        <section><h2>真实物理参数 <i>SI</i></h2><dl><dt>质量</dt><dd>{selected.mass_kg.toExponential(5)} kg</dd><dt>平均半径</dt><dd>{formatNumber.format(selected.mean_radius_m / 1000)} km</dd><dt>自转周期</dt><dd>{selected.rotation_period_s ? `${formatNumber.format(selected.rotation_period_s / 86_400)} d` : '—'}</dd>{selected.ephemeris && <><dt>轨道半长轴</dt><dd>{formatNumber.format(selected.ephemeris.semi_major_axis_m / AU)} AU</dd></>}</dl></section>
        <section><h2>星历状态 <i>{displayedDate.toISOString().slice(0, 10)}</i></h2><dl><dt>参考系</dt><dd>日心黄道 J2000</dd><dt>位置 X / Y / Z</dt><dd className="vector">{state.position_m.map((value) => `${(value / AU).toFixed(6)}`).join(' / ')} AU</dd><dt>速度</dt><dd>{formatNumber.format(Math.hypot(...state.velocity_mps) / 1000)} km/s</dd></dl></section>
        <section className="source"><h2>数据来源</h2><a href={selected.ephemeris_source.url} target="_blank" rel="noreferrer">{selected.ephemeris_source.name} ↗</a><p>{selected.ephemeris_source.kind === 'public_reference' ? '公开参考参数 · 可追溯' : '公开参数的解析近似 · 非执行级'}</p></section>
      </aside>

      {plannerOpen && (
        <section className="planner-drawer" aria-label="航迹规划">
          <header className="planner-header">
            <div><small>ALPHA V0.2 · TRANSFER PLANNER</small><h2>可达空间航迹规划</h2></div>
            <div className="planner-header-actions">
              <span>{plannerProgress ? `${plannerProgress.evaluated} / ${plannerProgress.planned} · ${plannerProgress.executableSolutions} 个可执行` : '等待计算'}</span>
              {plannerAbort.current && <button onClick={cancelPlanning}>取消计算</button>}
              <button aria-label="关闭航迹规划" onClick={() => { cancelPlanning(); setPlannerOpen(false) }}>×</button>
            </div>
          </header>

          <div className="planner-controls">
            <label>始发天体<select value={plannerOriginId} onChange={(event) => setPlannerOriginId(event.target.value)}>{catalog.bodies.map((body) => <option key={body.id} value={body.id}>{body.localized_name_zh} · {body.id}</option>)}</select></label>
            <label>目标天体<select value={plannerDestinationId} onChange={(event) => setPlannerDestinationId(event.target.value)}>{catalog.bodies.map((body) => <option key={body.id} value={body.id}>{body.localized_name_zh} · {body.id}</option>)}</select></label>
            <label>载荷 kg<input type="number" min={0} max={120000} value={plannerPayloadKg} onChange={(event) => setPlannerPayloadKg(Number(event.target.value))} /></label>
            <label>最短天数<input type="number" min={1} value={plannerMinimumDays} onChange={(event) => setPlannerMinimumDays(Number(event.target.value))} /></label>
            <label>最长天数<input type="number" min={plannerMinimumDays} value={plannerMaximumDays} onChange={(event) => setPlannerMaximumDays(Number(event.target.value))} /></label>
            <button className="calculate" disabled={Boolean(plannerAbort.current)} onClick={() => void startPlanning()}>计算 3 × 5 窗口</button>
          </div>

          <div className="planner-progress" aria-label="航迹计算进度">
            <span style={{ width: `${plannerProgress ? Math.min(100, plannerProgress.evaluated / plannerProgress.planned * 100) : 0}%` }} />
          </div>
          {plannerError && <div className="planner-error">{plannerError}</div>}

          <div className="planner-content">
            <section className="planner-summary">
              <h3>约束与目标服务</h3>
              <dl><dt>舰船</dt><dd>Lunar Courier · rev 1</dd><dt>舱容</dt><dd>120,000 kg / 650 m³</dd><dt>抵达条件</dt><dd>Rendezvous · 执行级复核</dd><dt>储备策略</dt><dd>显式零储备 · 不隐含补给</dd></dl>
              <div className="service-warning"><b>{bodyById.get(plannerDestinationId)?.localized_name_zh}</b><span>无市场 · 无补给 · 无维修</span><small>开发权限不阻止规划，但不会创建虚构服务。</small></div>
              {plannerResult && <p className="solver-status">状态 {plannerResult.report.termination_reason} · 已保留 {plannerResult.report.solutions.length} 个结果 / {plannerResult.report.failures.length} 个不可行原因</p>}
            </section>

            <section className="planner-heatmap">
              <h3>出发日—航程热图 <small>颜色越亮，工质越低</small></h3>
              <div className="heatmap-grid">
                {heatCells.map((cell) => {
                  const intensity = cell.solution ? Math.max(.12, 1 - cell.solution.propellant_consumed_kg / 300000) : 0
                  return <button key={cell.key} disabled={!cell.solution} className={cell.solution?.id === plannerSolutionId ? 'selected' : ''} style={{ '--heat': intensity } as CSSProperties} title={cell.solution ? `${dateFromEpoch(cell.departure).toISOString().slice(0, 10)} · ${(cell.duration / 86400).toFixed(1)} 天 · ${cell.solution.propellant_consumed_kg.toFixed(0)} kg` : cell.failure?.message} onClick={() => cell.solution && setPlannerSolutionId(cell.solution.id)}><span>{dateFromEpoch(cell.departure).toISOString().slice(5, 10)}</span><b>{(cell.duration / 86400).toFixed(0)}d</b></button>
                })}
                {!heatCells.length && <div className="empty-state">设置约束后开始计算；进行中会持续报告已评估数量和部分可执行结果。</div>}
              </div>
            </section>

            <section className="planner-table-wrap">
              <h3>Pareto 前沿 <small>{plannerFrontier.length} 个非支配方案</small></h3>
              <table className="planner-table"><thead><tr><th>代表</th><th>出发</th><th>航程</th><th>工质</th><th>载荷</th><th>寿命</th><th>估算成本</th></tr></thead><tbody>
                {plannerFrontier.map((solution) => <tr key={solution.id} className={solution.id === plannerSolutionId ? 'selected' : ''} onClick={() => setPlannerSolutionId(solution.id)}><td><b>{representativeLabel(solution.id) || '前沿'}</b></td><td>{dateFromEpoch(solution.departure).toISOString().slice(0, 10)}</td><td>{(solution.time_of_flight_s / 86400).toFixed(1)} d</td><td>{formatNumber.format(solution.propellant_consumed_kg)} kg</td><td>{formatNumber.format(solution.payload_mass_kg)} kg</td><td>{(solution.engine_lifetime_used_s / 3600).toFixed(1)} h</td><td>{formatNumber.format(solution.estimated_cost_credits)}</td></tr>)}
              </tbody></table>
            </section>

            <section className="planner-detail">
              <h3>执行级展开 <small>{plannerSolution?.id ?? '未选择方案'}</small></h3>
              {plannerSolution ? <>
                <div className="engineering-cards"><div><small>质量预算</small><b>{formatNumber.format(plannerSolution.propellant_consumed_kg)} kg 工质</b><span>{plannerSolution.fusion_fuel_consumed_kg.toFixed(3)} kg 聚变燃料</span></div><div><small>功率 / 热峰值</small><b>{(plannerSolution.peak_power_w / 1e9).toFixed(3)} GW</b><span>{(plannerSolution.peak_waste_heat_w / 1e6).toFixed(1)} MW 废热</span></div><div><small>复核误差余量</small><b>{formatNumber.format(plannerSolution.margins.position_error_m)} m</b><span>{plannerSolution.margins.velocity_error_mps.toFixed(4)} m/s</span></div></div>
                <ol className="segment-list">{plannerSolution.segments.map((segment, index) => <li key={`${segment.kind}-${index}`}><b>{segment.kind.replaceAll('_', ' ')}</b><span>{segment.phase ?? (segment.kind === 'coast' ? '日心滑行' : '抵达复核')}</span><em>{segment.powered_duration_s ? `${(segment.powered_duration_s / 3600).toFixed(1)} h · ${segment.chunk_count} 段` : segment.planned_position_error_m ? `${segment.planned_position_error_m.toFixed(1)} m` : '状态矢量已记录'}</em></li>)}</ol>
                <p className="solver-meta">输入 {plannerSolution.metadata.input_hash.slice(0, 16)}… · {plannerSolution.metadata.solver_version} · Lambert {plannerSolution.metadata.lambert_iterations} 次 · 积分 {plannerSolution.metadata.integrator_accepted_steps} 步 · 容差 {formatNumber.format(plannerSolution.metadata.position_tolerance_m)} m / {plannerSolution.metadata.velocity_tolerance_mps} m/s</p>
              </> : <div className="empty-state">从热图或 Pareto 表中选择一个可执行方案查看推力段、状态矢量、质量、功率、热峰值和误差余量。</div>}
            </section>
          </div>
        </section>
      )}

      {(notice || error) && <div className={error ? 'toast error' : 'toast'} role="status"><button onClick={() => { setNotice(null); setError(null) }}>×</button><b>{error ? '存档操作失败' : '完成'}</b><span>{error ?? notice}</span></div>}
    </main>
  )
}
