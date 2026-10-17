// Baut, was die Desktop-App neben sich braucht: das `beton`-Binary (Sidecar, DESK-002) und die
// Web-UI (apps/web/dist). `release` legt das Binary zusätzlich unter
// src-tauri/binaries/beton-<target-triple> ab, wo `tauri build` es ins Bundle packt
// (tauri.bundle.conf.json). Aufruf: node scripts/prepare-sidecar.mjs debug|release
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const profile = process.argv[2] === 'release' ? 'release' : 'debug'
const root = join(dirname(fileURLToPath(import.meta.url)), '../../..')
const run = (cmd, args, cwd = root) => execFileSync(cmd, args, { cwd, stdio: 'inherit' })

run('cargo', ['build', '-p', 'beton-cli', ...(profile === 'release' ? ['--release'] : [])])
run('pnpm', ['install', '--frozen-lockfile'], join(root, 'apps/web'))
run('pnpm', ['build'], join(root, 'apps/web'))

if (profile === 'release') {
  const host = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1]
  if (!host) throw new Error('Ziel-Triple von rustc nicht lesbar')
  const exe = process.platform === 'win32' ? '.exe' : ''
  const target = join(root, 'apps/desktop/src-tauri/binaries', `beton-${host}${exe}`)
  mkdirSync(dirname(target), { recursive: true })
  copyFileSync(join(root, 'target/release', `beton${exe}`), target)
  console.log(`Sidecar: ${target}`)
}
