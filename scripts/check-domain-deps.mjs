import { readFileSync } from 'node:fs'
import { globSync } from 'node:fs'

const forbidden = /(?:tauri|react|vite|wgpu|three)/i
const manifests = globSync('crates/*/Cargo.toml')
const violations = manifests.filter((path) => forbidden.test(readFileSync(path, 'utf8')))
if (violations.length) {
  console.error(`UI dependency found in core manifest: ${violations.join(', ')}`)
  process.exit(1)
}
console.log(`Domain dependency boundary OK (${manifests.length} manifests).`)

