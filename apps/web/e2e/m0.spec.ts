import { spawn } from 'node:child_process'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { BETON, Daemon, expect, freePort, test, attachLogs } from './fixtures'
import { fakeCli } from './fake-cli'

/** Startet `beton run …` und liefert die gedruckte Session-URL (stderr) und das Ende. */
function runAndUrl(daemon: Daemon, args: string[]) {
  const run = spawn(BETON, ['run', ...args], { env: daemon.env(), cwd: daemon.work })
  const exited = new Promise<number | null>((r) => run.on('exit', r))
  const url = new Promise<string>((resolve, reject) => {
    let err = ''
    run.stderr.on('data', (d: Buffer) => {
      err += String(d)
      const m = /(http:\/\/\S+\/s\/ses_\w+)/.exec(err)
      if (m) resolve(m[1]!)
    })
    void exited.then(() => reject(new Error(`keine URL in stderr: ${err}`)))
  })
  return { url, exited }
}

const words = (n: number) => Array.from({ length: n }, (_, i) => `w${i + 1}`).join(' ')

test('QA-007 AC1: beton run fake → gestreamte Antwort im Browser → Reload → vollständiger Verlauf @alle-engines', async ({ daemon, page }) => {
  const scenario = daemon.scenario(
    'm0',
    `turns:\n  - expect_input: "hallo"\n    emit:\n      - { message_delta: "Hallo aus dem Fake-Harness. Das ist die Antwort.", chunk: 4, chunk_delay_ms: 80 }\n`,
  )
  const { url, exited } = runAndUrl(daemon, ['fake', '--scenario', scenario, '-p', 'hallo'])
  await daemon.login(page, new URL(await url).pathname)
  const answer = page.getByTestId('agent-message').last()
  await expect(answer).toContainText('Hallo aus')
  await expect(answer).toContainText('Das ist die Antwort.')
  expect(await exited).toBe(0)
  await page.reload()
  await expect(page.getByText('hallo', { exact: true })).toBeVisible()
  await expect(page.getByTestId('agent-message')).toHaveCount(1)
  await expect(page.getByTestId('agent-message')).toContainText('Hallo aus dem Fake-Harness. Das ist die Antwort.')
})

test.describe('M0-Demo mit Claude-Adapter', () => {
  let daemon: Daemon
  test.beforeAll(async () => {
    // Eigener Daemon: `claude` ist die Fake-CLI (QA-002); fester Port für den Neustart.
    const port = await freePort()
    const scenario = join(mkdtempSync(join(tmpdir(), 'beton-claude-')), 'claude.yaml')
    writeFileSync(
      scenario,
      `turns:\n  - emit:\n      - { message_delta: "${words(120)}", chunk: 6, chunk_delay_ms: 40 }\n  - emit:\n      - { message: "Weiter nach dem Neustart." }\n`,
    )
    daemon = await Daemon.start({
      port,
      env: { BETON_CLAUDE_PATH: `${fakeCli()} --protocol stream-json --scenario ${scenario}` },
    })
  })
  test.afterAll(() => daemon.stop())
  // oxlint-disable-next-line no-empty-pattern -- Playwright verlangt hier das Objektmuster.
  test.afterEach(async ({}, testInfo) => attachLogs(daemon, testInfo))

  test('HAR-004 AC1, PROTO-009 AC3: beton run claude live im Browser, Netzabbruch, Resume, Daemon-Neustart', async ({ page, context }) => {
    const { url, exited } = runAndUrl(daemon, ['claude', '-p', 'Bitte zähle'])
    const path = new URL(await url).pathname
    const id = path.split('/').at(-1)!
    await daemon.login(page, path)
    const answer = page.getByTestId('agent-message').last()
    // Inkrementell: erst ein Teil, noch nicht alles.
    await expect(answer).toContainText('w1 ')
    expect(await answer.textContent()).not.toContain('w120')
    // Netzabbruch mitten im Strom, danach Resume ab seq.
    await context.setOffline(true)
    await page.waitForTimeout(1500)
    await context.setOffline(false)
    await expect(answer).toContainText('w120', { timeout: 30_000 })
    expect(await exited).toBe(0)
    // Keine doppelten oder fehlenden Nachrichten (PROTO-009 AC3).
    await expect(page.getByTestId('agent-message')).toHaveCount(1)
    await expect(page.getByText('Bitte zähle', { exact: true })).toHaveCount(1)
    expect((await answer.textContent())!.replace(/^.*?w1 /, 'w1 ').trim()).toContain(words(120))

    // Nach Reload vollständig aus dem Log (HAR-004 AC1).
    await page.reload()
    await expect(page.getByTestId('agent-message')).toHaveCount(1)
    await expect(page.getByTestId('agent-message')).toContainText(words(120))

    // Daemon-Neustart: Session ist noch da und lässt sich fortsetzen.
    await daemon.restart()
    await daemon.login(page, path)
    await expect(page.getByTestId('agent-message')).toHaveCount(1)
    await expect(page.getByTestId('agent-message')).toContainText('w120')
    await page.getByLabel('Nachricht').fill('Weiter')
    await page.getByLabel('Nachricht').press('Enter')
    await expect(page.getByTestId('agent-message')).toHaveCount(2, { timeout: 30_000 })
    const events = await daemon.events(id)
    expect(events.filter((e) => e.type === 'session.started').length).toBeGreaterThanOrEqual(2)
  })
})

