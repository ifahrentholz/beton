import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const root = join(import.meta.dirname, '..')

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name)
    return statSync(p).isDirectory() ? files(p) : p.endsWith('.ts') ? [p] : []
  })
}

describe('Portabilität', () => {
  it('API-004 AC3: ESM-Paket ohne Node-only-Abhängigkeiten im Browser-Pfad', () => {
    const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')) as {
      type: string
      engines: { node: string }
      exports: Record<string, { import?: string; require?: string }>
      dependencies?: Record<string, string>
    }
    expect(pkg.type).toBe('module')
    expect(pkg.engines.node).toBe('>=20')
    expect(pkg.exports['.']?.import).toBe('./dist/index.js')
    expect(pkg.exports['.']?.require).toBeUndefined()
    expect(pkg.dependencies ?? {}).toEqual({})
    const forbidden = [/from ['"]node:/, /from ['"](fs|path|http|https|net|os|crypto|child_process|ws)['"]/, /\brequire\(/, /\bprocess\./, /\bBuffer\b/]
    for (const file of files(join(root, 'src'))) {
      const text = readFileSync(file, 'utf8')
      for (const re of forbidden) {
        expect(re.test(text), `${file}: ${re}`).toBe(false)
      }
    }
    // Typprüfung des Quellcodes läuft mit `lib: dom` und ohne Node-Typen (tsconfig.json).
    const tsconfig = JSON.parse(readFileSync(join(root, 'tsconfig.json'), 'utf8')) as {
      compilerOptions: { types: string[]; lib: string[] }
    }
    expect(tsconfig.compilerOptions.types).toEqual([])
    expect(tsconfig.compilerOptions.lib).toContain('dom')
  })
})
