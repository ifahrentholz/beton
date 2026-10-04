// Build-Check (WEB-001 AC4, WEB-015, WEB-009, WEB-012): keine absoluten http(s)-Asset-URLs im
// Bundle, Initial-JS (gzip) höchstens 450 KB, Monaco und xyflow nur in nachgeladenen Chunks.
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { gzipSync } from 'node:zlib'

const dist = join(import.meta.dirname, '..', 'dist')
const INITIAL_JS_BUDGET = 450 * 1024

function files(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n)
    return statSync(p).isDirectory() ? files(p) : [p]
  })
}

const html = readFileSync(join(dist, 'index.html'), 'utf8')
const problems = []
// Asset-Verweise im HTML und in CSS: src/href/url(...) mit http(s)://
const assetUrl = /(?:src|href)\s*=\s*["']https?:\/\/|url\(\s*["']?https?:\/\//gi
if (assetUrl.test(html)) problems.push('index.html verweist auf fremde Origins')
for (const f of files(dist)) {
  if (f.endsWith('.css') && /url\(\s*["']?https?:\/\//i.test(readFileSync(f, 'utf8'))) {
    problems.push(`${f}: url() auf fremden Origin`)
  }
  if (f.endsWith('.js')) {
    const js = readFileSync(f, 'utf8')
    // Dynamisch geladene Skripte/Stile von fremden Origins (import("https://…"), <link href="https://…">)
    if (/import\(\s*["']https?:\/\//.test(js)) problems.push(`${f}: import() von fremdem Origin`)
  }
}
// Initial-JS: die Skripte, die index.html direkt lädt, plus deren modulepreloads.
const initial = [...html.matchAll(/(?:src|href)="\/(assets\/[^"]+\.js)"/g)].map((m) => m[1])
let gz = 0
for (const rel of new Set(initial)) gz += gzipSync(readFileSync(join(dist, rel))).length
console.log(`Initial-JS (gzip): ${(gz / 1024).toFixed(1)} KB von ${INITIAL_JS_BUDGET / 1024} KB`)
if (gz > INITIAL_JS_BUDGET) problems.push(`Initial-JS ${(gz / 1024).toFixed(1)} KB über Budget`)
// Monaco ist nicht im initialen Bundle (WEB-009): Kennung des Editors nur in nachgeladenen Chunks.
const MONACO = /MonacoEnvironment|monaco-editor/
for (const rel of new Set(initial)) {
  if (MONACO.test(readFileSync(join(dist, rel), 'utf8'))) problems.push(`${rel}: Monaco im initialen Bundle`)
}
if (!files(dist).some((f) => f.endsWith('.js') && MONACO.test(readFileSync(f, 'utf8')))) problems.push('Monaco-Chunk fehlt im Build')
// xyflow (Sub-Agent-Graph, WEB-012) kommt erst mit dem Agents-Tab.
const XYFLOW = /react-flow__|xyflow/
for (const rel of new Set(initial)) {
  if (XYFLOW.test(readFileSync(join(dist, rel), 'utf8'))) problems.push(`${rel}: xyflow im initialen Bundle`)
}
if (!files(dist).some((f) => f.endsWith('.js') && XYFLOW.test(readFileSync(f, 'utf8')))) problems.push('xyflow-Chunk fehlt im Build')
if (problems.length) {
  console.error(problems.join('\n'))
  process.exit(1)
}
