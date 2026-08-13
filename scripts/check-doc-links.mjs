import { readFileSync, existsSync, readdirSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'

const markdown = []
function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) walk(path)
    else if (entry.name.endsWith('.md')) markdown.push(path)
  }
}
walk('docs')
markdown.push('README.md')
const missing = []
for (const file of markdown) {
  const text = readFileSync(file, 'utf8')
  for (const match of text.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
    const link = decodeURIComponent(match[1].split('#')[0])
    if (!link || /^(https?:|mailto:)/.test(link)) continue
    const target = resolve(dirname(file), link)
    if (!existsSync(target)) missing.push(`${relative('.', file)} -> ${link}`)
  }
}
if (missing.length) {
  console.error(`Broken documentation links:\n${missing.join('\n')}`)
  process.exit(1)
}
console.log(`Documentation links OK (${markdown.length} files).`)

