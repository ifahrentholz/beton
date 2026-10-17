import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Page } from '@playwright/test'
import { expect, test } from './fixtures'

const rail = (page: Page) => page.getByTestId('workspace-rail')

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 })
})

const lines = (file: string) => (existsSync(file) ? readFileSync(file, 'utf8').split('\n').filter(Boolean).length : 0)

test('SES-019 AC1, WEB-010: Terminal in der Web-UI, Eingaben erreichen das PTY, der Prozess überlebt das Schließen der Seite', async ({ daemon, page }) => {
  const cwd = daemon.workspace({ 'README.md': 'hallo\n' })
  const id = await daemon.session('turns: []', 'Terminals', cwd)
  await daemon.login(page, `/s/${id}`)
  await rail(page).getByRole('tab', { name: /^Terminal/ }).click()
  await rail(page).getByLabel('Neues Terminal').click()
  const tab = rail(page).getByRole('tablist', { name: 'Terminals' }).getByRole('tab')
  await expect(tab).toHaveCount(1)
  await expect(tab).toHaveAttribute('aria-selected', 'true')
  // Die E2E-Daemons laufen ausdrücklich mit `backend: none` (SBX-006); das steht dran.
  await expect(rail(page).getByText('Ohne Sandbox (backend: none)')).toBeVisible()
  const term = rail(page).locator('.xterm')
  await expect(term).toBeVisible()
  await term.click()
  // Ein Langläufer wie ein Dev-Server: schreibt jede Zehntelsekunde eine Zeile.
  await page.keyboard.type('echo e2e-ok > ok.txt; while :; do echo tick >> ticks.txt; sleep 0.1; done\n')
  await expect.poll(() => (existsSync(join(cwd, 'ok.txt')) ? readFileSync(join(cwd, 'ok.txt'), 'utf8') : '')).toBe('e2e-ok\n')
  await expect.poll(() => lines(join(cwd, 'ticks.txt'))).toBeGreaterThan(3)
  // Alle Clients weg: Der Prozess läuft weiter.
  await page.goto('about:blank')
  const before = lines(join(cwd, 'ticks.txt'))
  await page.waitForTimeout(1500)
  expect(lines(join(cwd, 'ticks.txt'))).toBeGreaterThan(before + 5)
  // Zurück: Das Terminal ist noch da, mit Größe aus dem Fenster.
  await page.goto(`${daemon.url}/s/${id}`)
  await rail(page).getByRole('tab', { name: /^Terminal/ }).click()
  await expect(tab).toHaveCount(1)
  await expect(rail(page).locator('.xterm')).toBeVisible()
  await expect(rail(page).getByTestId('terminal-size')).toHaveText(/^\d+ × \d+$/)
  const list = await daemon.api<{ items: Array<{ terminal_id: string; cols: number; rows: number; name: string }> }>('GET', `/v1/sessions/${id}/terminals`)
  expect(list.items).toHaveLength(1)
  // WEB-010 AC2: Die Größe des Fensters ist beim PTY angekommen (nicht mehr die Startgröße).
  await expect.poll(async () => {
    const l = await daemon.api<{ items: Array<{ cols: number }> }>('GET', `/v1/sessions/${id}/terminals`)
    return l.items[0]?.cols
  }).not.toBe(120)
  // Umbenennen und Schließen über die Tab-Leiste.
  await rail(page).getByLabel('Umbenennen').click()
  await rail(page).getByLabel('Name des Terminals').fill('Dev-Server')
  await rail(page).getByLabel('Name des Terminals').press('Enter')
  await expect(tab).toHaveText(/Dev-Server/)
  await rail(page).getByLabel('Dev-Server schließen').click()
  await expect(tab).toHaveCount(0)
  const after = lines(join(cwd, 'ticks.txt'))
  await page.waitForTimeout(700)
  expect(lines(join(cwd, 'ticks.txt'))).toBeLessThanOrEqual(after + 1)
})
