// @vitest-environment node
// VOI-008 AC1: Das Frontend nutzt keine Browser-Spracherkennung (Web Speech API). Die Regel
// steht in `.oxlintrc.json` (`pnpm lint` in der CI); dieser Test prüft, dass sie greift, und
// durchsucht zusätzlich alle Quellen, auch nach Zugriffen als Zeichenkette.
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const web = join(import.meta.dirname, '..')
const oxlint = join(web, 'node_modules', '.bin', 'oxlint')
const FORBIDDEN = /\b(webkit)?Speech(Recognition|GrammarList)\b/

function lint(source: string): { ok: boolean; output: string } {
  const dir = mkdtempSync(join(tmpdir(), 'beton-web-speech-'))
  try {
    const file = join(dir, 'probe.ts')
    writeFileSync(file, source)
    try {
      const output = execFileSync(oxlint, ['-c', join(web, '.oxlintrc.json'), file], { encoding: 'utf8' })
      return { ok: true, output }
    } catch (e) {
      const err = e as { stdout?: string; stderr?: string }
      return { ok: false, output: `${err.stdout ?? ''}${err.stderr ?? ''}` }
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) return sources(path)
    return /\.(ts|tsx|js|jsx|mjs|html)$/.test(name) ? [path] : []
  })
}

describe('VOI-008 AC1: keine Web Speech API', () => {
  it('VOI-008 AC1 Lint-Regel lehnt SpeechRecognition und webkitSpeechRecognition ab', () => {
    for (const source of [
      'export const r = new SpeechRecognition()\n',
      'export const r = new webkitSpeechRecognition()\n',
      'export const r = window.webkitSpeechRecognition\n',
      "export const r = (window as unknown as Record<string, unknown>)['SpeechRecognition']\n",
    ]) {
      const result = lint(source)
      expect(result.ok, source).toBe(false)
      expect(result.output).toMatch(/no-restricted-(globals|properties)/)
    }
    expect(lint('export const ok = navigator.mediaDevices\n').ok).toBe(true)
  })

  it('VOI-008 AC1 keine Quelldatei referenziert die Web Speech API', () => {
    const files = [...sources(join(web, 'src')), join(web, 'index.html')]
    expect(files.length).toBeGreaterThan(10)
    const hits = files.filter((f) => FORBIDDEN.test(readFileSync(f, 'utf8')))
    expect(hits).toEqual([])
  })
})
