import { readFileSync } from 'node:fs'
import type { UsageReportView } from '@beton/sdk'
import { kpiRange, kpis, periods, rowsFor, dollars, fmtTokens, groupByFor, tzOffset } from '../src/lib/usage'
import { Daemon, expect, test } from './fixtures'

/*
 * Verbrauch (USE-006), Kontextanzeige (USE-008) und Compaction (USE-009) gegen den echten
 * Daemon mit Fake-Harness. Ein eigener Daemon je Test, damit der Datensatz genau der des Tests ist.
 */

/** Zwei Turns mit gemeldeten Kosten, einer ohne Preis (Modell nicht im Katalog). */
const PRICED = `turns:
  - emit:
      - { message: "Erledigt." }
      - { usage: { input_tokens: 120000, output_tokens: 3000, cache_read_tokens: 40000, cost_usd: 0.12 } }
  - emit:
      - { message: "Noch einmal." }
      - { usage: { input_tokens: 80000, output_tokens: 2000, cost_usd: 0.08 } }
`
const UNPRICED = `turns:
  - emit:
      - { message: "Ohne Preis." }
      - { usage: { input_tokens: 40000, output_tokens: 1000, model: "eigenes-modell-ohne-preis" } }
`
/** Kontext bei 75 % eines 200k-Fensters (USE-008 AC1). */
const CONTEXT = `turns:
  - emit:
      - { message: "Viel gelesen." }
      - { usage: { input_tokens: 150000, output_tokens: 500, context_window: 200000 } }
`

async function turns(d: Daemon, id: string, texts: string[]): Promise<void> {
  for (const [i, text] of texts.entries()) {
    await d.api('POST', `/v1/sessions/${id}/input`, { text })
    await expect.poll(async () => (await d.events(id)).filter((e) => e.type === 'turn.completed').length, { timeout: 20_000 }).toBe(i + 1)
  }
}

async function withDaemon(fn: (d: Daemon) => Promise<void>): Promise<void> {
  const d = await Daemon.start()
  try {
    await fn(d)
  } finally {
    d.stop()
  }
}

test('USE-006 AC1: die Seite zeigt für den Fake-Harness-Datensatz dieselben Summen wie GET /v1/usage', async ({ page }) => {
  await withDaemon(async (d) => {
    const a = await d.session(PRICED, 'Rechnung prüfen')
    const b = await d.session(UNPRICED, 'Eigenes Modell')
    await turns(d, a, ['eins', 'zwei'])
    await turns(d, b, ['eins'])

    const now = new Date()
    const tz = tzOffset(now)
    const period = periods(now).find((p) => p.id === '14d')!
    const q = (from: string, to: string, by: string[]) =>
      d.api<UsageReportView>('GET', `/v1/usage?from=${from}&to=${to}&tz=${encodeURIComponent(tz)}&group_by=${by.join(',')}`)
    const table = await q(period.from, period.to, groupByFor('session'))
    const range = kpiRange(now)
    const k = kpis((await q(range.from, range.to, ['day'])).groups, now)

    await d.login(page, '/usage')
    await expect(page.getByRole('heading', { name: 'Verbrauch' })).toBeVisible()
    // Kennzahlen: dieselben Summen wie die API (Tokens, Turns, Kosten nur nach Verbrauch).
    const kpiBox = page.getByTestId('usage-kpis')
    await expect(kpiBox).toContainText(`${fmtTokens(k.today.tokens)}Tokens · ${k.today.turns} Turns`)
    await expect(kpiBox).toContainText(dollars(k.cost7))
    expect(k.today.turns).toBe(3)
    expect(k.cost7).toBeCloseTo(0.2, 6)
    // Aufschlüsselung je Session: jede Zeile mit den Summen der API.
    const rows = rowsFor('session', table.groups, '')
    expect(rows).toHaveLength(2)
    for (const r of rows) {
      const tr = page.getByTestId('usage-table').locator(`tr[data-key="${r.key}"]`)
      const cells = tr.locator('td')
      await expect(cells.nth(1)).toHaveText(String(r.turns))
      await expect(cells.nth(2)).toHaveText(fmtTokens(r.input))
      await expect(cells.nth(3)).toHaveText(fmtTokens(r.output))
      await expect(cells.nth(4)).toHaveText(fmtTokens(r.cache))
    }
    const priced = rows.find((r) => r.sessionId === a)!
    expect(priced).toMatchObject({ turns: 2, input: 200_000, output: 5000, cache: 40_000, billing: { kind: 'cost' } })
    await expect(page.locator(`tr[data-key="${a}"] td`).nth(5)).toHaveText(dollars(0.2))
  })
})

