import fs from 'node:fs'

const rootCargoPackages = ['sim-app', 'sim-astro', 'sim-engineering', 'sim-save', 'sim-time', 'sim-tools']
const tauriCargoPackages = ['sim-app', 'sim-astro', 'sim-save', 'sim-time', 'transfer-window-desktop']
const currentVersionDocs = ['README.md', 'docs/README.md']
const versionPattern = /^(?:v)?(\d+)\.(\d+)\.(\d+)$/

function fail(message) {
  console.error(`Version error: ${message}`)
  process.exit(1)
}

function normalizeVersion(value) {
  const match = versionPattern.exec(value)
  if (!match) fail(`invalid semantic version: ${value}`)
  return `${Number(match[1])}.${Number(match[2])}.${Number(match[3])}`
}

function read(path) {
  return fs.readFileSync(path, 'utf8')
}

function write(path, content) {
  fs.writeFileSync(path, content)
}

function replaceOne(path, pattern, replacement, description) {
  const content = read(path)
  const matches = [...content.matchAll(new RegExp(pattern.source, pattern.flags.includes('g') ? pattern.flags : `${pattern.flags}g`))]
  if (matches.length !== 1) {
    fail(`${path}: expected one ${description}, found ${matches.length}`)
  }
  write(path, content.replace(pattern, replacement))
}

function latestChangelogVersion() {
  const match = /^## \[v(\d+\.\d+\.\d+)\] - \d{4}-\d{2}-\d{2}$/m.exec(read('CHANGELOG.md'))
  if (!match) fail('CHANGELOG.md has no released version heading')
  return match[1]
}

function updateCargoLock(path, packageNames, version) {
  let content = read(path)
  const seen = new Set()
  content = content.replace(/\[\[package\]\]\n[\s\S]*?(?=\n\[\[package\]\]|\s*$)/g, (block) => {
    const name = /^name = "([^"]+)"$/m.exec(block)?.[1]
    if (!name || !packageNames.includes(name)) return block
    if (seen.has(name)) fail(`${path}: duplicate workspace package ${name}`)
    seen.add(name)
    if (!/^version = "[^"]+"$/m.test(block)) fail(`${path}: ${name} has no version`)
    return block.replace(/^version = "[^"]+"$/m, `version = "${version}"`)
  })
  const missing = packageNames.filter((name) => !seen.has(name))
  if (missing.length) fail(`${path}: missing workspace packages: ${missing.join(', ')}`)
  write(path, content)
}

function updateVersionDocs(version) {
  for (const path of currentVersionDocs) {
    replaceOne(
      path,
      /(<!-- transfer-window-current-version -->\n当前项目版本：`v)[0-9]+\.[0-9]+\.[0-9]+(`)/,
      `$1${version}$2`,
      'current-version marker',
    )
  }
}

