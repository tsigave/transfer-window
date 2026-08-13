import { useEffect, useMemo, useState } from 'react'
import { SolarMap, type CameraAction } from './SolarMap'
import { bodyById, catalog, childrenOf, dateFromEpoch, epochFromDate, heliocentricState, searchBodies, type Body } from './model'
import { loadSnapshot, saveSnapshot } from './persistence'
import { queryBodyState } from './runtime'
import './styles.css'

const initialDate = new Date('2160-01-01T00:00:00.000Z')
const AU = 149_597_870_700
const yearMicros = 365.25 * 86_400 * 1e6
const formatNumber = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 3 })

function shiftCalendarYears(epochTdbMicros: number, years: number): number {
  const date = dateFromEpoch(epochTdbMicros)
  date.setUTCFullYear(date.getUTCFullYear() + years)
  return epochFromDate(date)
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
  const [cameraAction, setCameraAction] = useState<CameraAction>({ id: 0, type: 'reset' })
  const [notice, setNotice] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const selected = bodyById.get(selectedId) ?? bodyById.get('earth')!
  const results = useMemo(() => searchBodies(query), [query])
  const previewState = useMemo(() => heliocentricState(selected, epochTdbMicros), [selected, epochTdbMicros])
  const [pausedState, setPausedState] = useState(previewState)
  const state = timeRate === 0 ? pausedState : previewState
  const displayedDate = dateFromEpoch(epochTdbMicros)

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
    setFocusId(childrenOf(body.id).length > 0 ? body.id : body.parent_id ?? 'sun')
    setQuery('')
  }

  function focusBody(id: string) {
    const body = bodyById.get(id)
    if (!body) return
    setSelectedId(id)
    setFocusId(childrenOf(body.id).length > 0 ? body.id : body.parent_id ?? 'sun')
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

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand"><span className="brand-mark">S</span><div><b>SOLARSTORM</b><small>太阳系事实层 · alpha v0.1</small></div></div>
        <div className="clock-block">
          <small>模拟时间 · TDB 内部纪元</small>
          <time>{displayedDate.toISOString().slice(0, 10)} <b>{displayedDate.toISOString().slice(11, 19)}</b></time>
        </div>
        <div className="save-actions">
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
          <nav className="tree" aria-label="天体层级"><ul>{childrenOf(null).map((body) => <TreeBranch key={body.id} body={body} selectedId={selectedId} onSelect={setSelectedId} />)}</ul></nav>
        )}
        <div className="regions">
          <small>统计区域 · 非独立天体</small>
          {catalog.regions.map((region) => <div key={region.id}><span>⌁</span>{region.localized_name_zh}<em>区域</em></div>)}
        </div>
      </aside>

      <section className="map-panel">
        <div className="map-toolbar">
          <div className="toolbar-group scope-tools">
            <button className={focusId === 'sun' ? 'active' : ''} onClick={() => setFocusId('sun')}>太阳系</button>
            <button onClick={() => focusBody(selected.id)}>◎ 聚焦所选层级</button>
            <button onClick={goUpOneLevel} disabled={focusId === 'sun'}>↰ 上一级</button>
            <span className="scope-readout">当前：{bodyById.get(focusId)?.localized_name_zh ?? '太阳'}系</span>
          </div>
          <div className="toolbar-group camera-tools">
            <button className={viewPreset === 'perspective' ? 'active' : ''} onClick={() => setViewPreset('perspective')}>3D 透视</button>
            <button className={viewPreset === 'top' ? 'active' : ''} onClick={() => setViewPreset('top')}>黄道俯视</button>
            <button aria-label="放大地图" title="放大" onClick={() => moveCamera('zoom-in')}>＋</button>
            <button aria-label="缩小地图" title="缩小" onClick={() => moveCamera('zoom-out')}>−</button>
            <button aria-label="复位相机" title="复位相机" onClick={() => moveCamera('reset')}>↺</button>
          </div>
        </div>
        <SolarMap
          epochTdbMicros={epochTdbMicros}
          timeRate={timeRate}
          selectedId={selectedId}
          focusId={focusId}
          viewPreset={viewPreset}
          cameraAction={cameraAction}
          onSelect={setSelectedId}
          onFocus={focusBody}
        />
        <div className="time-control">
          <div className="rate-buttons" title="相对真实时间的连续播放倍率"><button className={timeRate === 0 ? 'active' : ''} onClick={() => setTimeRate(0)}>Ⅱ</button>{[1, 100, 10_000].map((rate) => <button key={rate} className={timeRate === rate ? 'active' : ''} onClick={() => setTimeRate(rate)}>×{formatNumber.format(rate)}</button>)}</div>
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

      {(notice || error) && <div className={error ? 'toast error' : 'toast'} role="status"><button onClick={() => { setNotice(null); setError(null) }}>×</button><b>{error ? '存档操作失败' : '完成'}</b><span>{error ?? notice}</span></div>}
    </main>
  )
}
