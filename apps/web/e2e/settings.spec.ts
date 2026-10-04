import { expect, test } from './fixtures'

/** Jeder Turn meldet Modell, Effort und Permission-Mode, mit denen der Harness ihn ausführt. */
const ECHO = `turns:
  - emit: [{ echo_settings: true }]
  - emit: [{ echo_settings: true }]
  - emit: [{ echo_settings: true }]
`

test('WEB-004 AC2, HAR-017 AC1: Modellwechsel im Composer erzeugt ein Event und wirkt ab dem nächsten Turn', async ({ page, daemon }) => {
  const id = await daemon.session(ECHO, 'Modellwechsel')
  await daemon.login(page, `/s/${id}`)
  const box = page.getByLabel('Nachricht')
  await box.fill('eins')
  await box.press('Enter')
  await expect(page.getByText('model=fake-model effort=- mode=default')).toBeVisible()

  await page.getByRole('button', { name: 'Modell' }).click()
  const menu = page.getByRole('menu', { name: 'Modell und Effort' })
  await expect(menu).toContainText('Modell · Fake-Harness')
  await menu.getByRole('menuitemradio', { name: 'fake-large' }).click()

  // Das Event steht im Log und als Hinweis im Verlauf; der Picker zeigt das neue Modell.
  await expect(page.getByText('Modell gewechselt auf fake-large · sofort wirksam, ab Turn 2 · Verlauf bleibt erhalten')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Modell' })).toContainText('fake-large')
  await expect
    .poll(async () => (await daemon.events(id)).find((e) => e.type === 'session.settings_changed')?.payload)
    .toMatchObject({ model: 'fake-large', mechanism: 'live' })

  await box.fill('zwei')
  await box.press('Enter')
  await expect(page.getByText('model=fake-large effort=- mode=default')).toBeVisible()
})

test('HAR-017 AC3: ein nicht unterstützter Effort wird gemappt und so angezeigt', async ({ page, daemon }) => {
  // Der Fake-Harness kennt nur niedrig, mittel und hoch; „sehr hoch“ kommt über die API.
  const id = await daemon.session(ECHO, 'Effort')
  await daemon.login(page, `/s/${id}`)
  await daemon.api('PATCH', `/v1/sessions/${id}`, { effort: 'xhigh' })
  await expect(page.getByText('Effort: „sehr hoch“ angefragt, „hoch“ aktiv – Fake-Harness unterstützt keine höhere Stufe')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Effort' })).toContainText('Effort: hoch')

  await page.getByRole('button', { name: 'Effort' }).click()
  await page.getByRole('group', { name: 'Effort' }).getByRole('menuitemradio', { name: 'niedrig' }).click()
  await expect(page.getByRole('button', { name: 'Effort' })).toContainText('Effort: niedrig')
  const box = page.getByLabel('Nachricht')
  await box.fill('eins')
  await box.press('Enter')
  await expect(page.getByText('model=fake-model effort=low mode=default')).toBeVisible()
})

test('HAR-027 AC1, AC3: Permission-Mode wechseln; YOLO bleibt ohne Sandbox gesperrt', async ({ page, daemon }) => {
  const id = await daemon.session(ECHO, 'Modus')
  await daemon.login(page, `/s/${id}`)
  const trigger = page.getByRole('button', { name: 'Permission-Mode' })
  await expect(trigger).toContainText('Fragen bei Schreibzugriff')
  await trigger.click()
  const menu = page.getByRole('menu', { name: 'Permission-Mode' })
  await expect(menu).toContainText('Wie viel darf der Agent ohne Rückfrage?')
  const yolo = menu.getByRole('menuitemradio', { name: /YOLO – keine Rückfragen des Agents/ })
  await expect(yolo).toBeDisabled()
  await expect(menu).toContainText('Nicht verfügbar: Sandbox für Tools ist auf diesem Rechner aus. YOLO startet nur mit Sandbox und Egress-Proxy.')
  await menu.getByRole('menuitemradio', { name: /Nur planen/ }).click()
  await expect(trigger).toContainText('Nur planen')
  await expect
    .poll(async () => (await daemon.events(id)).find((e) => e.type === 'session.settings_changed')?.payload)
    .toMatchObject({ permission_mode: 'plan', mechanism: 'live' })

  const box = page.getByLabel('Nachricht')
  await box.fill('eins')
  await box.press('Enter')
  await expect(page.getByText('model=fake-model effort=- mode=plan')).toBeVisible()

  // Auch über die API kein YOLO (fail closed).
  await expect(daemon.api('PATCH', `/v1/sessions/${id}`, { permission_mode: 'yolo' })).rejects.toThrow(/409 .*sandbox_required/)
})
