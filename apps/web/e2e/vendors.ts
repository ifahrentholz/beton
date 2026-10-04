import { mkdtempSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fakeCli } from './fake-cli'

/** Schritt eines Szenarios: ein System-Tool von beton aufrufen (AGT-007, AGT-009). */
export const call = (tool: string, args: unknown) => ({ mcp_call: { server: 'beton', tool, args } })

/**
 * Fake-CLIs für Claude und Codex mit je einem Szenario (JSON ist gültiges YAML);
 * `select: by_input` bedient Parent und Childs eines Harness aus einer Datei.
 */
export function vendorEnv(claude: unknown[], codex: unknown[]): Record<string, string> {
  const dir = mkdtempSync(join(tmpdir(), 'beton-e2e-sub-'))
  const write = (name: string, turns: unknown[]) => {
    const p = join(dir, name)
    writeFileSync(p, JSON.stringify({ select: 'by_input', turns }))
    return p
  }
  // Ein Link namens `codex`, damit der Versions-Probe die Codex-Version liest.
  const link = join(dir, 'codex')
  symlinkSync(fakeCli(), link)
  return {
    BETON_CLAUDE_PATH: `${fakeCli()} --protocol stream-json --scenario ${write('claude.yaml', claude)}`,
    BETON_CODEX_PATH: `${link} --protocol app-server --scenario ${write('codex.yaml', codex)}`,
  }
}

