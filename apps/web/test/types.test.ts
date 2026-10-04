// @vitest-environment node
import { execFileSync } from 'node:child_process'
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const web = join(import.meta.dirname, '..')
const sdk = join(web, '..', '..', 'packages', 'sdk-ts', 'src')
const tsc = join(web, 'node_modules', '.bin', 'tsc')

/** Typprüfung der Web-UI gegen eine (ggf. veränderte) Kopie der generierten SDK-Typen. */
const cleanup: string[] = []

function typecheck(mutate?: (genDir: string) => void): { ok: boolean; output: string } {
  const dir = mkdtempSync(join(tmpdir(), 'beton-web-types-'))
  try {
    cpSync(sdk, join(dir, 'sdk'), { recursive: true })
    mutate?.(join(dir, 'sdk', 'gen'))
    const base = JSON.parse(readFileSync(join(web, 'tsconfig.app.json'), 'utf8')) as { compilerOptions: Record<string, unknown> }
    // Die Konfiguration liegt unter apps/web, damit Typen wie `vite/client` gefunden werden.
    const tmpConfigDir = join(web, 'node_modules', '.tmp')
    mkdirSync(tmpConfigDir, { recursive: true })
    const configPath = join(tmpConfigDir, `tsconfig.types-${process.pid}-${Date.now()}.json`)
    const config = {
      compilerOptions: {
        ...base.compilerOptions,
        tsBuildInfoFile: undefined,
        paths: { '@/*': [join(web, 'src', '*')], '@beton/sdk': [join(dir, 'sdk', 'index.ts')] },
      },
      include: [join(web, 'src')],
    }
    writeFileSync(configPath, JSON.stringify(config))
    cleanup.push(configPath)
    try {
      execFileSync(tsc, ['-p', configPath, '--noEmit'], { encoding: 'utf8', stdio: 'pipe' })
      return { ok: true, output: '' }
    } catch (e) {
      const err = e as { stdout?: string; stderr?: string }
      return { ok: false, output: `${err.stdout ?? ''}${err.stderr ?? ''}` }
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
    for (const f of cleanup.splice(0)) rmSync(f, { force: true })
  }
}

describe('Generierte Typen', () => {
  it('PROTO-013 AC2: tsc der Web-UI scheitert, wenn ein genutztes Feld im Rust-Typ fehlt', () => {
    expect(typecheck().ok).toBe(true)
    const broken = typecheck((gen) => {
      // Wie nach dem Entfernen von `SessionSummary::title` in Rust und `cargo xtask codegen`.
      const file = join(gen, 'SessionSummary.ts')
      const text = readFileSync(file, 'utf8')
      const without = text.replace(/title: string, /, '')
      expect(without).not.toBe(text)
      writeFileSync(file, without)
    })
    expect(broken.ok).toBe(false)
    expect(broken.output).toContain("'title'")
  }, 120_000)

  it('QA-006 AC3: dieselbe Prüfung deckt Event-Payloads ab (MessageCompleted.content)', () => {
    const broken = typecheck((gen) => {
      const file = join(gen, 'MessageCompleted.ts')
      const text = readFileSync(file, 'utf8')
      writeFileSync(file, text.replace(/content: Array<JsonValue>, /, ''))
    })
    expect(broken.ok).toBe(false)
    expect(broken.output).toContain("'content'")
  }, 120_000)
})
