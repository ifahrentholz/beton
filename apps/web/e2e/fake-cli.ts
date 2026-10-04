import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { BETON } from './fixtures'

/** Pfad der Fake-Vendor-CLI (QA-002) neben `beton`; baut sie bei Bedarf. */
export function fakeCli(): string {
  const path = join(dirname(BETON), process.platform === 'win32' ? 'beton-fake-cli.exe' : 'beton-fake-cli')
  if (!existsSync(path)) {
    execFileSync('cargo', ['build', '-q', '-p', 'beton-fake-cli'], { stdio: 'inherit', cwd: join(dirname(BETON), '..', '..') })
  }
  return path
}
