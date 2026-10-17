import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import AxeBuilder from '@axe-core/playwright'
import type { Page } from '@playwright/test'
import { expect, test } from './fixtures'

/**
 * Verstöße `serious`/`critical` in der Workspace-Rail (WEB-016). Der Bildschirminhalt des
 * Terminals ist Ausgabe der Programme (Farben der Shell), nicht UI: Er bleibt außen vor.
 */
async function axe(page: Page): Promise<string[]> {
  const res = await new AxeBuilder({ page }).include('[data-testid="workspace-rail"]').exclude('.xterm-rows').analyze()
  return res.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical').map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`)
}

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
  // Beendet: Der Tab bleibt mit dem letzten Bildschirm (DATA-011 AC2), bis man ihn ausblendet.
  await expect(tab.getByLabel('beendet')).toBeVisible()
  const after = lines(join(cwd, 'ticks.txt'))
  await page.waitForTimeout(700)
  expect(lines(join(cwd, 'ticks.txt'))).toBeLessThanOrEqual(after + 1)
  await rail(page).getByLabel('Dev-Server ausblenden').click()
  await expect(tab).toHaveCount(0)
  await expect(rail(page).getByText('Noch kein Terminal in dieser Session')).toBeVisible()
})

test('DATA-011 AC2: Terminal schließen, Seite neu laden – der letzte Bildschirm ist sichtbar', async ({ daemon, page }) => {
  const cwd = daemon.workspace({ 'README.md': 'hallo\n' })
  const id = await daemon.session('turns: []', 'Snapshot', cwd)
  await daemon.login(page, `/s/${id}`)
  await rail(page).getByRole('tab', { name: /^Terminal/ }).click()
  // Leerer Zustand mit Erklärung; der Knopf öffnet die erste Shell.
  await expect(rail(page).getByText('Noch kein Terminal in dieser Session')).toBeVisible()
  expect(await axe(page)).toEqual([])
  await rail(page).getByRole('button', { name: 'Neues Terminal' }).last().click()
  const tab = rail(page).getByRole('tablist', { name: 'Terminals' }).getByRole('tab')
  await expect(tab).toHaveCount(1)
  await rail(page).locator('.xterm').click()
  await page.keyboard.type('echo snapshot-$((40+2)) > done.txt; echo snapshot-$((40+2))\n')
  await expect.poll(() => (existsSync(join(cwd, 'done.txt')) ? readFileSync(join(cwd, 'done.txt'), 'utf8') : '')).toBe('snapshot-42\n')
  await rail(page).getByLabel('Shell 1 schließen').click()
  await expect(tab.getByLabel('beendet')).toBeVisible()
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'terminal.closed')).toBe(true)
  const snapshots = (await daemon.events(id)).filter((e) => e.type === 'terminal.snapshot')
  expect(snapshots.length).toBeGreaterThan(0)

  // Neu laden: Der Tab des beendeten Terminals ist da, mit Ende und dem letzten Bildschirm.
  await page.reload()
  await rail(page).getByRole('tab', { name: /^Terminal/ }).click()
  await expect(tab).toHaveCount(1)
  await expect(tab).toHaveClass(/italic/)
  await expect(rail(page).getByText(/^Beendet um \d\d:\d\d .*letzter Bildschirm, keine Eingabe mehr möglich$/)).toBeVisible()
  const screen = rail(page).getByTestId('terminal-snapshot')
  await expect(screen.locator('.xterm-rows')).toContainText('snapshot-42')
  expect(await axe(page)).toEqual([])
  // Nur der Bildschirm, kein Kanal: Eingaben gehen nirgendwohin.
  await expect(rail(page).getByLabel('Umbenennen')).toHaveCount(0)
})

test('UX-005 AC1: Theme-Wechsel ändert den Terminal-Hintergrund ohne Neuladen', async ({ daemon, page }) => {
  const id = await daemon.session('turns: []', 'Terminal-Theme')
  // Ausgangspunkt Nord; das Terminal bleibt dunkel und nimmt den dunklen Satz der Familie.
  await daemon.login(page, '/settings/appearance')
  await page.getByRole('radio', { name: /^Nord/ }).click()
  await expect(page.locator('html')).toHaveClass(/theme-nord/)
  await page.goto(`${daemon.url}/s/${id}`)
  await rail(page).getByRole('tab', { name: /^Terminal/ }).click()
  await rail(page).getByRole('button', { name: 'Neues Terminal' }).last().click()
  const bg = rail(page).locator('.xterm .xterm-scrollable-element')
  await expect(bg).toBeVisible()
  const color = () => bg.evaluate((el) => getComputedStyle(el).backgroundColor)
  const sunken = () => rail(page).getByTestId('terminal-screen').evaluate((el) => getComputedStyle(el).backgroundColor)
  await expect.poll(color).toBe(await sunken())
  const nord = await color()
  await page.evaluate(() => ((window as unknown as { __noReload: boolean }).__noReload = true))
  // „Theme wechseln“ über die Palette: Von Nord geht es auf Sichtbeton.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type('theme')
  await page.keyboard.press('Enter')
  await expect(page.locator('html')).not.toHaveClass(/theme-nord/)
  await expect.poll(color).not.toBe(nord)
  expect(await color()).toBe(await sunken())
  expect(await page.evaluate(() => (window as unknown as { __noReload?: boolean }).__noReload === true)).toBe(true)
})
