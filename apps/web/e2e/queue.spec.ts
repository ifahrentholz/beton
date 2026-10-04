import { expect, test } from './fixtures'

/** Turn, der läuft, bis er unterbrochen wird. */
const HANG = 'turns:\n  - emit: [{ message: "Ich arbeite." }, { hang: true }]\n'

const texts = (rows: import('@playwright/test').Locator) =>
  rows.evaluateAll((els) => els.map((e) => e.querySelector('span.truncate')?.textContent ?? ''))

test('WEB-005 AC1: Drag-&-Drop-Reorder wird an alle Clients propagiert', async ({ daemon, browser }) => {
  const id = await daemon.session(HANG, 'Queue')
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'los' })
  await daemon.waitStatus(id, 'running')
  for (const text of ['erst das', 'dann das', 'zuletzt']) {
    await daemon.api('POST', `/v1/sessions/${id}/input`, { text })
  }
  const a = await (await browser.newContext()).newPage()
  const b = await (await browser.newContext()).newPage()
  await daemon.login(a, `/s/${id}`)
  await daemon.login(b, `/s/${id}`)
  const rowsA = a.getByTestId('queue-row')
  const rowsB = b.getByTestId('queue-row')
  await expect(rowsA).toHaveCount(3)
  await expect(rowsB).toHaveCount(3)
  await rowsA.nth(2).dragTo(rowsA.nth(0))
  await expect.poll(() => texts(rowsB)).toEqual(['zuletzt', 'erst das', 'dann das'])
  await expect.poll(() => texts(rowsA)).toEqual(['zuletzt', 'erst das', 'dann das'])
  // Entfernen in A ist in B ebenfalls sichtbar.
  await rowsA.nth(1).getByRole('button', { name: 'Entfernen' }).click()
  await expect(rowsB).toHaveCount(2)
  await daemon.api('POST', `/v1/sessions/${id}/interrupt`)
})

test('WEB-005 AC2: „Als Steer senden“ ist ohne Steering deaktiviert (mit Tooltip)', async ({ daemon, page }) => {
  const id = await daemon.session(`capabilities: { steering: false }\n${HANG}`, 'Ohne Steering')
  await daemon.login(page, `/s/${id}`)
  await page.getByLabel('Nachricht').fill('los')
  await page.getByLabel('Nachricht').press('Enter')
  await expect(page.getByRole('button', { name: 'Unterbrechen' })).toBeVisible()
  // Während des Turns reiht der Composer ein.
  await page.getByLabel('Nachricht').fill('bitte danach')
  await expect(page.getByRole('button', { name: 'Einreihen' })).toBeVisible()
  await page.getByLabel('Nachricht').press('Enter')
  const row = page.getByTestId('queue-row')
  await expect(row).toContainText('bitte danach')
  const steer = row.getByRole('button', { name: 'Jetzt lenken' })
  await expect(steer).toBeDisabled()
  await expect(steer.locator('..')).toHaveAttribute('title', /kann nicht in einen laufenden Turn eingreifen/)
  // Mit Steering (Fake-Standard) ist die Aktion möglich und landet im laufenden Turn.
  const withSteer = await daemon.session(
    'turns:\n  - emit: [{ message: "Ich arbeite." }, { await_steer: "auch die Tests" }, { message: "Tests ergänze ich." }]\n',
    'Mit Steering',
  )
  await page.goto(`${daemon.url}/s/${withSteer}`)
  await page.getByLabel('Nachricht').fill('los')
  await page.getByLabel('Nachricht').press('Enter')
  await expect(page.getByText('Ich arbeite.')).toBeVisible()
  await page.getByLabel('Nachricht').fill('auch die Tests')
  await page.getByLabel('Nachricht').press('Enter')
  const enabled = page.getByTestId('queue-row').getByRole('button', { name: 'Jetzt lenken' })
  await expect(enabled).toBeEnabled()
  await enabled.click()
  await expect(page.getByText('Tests ergänze ich.')).toBeVisible()
  await expect(page.getByTestId('queue-row')).toHaveCount(0)
  const types = (await daemon.events(withSteer)).map((e) => e.type)
  expect(types.filter((t) => t === 'turn.started')).toHaveLength(1)
  expect(types).not.toContain('turn.interrupted')
  await daemon.api('POST', `/v1/sessions/${id}/interrupt`)
})

test('WEB-005: Esc zweimal unterbricht den laufenden Turn', async ({ daemon, page }) => {
  const id = await daemon.session(HANG, 'Esc Esc')
  await daemon.login(page, `/s/${id}`)
  await page.getByLabel('Nachricht').fill('los')
  await page.getByLabel('Nachricht').press('Enter')
  await expect(page.getByRole('button', { name: 'Unterbrechen' })).toBeVisible()
  await page.keyboard.press('Escape')
  await page.keyboard.press('Escape')
  await expect(page.getByRole('button', { name: 'Unterbrechen' })).toBeHidden()
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'turn.interrupted')).toBe(true)
})

test('SES-012 AC2, AC3: angepinnt oben, gelesen auf dem anderen Gerät binnen 2 s', async ({ daemon, browser }) => {
  const pinned = await daemon.session('turns: []', 'Angepinnt alt')
  const unread = await daemon.session('turns:\n  - emit: [{ message: "Fertig." }]\n', 'Ungelesen')
  await daemon.api('PUT', `/v1/sessions/${pinned}/pin`, { pinned: true })
  await daemon.api('POST', `/v1/sessions/${unread}/input`, { text: 'los' })
  await expect.poll(async () => (await daemon.events(unread)).some((e) => e.type === 'turn.completed')).toBe(true)
  const deviceB = await (await browser.newContext()).newPage()
  await daemon.login(deviceB, `/s/${pinned}`)
  const list = deviceB.getByTestId('session-list')
  await expect(list.getByTestId('session-group').first()).toHaveText('Angepinnt')
  await expect(list.getByTestId('session-row').first()).toHaveAttribute('data-session', pinned)
  const row = list.locator(`[data-session="${unread}"]`)
  await expect(row).toHaveAttribute('data-unread', 'true')
  // Gerät A öffnet die Session; Gerät B zeigt sie binnen 2 s als gelesen.
  const deviceA = await (await browser.newContext()).newPage()
  await daemon.login(deviceA, `/s/${unread}`)
  await expect(deviceA.getByText('Fertig.')).toBeVisible()
  await expect(row).not.toHaveAttribute('data-unread', 'true', { timeout: 2500 })
})