test('PROTO-003 AC2: wer mitten in der Nachricht neu verbindet, sieht denselben Text', async ({ daemon, browser }) => {
  const id = await daemon.session(
    `turns:\n  - expect_input: "los"\n    emit:\n      - { message_delta: "${words(150)}", chunk: 5, chunk_delay_ms: 40 }\n`,
    'Snapshot',
  )
  const ctxA = await browser.newContext()
  const ctxB = await browser.newContext()
  const a = await ctxA.newPage()
  const b = await ctxB.newPage()
  await daemon.login(a, `/s/${id}`)
  await daemon.login(b, `/s/${id}`)
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'los' })
  await expect(b.getByTestId('agent-message').last()).toContainText('w5 ')
  await ctxB.setOffline(true)
  await b.waitForTimeout(1200)
  await ctxB.setOffline(false)
  // Nach dem Reconnect und noch während des Streamings: lückenlos ab w1.
  await expect(b.getByTestId('agent-message').last()).toContainText('w60', { timeout: 15_000 })
  const midB = (await b.getByTestId('agent-message').last().textContent())!
  // Das letzte Token kann an einer Stück-Grenze abgeschnitten sein.
  const visible = midB.match(/w\d+/g)!.map((w) => Number(w.slice(1))).slice(0, -1)
  expect(visible.slice(0, 5)).toEqual([1, 2, 3, 4, 5])
  expect(visible).toEqual(Array.from({ length: visible.length }, (_, i) => i + 1))
  // Am Ende sehen beide denselben Text.
  await expect(a.getByTestId('agent-message').last()).toContainText('w150')
  await expect(b.getByTestId('agent-message').last()).toContainText('w150')
  expect(await b.getByTestId('agent-message').last().textContent()).toBe(await a.getByTestId('agent-message').last().textContent())
  await ctxA.close()
  await ctxB.close()
})

test('PROTO-009 AC3: nach Netzabbruch keine doppelten oder fehlenden Nachrichten', async ({ daemon, page, context }) => {
  const steps = Array.from({ length: 20 }, (_, i) => `      - { message: "Nachricht ${i + 1}", delay_ms: 150 }`).join('\n')
  const id = await daemon.session(`turns:\n  - expect_input: "start"\n    emit:\n${steps}\n`, 'Zwanzig')
  // Hoch genug, damit die virtualisierte Liste alle 20 Nachrichten im DOM hat.
  await page.setViewportSize({ width: 1280, height: 2600 })
  await daemon.login(page, `/s/${id}`)
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'start' })
  await expect(page.getByText('Nachricht 3', { exact: true })).toBeVisible()
  await context.setOffline(true)
  await page.waitForTimeout(1500)
  await context.setOffline(false)
  await daemon.waitStatus(id, 'idle')
  await expect(page.getByText('Nachricht 20', { exact: true })).toBeVisible({ timeout: 15_000 })
  const texts = await page.getByTestId('agent-message').allTextContents()
  const numbers = texts.map((t) => Number(/Nachricht (\d+)/.exec(t)?.[1]))
  expect(numbers).toEqual(Array.from({ length: 20 }, (_, i) => i + 1))
})

test('QA-018 AC4: ohne Netz keine Fehlermeldung und keine Anfrage der UI nach außen', async ({ daemon, page }) => {
  const external: string[] = []
  page.on('request', (r) => {
    const u = new URL(r.url())
    if (/^(https?|wss?):$/.test(u.protocol) && !['127.0.0.1', 'localhost', '[::1]'].includes(u.hostname)) external.push(r.url())
  })
  const errors: string[] = []
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text())
  })
  const id = await daemon.session(`turns:\n  - expect_input: "hallo"\n    emit:\n      - { message: "Antwort ohne Netz" }\n`, 'Offline')
  await daemon.login(page, `/s/${id}`)
  await page.getByLabel('Nachricht').fill('hallo')
  await page.getByLabel('Nachricht').press('Enter')
  await expect(page.getByTestId('agent-message')).toContainText('Antwort ohne Netz')
  await expect(page.getByRole('alert')).toHaveCount(0)
  expect(external).toEqual([])
  expect(errors).toEqual([])
})
