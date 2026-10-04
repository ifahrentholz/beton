import { mkdtempSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Daemon, attachLogs, expect, test } from './fixtures'
import { fakeCli } from './fake-cli'

/** Codex als Fake-CLI, deren erster Turn die erhaltene Eingabe spiegelt (HAR-018). */
function codexEnv(): Record<string, string> {
  const dir = mkdtempSync(join(tmpdir(), 'beton-e2e-codex-'))
  // Ein Link namens `codex`, damit der Versions-Probe die Codex-Version liest.
  const link = join(dir, 'codex')
  symlinkSync(fakeCli(), link)
  const scenario = join(dir, 'codex.yaml')
  writeFileSync(scenario, 'turns: [{ emit: [{ echo_input: true }] }]\n')
  return { BETON_CODEX_PATH: `${link} --protocol app-server --scenario ${scenario}` }
}

test('SES-007 AC5: „Weiter mit Codex“ forkt ab dem letzten Ereignis, die Quelle bleibt beim Harness (SES-007 AC4: Banner mit Link)', async ({ page }, testInfo) => {
  const daemon = await Daemon.start({ env: codexEnv() })
  try {
    const source = await daemon.session('turns: [{ emit: [{ message: "Rate-Limiter steht." }] }]\n', 'Rate-Limiter')
    await daemon.api('POST', `/v1/sessions/${source}/input`, { text: 'Baue einen Rate-Limiter' })
    await expect.poll(async () => (await daemon.events(source)).some((e) => e.type === 'turn.completed')).toBe(true)
    await daemon.waitStatus(source, 'idle')

    await daemon.login(page, `/s/${source}`)
    await expect(page.getByText('Rate-Limiter steht.')).toBeVisible()
    await page.getByRole('button', { name: 'Harness' }).click()
    const menu = page.getByRole('menu')
    await expect(menu).toContainText('diese bleibt bei Fake-Harness')
    await menu.getByRole('menuitem', { name: /Weiter mit Codex/ }).click()

    // Die neue Session öffnet sich direkt, mit Banner und Link zur Quelle.
    await expect(page).not.toHaveURL(new RegExp(`/s/${source}$`))
    const fork = page.url().split('/s/')[1] ?? ''
    expect(fork).toMatch(/^ses_/)
    const banner = page.getByRole('note', { name: 'Herkunft des Forks' })
    await expect(banner).toContainText('Fortgesetzt aus „Rate-Limiter“ auf Codex')
    await expect(banner.getByRole('link', { name: /Original öffnen/ })).toHaveAttribute('href', `/s/${source}`)

    // Fork ab dem letzten `seq`; die Quelle bleibt auf ihrem Harness.
    const created = await daemon.api<{ harness: string }>('GET', `/v1/sessions/${fork}`)
    expect(created.harness).toBe('codex')
    const sourceNow = await daemon.api<{ harness: string; head_seq: number }>('GET', `/v1/sessions/${source}`)
    expect(sourceNow.harness).toBe('fake')
    const sourceEvents = await daemon.events(source)
    const marker = sourceEvents.find((e) => e.type === 'session.fork_created')
    expect(marker?.payload?.child).toBe(fork)
    await expect
      .poll(async () => (await daemon.events(fork)).find((e) => e.type === 'session.forked')?.payload)
      .toMatchObject({ from_session: source, harness: 'codex', from_harness: 'fake', history_mode: 'preamble' })

    // Der erste Turn auf Codex erhält die Übergabe.
    await page.getByLabel('Nachricht').fill('Weiter bitte')
    await page.getByLabel('Nachricht').press('Enter')
    await expect(page.getByText(/\[beton · Übergabe\]/)).toBeVisible()

    // Der Link führt zurück zur Quelle.
    await banner.getByRole('link', { name: /Original öffnen/ }).click()
    await expect(page).toHaveURL(new RegExp(`/s/${source}$`))
  } finally {
    await attachLogs(daemon, testInfo)
    daemon.stop()
  }
})