function setVersion(version) {
  version = normalizeVersion(version)
  const changelogVersion = latestChangelogVersion()
  if (changelogVersion !== version) {
    fail(`CHANGELOG.md latest release is v${changelogVersion}, not v${version}`)
  }

  const packageJson = JSON.parse(read('package.json'))
  packageJson.version = version
  write('package.json', `${JSON.stringify(packageJson, null, 2)}\n`)

  const packageLock = JSON.parse(read('package-lock.json'))
  packageLock.version = version
  packageLock.packages[''].version = version
  write('package-lock.json', `${JSON.stringify(packageLock, null, 2)}\n`)

  replaceOne(
    'Cargo.toml',
    /(\[workspace\.package\][\s\S]*?^version\s*=\s*")[^"]+("$)/m,
    `$1${version}$2`,
    'workspace package version',
  )
  replaceOne(
    'src-tauri/Cargo.toml',
    /(\[package\][\s\S]*?^version\s*=\s*")[^"]+("$)/m,
    `$1${version}$2`,
    'desktop package version',
  )

  replaceOne(
    'src-tauri/tauri.conf.json',
    /("version"\s*:\s*")[^"]+(")/,
    `$1${version}$2`,
    'Tauri application version',
  )

  updateCargoLock('Cargo.lock', rootCargoPackages, version)
  updateCargoLock('src-tauri/Cargo.lock', tauriCargoPackages, version)
  updateVersionDocs(version)
  console.log(`Synchronized project version v${version}.`)
}

function localDate() {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: process.env.TZ || 'America/Los_Angeles',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).formatToParts(new Date())
  const value = Object.fromEntries(parts.map(({ type, value: part }) => [type, part]))
  return `${value.year}-${value.month}-${value.day}`
}

function bumpVersion(current, level) {
  const [major, minor, patch] = current.split('.').map(Number)
  if (level === 'major') return `${major + 1}.0.0`
  if (level === 'minor') return `${major}.${minor + 1}.0`
  if (level === 'patch') return `${major}.${minor}.${patch + 1}`
  fail(`bump must be major, minor, or patch; received ${level}`)
}

function releaseChangelog(version, level, forcePatch) {
  const content = read('CHANGELOG.md')
  const unreleasedPattern = /(## \[未发布\]\n\n)([\s\S]*?)(?=\n## \[v)/
  const match = unreleasedPattern.exec(content)
  if (!match) fail('CHANGELOG.md has no valid 未发布 section')

  let releaseBody = match[2].trim()
  const staging = /^### 小版本暂存区（(\d+)\/5）\n\n([\s\S]*)$/.exec(releaseBody)
  if (staging) {
    const declaredCount = Number(staging[1])
    const stagedContent = staging[2].trim()
    const actualCount = (stagedContent.match(/^- /gm) || []).length
    if (declaredCount !== actualCount) {
      fail(`CHANGELOG.md staging count says ${declaredCount}/5 but contains ${actualCount} entries`)
    }
    if (level === 'patch' && declaredCount < 5 && !forcePatch) {
      fail(`patch staging is ${declaredCount}/5; use --force only for an approved early release`)
    }
    if (!actualCount) fail('cannot release an empty changelog staging area')
    releaseBody = `### 维护\n\n${stagedContent}`
  }

  if (releaseBody === '暂无。' || !releaseBody) fail('cannot release an empty 未发布 section')
  const nextUnreleased = '## [未发布]\n\n### 小版本暂存区（0/5）\n\n暂无。\n'
  const release = `## [v${version}] - ${localDate()}\n\n${releaseBody}\n`
  write('CHANGELOG.md', content.replace(unreleasedPattern, `${nextUnreleased}\n${release}`))
}

function expectedVersions() {
  const packageJson = JSON.parse(read('package.json'))
  const packageLock = JSON.parse(read('package-lock.json'))
  const tauriConfig = JSON.parse(read('src-tauri/tauri.conf.json'))
  const values = [
    ['package.json', packageJson.version],
    ['package-lock.json', packageLock.version],
    ['package-lock.json root package', packageLock.packages[''].version],
    ['src-tauri/tauri.conf.json', tauriConfig.version],
    ['CHANGELOG.md latest release', latestChangelogVersion()],
  ]

  for (const [path, pattern] of [
    ['Cargo.toml', /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m],
    ['src-tauri/Cargo.toml', /\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m],
  ]) {
    const match = pattern.exec(read(path))
    if (!match) fail(`${path}: version not found`)
    values.push([path, match[1]])
  }

  for (const path of currentVersionDocs) {
    const match = /<!-- transfer-window-current-version -->\n当前项目版本：`v([0-9]+\.[0-9]+\.[0-9]+)`/.exec(read(path))
    if (!match) fail(`${path}: current-version marker not found`)
    values.push([path, match[1]])
  }
  return values
}

function checkChangelogStaging() {
  const match = /## \[未发布\]\n\n### 小版本暂存区（(\d+)\/5）\n\n([\s\S]*?)(?=\n## \[v)/.exec(read('CHANGELOG.md'))
  if (!match) fail('CHANGELOG.md has no valid small-version staging area')
  const declaredCount = Number(match[1])
  const actualCount = (match[2].match(/^- /gm) || []).length
  if (declaredCount !== actualCount) {
    fail(`CHANGELOG.md staging count says ${declaredCount}/5 but contains ${actualCount} entries`)
  }
  if (declaredCount > 5) fail(`CHANGELOG.md staging count ${declaredCount}/5 exceeds the release threshold`)
}

function checkCargoLock(path, packageNames, expected) {
  const versions = new Map()
  for (const block of read(path).match(/\[\[package\]\]\n[\s\S]*?(?=\n\[\[package\]\]|\s*$)/g) || []) {
    const name = /^name = "([^"]+)"$/m.exec(block)?.[1]
    if (!name || !packageNames.includes(name)) continue
    versions.set(name, /^version = "([^"]+)"$/m.exec(block)?.[1])
  }
  for (const name of packageNames) {
    if (versions.get(name) !== expected) {
      fail(`${path}: ${name} is ${versions.get(name) || 'missing'}, expected ${expected}`)
    }
  }
}

function check() {
  const versions = expectedVersions()
  const expected = normalizeVersion(versions[0][1])
  const mismatches = versions.filter(([, version]) => version !== expected)
  if (mismatches.length) {
    fail(`version mismatch; expected ${expected}: ${mismatches.map(([path, version]) => `${path}=${version}`).join(', ')}`)
  }
  checkCargoLock('Cargo.lock', rootCargoPackages, expected)
  checkCargoLock('src-tauri/Cargo.lock', tauriCargoPackages, expected)
  checkChangelogStaging()
  console.log(`All project versions are synchronized at v${expected}.`)
}

const [command, argument, ...flags] = process.argv.slice(2)
if (command === 'set') {
  if (!argument) fail('usage: version.mjs set X.Y.Z')
  setVersion(argument)
} else if (command === 'bump') {
  check()
  const current = JSON.parse(read('package.json')).version
  const next = bumpVersion(current, argument)
  releaseChangelog(next, argument, flags.includes('--force'))
  setVersion(next)
} else if (command === 'check') {
  check()
} else {
  fail('usage: version.mjs <set X.Y.Z | bump major|minor|patch [--force] | check>')
}