test('USE-006 AC2–AC4: Session öffnen, CSV der Ansicht und Hinweis ohne Preis mit Weg zu den Preisen', async ({ page }) => {
  await withDaemon(async (d) => {
    const a = await d.session(PRICED, 'Rechnung prüfen')
    const b = await d.session(UNPRICED, 'Eigenes Modell')
    await turns(d, a, ['eins', 'zwei'])
    await turns(d, b, ['eins'])
    await d.login(page, '/usage')
    const table = page.getByTestId('usage-table')
    await expect(table.locator(`tr[data-key="${b}"]`)).toBeVisible()

    // AC3: CSV mit den sichtbaren Spalten, Zeitraum und Gruppierung im Dateinamen.
    const period = periods(new Date()).find((p) => p.id === '14d')!
    const [download] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'CSV exportieren' }).click()])
    expect(download.suggestedFilename()).toBe(`verbrauch_${period.from}_${period.to}_nach-session.csv`)
    const csv = readFileSync((await download.path())!, 'utf8')
    expect(csv.split('\r\n')[0]).toBe('Session,Turns,Eingabe,Ausgabe,Cache gelesen,Abrechnung')
    expect(csv).toContain('Rechnung prüfen,2,200000,5000,40000,"0,20')
    expect(csv).toContain('Eigenes Modell,1,40000,1000,0,ohne Preis')
    await page.getByRole('radio', { name: 'Modell' }).click()
    await expect(page.getByRole('button', { name: 'CSV exportieren' })).toHaveAttribute('title', `verbrauch_${period.from}_${period.to}_nach-modell.csv`)
    await page.getByRole('radio', { name: 'Session' }).click()

    // AC4: Hinweis „ohne Preis“ mit Link zu den Preisen; dort steht das Modell.
    const link = table.locator(`tr[data-key="${b}"]`).getByRole('link', { name: 'ohne Preis – eigenen Preis anlegen' })
    await link.click()
    await expect(page).toHaveURL(/\/usage\/pricing$/)
    await expect(page.getByTestId('unpriced-hint')).toContainText('1 Turn ohne Preis')
    await expect(page.getByTestId('unpriced-hint')).toContainText('fake/eigenes-modell-ohne-preis')
    await expect(page.getByTestId('catalog-table')).toContainText('claude-opus-5-5')

    // AC2: Klick auf eine Session öffnet sie.
    await page.goBack()
    await page.getByTestId('usage-table').locator(`tr[data-key="${a}"]`).getByText('Rechnung prüfen').click()
    await expect(page).toHaveURL(new RegExp(`/s/${a}$`))
    await expect(page.getByText('Noch einmal.')).toBeVisible()
  })
})

test('USE-008 AC1 USE-009 AC1: Ring bei 75 %, „Kompaktieren“ erzeugt die Events und die Anzeige folgt ohne Reload', async ({ daemon, page }) => {
  const id = await daemon.session(CONTEXT, 'Kontext')
  await turns(daemon, id, ['lies alles'])
  await daemon.login(page, `/s/${id}`)
  const ring = page.getByTestId('context-ring')
  await expect(ring).toHaveText(/75 % Kontext/)
  await expect(ring).toHaveAttribute('data-level', 'warn')
  await expect(ring).toContainText('wird knapp')
  await expect(page.getByTestId('context-bar')).toContainText('150.000 von 200.000 Tokens')
  await page.getByRole('button', { name: 'Kompaktieren' }).click()
  await expect(page.getByText('Kontext kompaktiert · 150.000 → 30.000 Tokens · von dir ausgelöst')).toBeVisible()
  await expect(ring).toHaveText('15 % Kontext')
  const types = (await daemon.events(id)).map((e) => e.type)
  expect(types.filter((t) => t === 'compaction.started')).toHaveLength(1)
  expect(types.indexOf('compaction.completed')).toBeGreaterThan(types.indexOf('compaction.started'))
})

test('USE-009 AC3: bei compaction: none ist „Kompaktieren“ deaktiviert und nennt den Grund', async ({ daemon, page }) => {
  const id = await daemon.session(`capabilities: { compaction: none }\n${CONTEXT}`, 'Ohne Compaction')
  await turns(daemon, id, ['lies alles'])
  await daemon.login(page, `/s/${id}`)
  const button = page.getByRole('button', { name: 'Kompaktieren' })
  await expect(button).toBeDisabled()
  await expect(button).toHaveAttribute('title', 'Fake-Harness kann nicht kompaktieren. Starte bei Bedarf eine neue Session.')
  await expect(page.getByTestId('context-bar')).toContainText('nicht verfügbar für diesen Harness')
})
