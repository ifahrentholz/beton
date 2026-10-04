import { spawn } from 'node:child_process'
import { BETON, expect, test } from './fixtures'

test('WEB-001 AC1: `beton run` druckt eine URL, unter der die Session live sichtbar ist', async ({ daemon, page }) => {
  const scenario = daemon.scenario(
    'live',
    `turns:\n  - expect_input: "hallo"\n    emit:\n      - { message_delta: "Erster Teil. ", chunk: 4, delay_ms: 30 }\n      - { message_delta: "Zweiter Teil.", chunk: 4, delay_ms: 900 }\n`,
  )
  const run = spawn(BETON, ['run', 'fake', '--scenario', scenario, '-p', 'hallo'], {
    env: daemon.env(),
    cwd: daemon.work,
  })
  const exited = new Promise((r) => run.on('exit', r))
  const url = await new Promise<string>((resolve, reject) => {
    let err = ''
    run.stderr.on('data', (d: Buffer) => {
      err += String(d)
      const m = /(http:\/\/\S+\/s\/ses_\w+)/.exec(err)
      if (m) resolve(m[1]!)
    })
    run.on('exit', () => reject(new Error(`keine URL in stderr: ${err}`)))
  })
  await daemon.login(page, new URL(url).pathname)
  // Live: der erste Teil ist sichtbar, bevor der Turn endet.
  await expect(page.getByTestId('agent-message').last()).toContainText('Erster Teil.')
  await expect(page.getByText('Zweiter Teil.')).toHaveCount(0)
  await expect(page.getByTestId('agent-message').last()).toContainText('Zweiter Teil.', { timeout: 15_000 })
  await exited
})

test('WEB-001 AC2: Deep-Link /s/<id> lädt die Session direkt und übersteht Reload', async ({ daemon, page }) => {
  const id = await daemon.session(`turns:\n  - expect_input: "eins"\n    emit: [{ message: "Antwort eins" }]\n`, 'Deep-Link')
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'eins' })
  await daemon.waitStatus(id, 'idle')
  await daemon.login(page, `/s/${id}`)
  await expect(page.getByRole('heading', { name: 'Deep-Link' })).toBeVisible()
  await expect(page.getByText('Antwort eins')).toBeVisible()
  await page.reload()
  await expect(page).toHaveURL(new RegExp(`/s/${id}$`))
  await expect(page.getByText('Antwort eins')).toBeVisible()
})

test('WEB-001 AC3: bei 375 px kein horizontaler Scroll, Liste über Navigation erreichbar', async ({ daemon, page }) => {
  const id = await daemon.session('turns: []', 'Schmal')
  await page.setViewportSize({ width: 375, height: 812 })
  await daemon.login(page, `/s/${id}`)
  await expect(page.getByRole('heading', { name: 'Schmal' })).toBeVisible()
  const overflow = await page.evaluate(() => document.scrollingElement!.scrollWidth - document.scrollingElement!.clientWidth)
  expect(overflow).toBeLessThanOrEqual(0)
  await page.getByRole('button', { name: 'Sessions anzeigen' }).click()
  const row = page.locator(`[data-session="${id}"]`)
  await expect(row).toBeVisible()
  await row.click()
  await expect(row).toBeHidden()
  const overflowAfter = await page.evaluate(() => document.scrollingElement!.scrollWidth - document.scrollingElement!.clientWidth)
  expect(overflowAfter).toBeLessThanOrEqual(0)
})

test('WEB-001 AC4: alle Requests gehen an den eigenen Origin', async ({ daemon, page }) => {
  const foreign: string[] = []
  const origin = new URL(daemon.url).origin
  page.on('request', (r) => {
    const u = r.url()
    if (u.startsWith('data:') || u.startsWith('blob:')) return
    if (new URL(u).origin !== origin) foreign.push(u)
  })
  const id = await daemon.session(
    `turns:\n  - expect_input: "zeig code"\n    emit: [{ message: "Code:\\n\\n\`\`\`rust\\nfn main() {}\\n\`\`\`\\n\\n\`\`\`mermaid\\ngraph TD; A-->B\\n\`\`\`" }]\n`,
    'Origin',
  )
  await daemon.login(page, `/s/${id}`)
  await page.getByLabel('Nachricht').fill('zeig code')
  await page.getByLabel('Nachricht').press('Enter')
  await expect(page.locator('.md pre code span[style]').first()).toBeVisible({ timeout: 15_000 })
  await expect(page.locator('.md svg').first()).toBeVisible({ timeout: 15_000 })
  expect(foreign).toEqual([])
})

test('PROTO-001 AC4: ausgelagerte Nutzlast erscheint als Platzhalter und lädt per Blob-API', async ({ daemon, page }) => {
  const big = 'z'.repeat(70 * 1024)
  const id = await daemon.session(`turns:\n  - expect_input: "groß"\n    emit: [{ message: "${big}" }]\n`, 'Groß')
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'groß' })
  await daemon.waitStatus(id, 'idle')
  await daemon.login(page, `/s/${id}`)
  const placeholder = page.getByTestId('offloaded')
  await expect(placeholder).toBeVisible()
  await placeholder.getByRole('button', { name: 'Inhalt laden' }).click()
  await expect(page.getByText('z'.repeat(200), { exact: false })).toBeVisible()
})
