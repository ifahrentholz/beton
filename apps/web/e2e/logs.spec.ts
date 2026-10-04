import { execFile } from 'node:child_process'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { promisify } from 'node:util'
import { BETON, expect, test } from './fixtures'

const run = promisify(execFile)

/** Alle Dateien im Log-Verzeichnis (JSON-Lines und rotierte Dateien). */
function logFiles(dir: string): string[] {
  return existsSync(dir) ? readdirSync(dir).map((f) => join(dir, f)) : []
}

test('OBS-001 AC1, OBS-001 AC4: Logs sind JSON mit Pflichtfeldern und enthalten Prompts nie ab info', async ({ daemon, page }) => {
  const marker = `MARKER${Date.now()}PROMPT`
  const scenario = daemon.scenario(
    'logs',
    [
      'turns:',
      `  - expect_input: "${marker} cli"`,
      '    emit:',
      `      - { message_delta: "Antwort mit ${marker}", chunk: 4 }`,
      `      - { tool_call: { name: Bash, kind: shell, args: { command: "echo ${marker}" } } }`,
      `      - { tool_result: { output: "${marker}" } }`,
      `  - expect_input: "${marker} web"`,
      '    emit:',
      `      - { message: "Zweite Antwort ${marker}" }`,
      '',
    ].join('\n'),
  )

  // Prompt über die CLI (cli.log, daemon.log, runner.log) …
  const { stderr } = await run(BETON, ['run', 'fake', '--scenario', scenario, '-p', `${marker} cli`], {
    env: daemon.env({ BROWSER: 'true' }),
    cwd: daemon.work,
  })
  const id = /\/s\/(ses_\w+)/.exec(stderr)?.[1]
  expect(id, stderr).toBeTruthy()

  // … und über die Web-UI.
  await daemon.login(page, `/s/${id}`)
  await page.getByLabel('Nachricht').fill(`${marker} web`)
  await page.getByLabel('Nachricht').press('Enter')
  await expect(page.getByText(`Zweite Antwort ${marker}`)).toBeVisible()

  // Geordnet beenden, damit die Writer alles geschrieben haben.
  await daemon.restart()

  const files = logFiles(daemon.logsDir())
  const daemonLog = join(daemon.logsDir(), 'daemon.log')
  expect(files).toContain(daemonLog)
  expect(files).toContain(join(daemon.logsDir(), 'runner.log'))
  expect(files).toContain(join(daemon.logsDir(), 'cli.log'))

  // AC1: jede Zeile in daemon.log ist JSON mit den Pflichtfeldern.
  const lines = readFileSync(daemonLog, 'utf8').split('\n').filter((l) => l.length > 0)
  expect(lines.length).toBeGreaterThan(0)
  for (const line of lines) {
    const entry = JSON.parse(line) as Record<string, unknown>
    for (const field of ['ts', 'level', 'target', 'component', 'version']) expect(entry, line).toHaveProperty(field)
    expect(entry.component).toBe('daemon')
  }

  // AC4: Der Marker steht in keinem Log auf info oder höher; Nicht-JSON-Zeilen gar nicht.
  const leaks: string[] = []
  for (const file of files) {
    for (const line of readFileSync(file, 'utf8').split('\n')) {
      if (!line.includes(marker)) continue
      let level = ''
      try {
        level = String((JSON.parse(line) as { level?: unknown }).level ?? '')
      } catch {
        // Nicht-JSON (z. B. serve.out) zählt immer.
      }
      if (!['DEBUG', 'TRACE', 'debug', 'trace'].includes(level)) leaks.push(`${file}: ${line.slice(0, 200)}`)
    }
  }
  expect(leaks).toEqual([])
})
