import { join } from 'node:path'
import type { Page } from '@playwright/test'
import { expect, test, type Daemon } from './fixtures'

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.BETON_E2E_SHOTS
  if (dir) await page.screenshot({ path: join(dir, `${name}.png`) })
}

/** Session ohne Titel; der Fake beantwortet den Einmal-Aufruf für den Titel (SES-010). */
async function untitled(daemon: Daemon, yaml: string): Promise<string> {
  const scenario = daemon.scenario(`t${Date.now()}`, yaml)
  const s = await daemon.api<{ id: string }>('POST', '/v1/sessions', { target: 'fake', cwd: daemon.work, harness_opts: { scenario } })
  await daemon.waitStatus(s.id, 'idle')
  return s.id
}

async function titleEvent(daemon: Daemon, id: string): Promise<{ title: string; source: string } | undefined> {
  const e = (await daemon.events(id)).find((x) => x.type === 'session.title_changed')
  return e?.payload as { title: string; source: string } | undefined
}

test('UX-009 AC1: Ein erzeugter Titel erscheint binnen 1 s in Sidebar und Fenstertitel aller Clients', async ({ daemon, browser }) => {
  const id = await untitled(
    daemon,
    'one_shot: { reply: "Rate-Limiter für die Login-API" }\nturns:\n  - emit:\n      - { message: "Mache ich, mit Tests.", delay_ms: 300 }\n',
  )
  const a = await (await browser.newContext()).newPage()
  const b = await (await browser.newContext()).newPage()
  await daemon.login(a, `/s/${id}`)
  await daemon.login(b, `/s/${id}`)
  for (const p of [a, b]) {
    await expect(p.getByRole('heading', { name: 'Neue Session' })).toBeVisible()
    await expect(p.locator(`[data-session="${id}"]`)).toContainText('Neue Session')
    await expect(p).toHaveTitle('Neue Session · beton')
  }
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP.' })
  await expect.poll(() => titleEvent(daemon, id), { intervals: [20] }).toEqual({ title: 'Rate-Limiter für die Login-API', source: 'generated' })
  // Ab hier höchstens 1 s.
  for (const p of [a, b]) {
    await expect(p.locator(`[data-session="${id}"]`)).toContainText('Rate-Limiter für die Login-API', { timeout: 1000 })
    await expect(p).toHaveTitle('Rate-Limiter für die Login-API · beton', { timeout: 1000 })
  }
  await expect(a.getByRole('heading', { name: 'Rate-Limiter für die Login-API' })).toBeVisible()
  await expect(a.getByText('automatisch')).toBeVisible()
  await shot(a, 'titel-erzeugt')
})

test('UX-009 AC2: Nach Inline-Umbenennung bleibt der User-Titel, ein später erzeugter wird nicht angewendet', async ({ daemon, page }) => {
  const id = await untitled(
    daemon,
    'one_shot: { reply: "Zu spät erzeugt" }\nturns:\n  - emit:\n      - { message: "Ich arbeite daran.", delay_ms: 1500 }\n',
  )
  await daemon.login(page, `/s/${id}`)
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'Login absichern' })
  await daemon.waitStatus(id, 'running')
  // Umbenennen per Doppelklick, während der erste Turn läuft.
  await page.getByRole('heading', { name: 'Neue Session' }).dblclick()
  const input = page.getByLabel('Titel der Session')
  await input.fill('Login-Schutz gegen Brute-Force')
  await expect(page.getByText('danach kein automatischer Titel mehr')).toBeVisible()
  await shot(page, 'titel-umbenennen')
  await input.press('Enter')
  await expect(page.getByRole('heading', { name: 'Login-Schutz gegen Brute-Force' })).toBeVisible()
  await expect(page.getByText('von dir benannt')).toBeVisible()
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'turn.completed')).toBe(true)
  await page.waitForTimeout(1000)
  await expect(page.getByRole('heading', { name: 'Login-Schutz gegen Brute-Force' })).toBeVisible()
  await expect(page.locator(`[data-session="${id}"]`)).toContainText('Login-Schutz gegen Brute-Force')
  await expect(page).toHaveTitle('Login-Schutz gegen Brute-Force · beton')
  const titles = (await daemon.events(id)).filter((e) => e.type === 'session.title_changed').map((e) => e.payload)
  expect(titles).toEqual([{ title: 'Login-Schutz gegen Brute-Force', source: 'user' }])
  // F2 öffnet das Umbenennen erneut.
  await page.getByTestId('stream').click()
  await page.keyboard.press('F2')
  await expect(page.getByLabel('Titel der Session')).toBeVisible()
  await page.keyboard.press('Escape')
})
