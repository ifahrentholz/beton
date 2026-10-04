import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Page } from '@playwright/test'
import { expect, test } from './fixtures'

/** Turn, der eine Datei schreibt (Fake-Harness, `write_file`). */
const write = (prompt: string, path: string, content: string, message = 'Erledigt.') =>
  `  - expect_input: ${JSON.stringify(prompt)}\n    emit:\n      - { write_file: { path: ${JSON.stringify(path)}, content: ${JSON.stringify(content)} } }\n      - { message: ${JSON.stringify(message)} }\n`

const rail = (page: Page) => page.getByTestId('workspace-rail')
const tab = (page: Page, name: string) => rail(page).getByRole('tab', { name: new RegExp(`^${name}`) })

/**
 * Modifier, den Monaco erwartet: Monaco erkennt die Plattform am User-Agent, und der
 * emulierte Desktop-Chrome meldet Windows, auch wenn der Test auf macOS läuft.
 */
const mod = (page: Page) => page.evaluate(() => (/Macintosh/.test(navigator.userAgent) ? 'Meta' : 'Control'))

/** Monaco im Editor fokussieren und am Ende tippen. */
async function typeInEditor(page: Page, text: string) {
  const editor = page.getByTestId('code-editor')
  await expect(editor.locator('.view-lines')).toBeVisible()
  await editor.locator('.view-lines').click()
  await page.keyboard.press(`${await mod(page)}+End`)
  await page.keyboard.type(text)
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 })
})

test('WEB-008 AC1: fs.changed setzt ein Change-Badge am Files-/Changes-Tab', async ({ daemon, page }) => {
  const cwd = daemon.workspace({ 'notes.txt': 'alt\n' })
  const id = await daemon.session(`turns:\n${write('eins', 'notes.txt', 'neu\n')}${write('zwei', 'neu.txt', 'hallo\n')}`, 'Badge', cwd)
  await daemon.login(page, `/s/${id}`)
  await expect(tab(page, 'Änderungen')).toHaveAttribute('aria-selected', 'true')
  await expect(rail(page).getByTestId('badge-files')).toHaveCount(0)

  // Der Agent schreibt, während „Änderungen“ offen ist: Badge am Files-Tab.
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'eins' })
  await expect(rail(page).getByTestId('badge-files')).toHaveText('1')
  await expect(rail(page).getByTestId('badge-changes')).toHaveCount(0)

  // Files ansehen: Badge weg. Nächste Änderung: Badge am Changes-Tab.
  await tab(page, 'Dateien').click()
  await expect(rail(page).getByTestId('badge-files')).toHaveCount(0)
  await daemon.waitStatus(id, 'idle')
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'zwei' })
  await expect(rail(page).getByTestId('badge-changes')).toHaveText('1')
  await tab(page, 'Änderungen').click()
  await expect(rail(page).getByTestId('badge-changes')).toHaveCount(0)
  await expect(rail(page).getByTestId('changed-file')).toContainText(['neu.txt'])
})

test('WEB-008 AC2: nicht verfügbare Tabs sind ausgeblendet, nicht kaputt', async ({ daemon, page }) => {
  const id = await daemon.session('turns: []', 'Tabs', daemon.workspace({ 'a.txt': 'a\n' }))
  await daemon.login(page, `/s/${id}`)
  const tabs = rail(page).getByRole('tablist', { name: 'Workspace' }).getByRole('tab')
  await expect(tabs).toHaveText(['Dateien', 'Änderungen'])
  // Terminal, Browser (M3), Agents (WEB-012, WP-27), PR, Side-Chats, Kommentare (M4) fehlen.
  for (const name of ['Terminal', 'Browser', 'Agents', 'PR', 'Side-Chats', 'Kommentare']) {
    await expect(tabs.filter({ hasText: name })).toHaveCount(0)
  }
  // Tabs per Tastenkürzel wechseln.
  await page.keyboard.press('Alt+1')
  await expect(tab(page, 'Dateien')).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Alt+2')
  await expect(tab(page, 'Änderungen')).toHaveAttribute('aria-selected', 'true')
})

test('WEB-009 AC1: Agent ändert eine geöffnete, ungespeicherte Datei → Konfliktdialog statt Überschreiben', async ({ daemon, page }) => {
  const cwd = daemon.workspace({ 'src/app.ts': 'export const answer = 41\n' })
  const id = await daemon.session(`turns:\n${write('ändern', 'src/app.ts', 'export const answer = 42 // Agent\n')}`, 'Konflikt', cwd)
  await daemon.login(page, `/s/${id}`)
  await tab(page, 'Dateien').click()
  await rail(page).getByRole('treeitem', { name: 'src' }).click()
  await rail(page).getByRole('treeitem', { name: 'app.ts' }).click()
  await typeInEditor(page, '// meins')
  await expect(page.getByTestId('save-state')).toHaveText(/Ungespeichert/)

  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'ändern' })
  const dialog = page.getByRole('alertdialog')
  await expect(dialog).toBeVisible()
  await expect(dialog).toContainText('Fake-Harness hat app.ts geändert')
  await expect(dialog).toContainText('// meins')
  await expect(dialog).toContainText('42 // Agent')
  // Nichts still überschrieben: auf der Platte steht der Stand des Agents.
  expect(readFileSync(join(cwd, 'src/app.ts'), 'utf8')).toBe('export const answer = 42 // Agent\n')

  await dialog.getByRole('button', { name: 'Meine Version speichern' }).click()
  await expect(dialog).toBeHidden()
  await expect(page.getByTestId('save-state')).toHaveText('Gespeichert')
  expect(readFileSync(join(cwd, 'src/app.ts'), 'utf8')).toContain('// meins')
})

test('WEB-009 AC2: Monaco ist nicht im initialen Bundle und lädt erst mit der ersten Datei', async ({ daemon, page }) => {
  const dist = join(import.meta.dirname, '..', 'dist')
  const html = readFileSync(join(dist, 'index.html'), 'utf8')
  const initial = [...new Set([...html.matchAll(/(?:src|href)="\/(assets\/[^"]+\.js)"/g)].map((m) => m[1]!))]
  expect(initial.length).toBeGreaterThan(0)
  for (const f of initial) expect(readFileSync(join(dist, f), 'utf8')).not.toMatch(/MonacoEnvironment|monaco-editor/)

  const scripts: string[] = []
  page.on('request', (r) => {
    if (r.resourceType() === 'script') scripts.push(new URL(r.url()).pathname)
  })
  const id = await daemon.session('turns: []', 'Bundle', daemon.workspace({ 'a.md': '# Titel\n\nText\n' }))
  await daemon.login(page, `/s/${id}`)
  await expect(tab(page, 'Änderungen')).toBeVisible()
  await page.waitForLoadState('networkidle')
  expect(scripts.some((s) => s.includes('editor-impl'))).toBe(false)

  await tab(page, 'Dateien').click()
  await rail(page).getByRole('treeitem', { name: 'a.md' }).click()
  await expect(page.getByTestId('code-editor').locator('.view-lines')).toContainText('Titel')
  expect(scripts.some((s) => s.includes('editor-impl'))).toBe(true)
  // Markdown-Vorschau neben dem Quelltext.
  await rail(page).getByRole('button', { name: 'Vorschau' }).click()
  await expect(page.getByTestId('markdown-preview').getByRole('heading', { name: 'Titel' })).toBeVisible()
})

async function frames(page: Page, ms: number, scroller: string) {
  return page.evaluate(
    ({ duration, sel }) =>
      new Promise<{ median: number; slowShare: number; count: number }>((resolve) => {
        const el = document.querySelector<HTMLElement>(sel)!
        const deltas: number[] = []
        let last = performance.now()
        const end = last + duration
        const step = (t: number) => {
          deltas.push(t - last)
          last = t
          // Gleichmäßig nach unten scrollen, am Ende wieder nach oben.
          el.scrollTop = el.scrollTop + 400 >= el.scrollHeight - el.clientHeight ? 0 : el.scrollTop + 400
          if (t < end) requestAnimationFrame(step)
          else {
            const d = deltas.slice(1)
            const sorted = [...d].sort((a, b) => a - b)
            resolve({ median: 1000 / sorted[Math.floor(sorted.length / 2)]!, slowShare: d.filter((x) => x > 32).length / d.length, count: d.length })
          }
        }
        requestAnimationFrame(step)
      }),
    { duration: ms, sel: scroller },
  )
}

test('WEB-011 AC1: Ein Diff mit 5 000 geänderten Zeilen bleibt flüssig scrollbar (virtualisiert)', async ({ daemon, page }) => {
  const content = Array.from({ length: 5000 }, (_, i) => `export const value${i} = ${i} // Zeile ${i + 1}`).join('\n') + '\n'
  const id = await daemon.session(`turns:\n${write('groß', 'big.ts', content)}`, 'Großer Diff', daemon.workspace({}))
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'groß' })
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'fs.changed'), { timeout: 15_000 }).toBe(true)
  await daemon.login(page, `/s/${id}`)
  // Kein Git-Repository: die Rail zeigt gleich die Sicht pro Turn.
  await expect(rail(page).getByRole('tab', { name: 'Pro Turn' })).toHaveAttribute('aria-selected', 'true')
  await expect(rail(page).getByTestId('changed-file')).toContainText('big.ts')
  await expect(rail(page).locator('[data-line="N1"]')).toBeVisible()
  // Virtualisiert: nur ein kleiner Ausschnitt der 5 000 Zeilen steht im DOM.
  expect(await rail(page).locator('[data-line]').count()).toBeLessThan(200)
  // Lokal CI-Bedingungen nachstellen: BETON_CPU_THROTTLE=6 bremst die CPU im Browser.
  if (process.env.BETON_CPU_THROTTLE) {
    const cdp = await page.context().newCDPSession(page)
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: Number(process.env.BETON_CPU_THROTTLE) })
  }
  const f = await frames(page, 4000, '[data-testid="diff-list"]')
  test.info().annotations.push({ type: 'Messwerte', description: JSON.stringify({ fpsMedian: Math.round(f.median), slowFramesPct: Math.round(f.slowShare * 1000) / 10, frames: f.count }) })
  console.log('WEB-011 Messwerte', test.info().annotations.at(-1)?.description)
  expect(f.count).toBeGreaterThan(100)
  expect(f.median).toBeGreaterThanOrEqual(55)
  expect(f.slowShare).toBeLessThan(0.05)
  expect(await rail(page).locator('[data-line]').count()).toBeLessThan(200)
  // Die letzte Zeile ist erreichbar.
  await page.locator('[data-testid="diff-list"]').evaluate((el) => (el.scrollTop = el.scrollHeight))
  await expect(rail(page).locator('[data-line="N5000"]')).toBeVisible()
})

test('WEB-011 AC2: Klick auf eine Zeilennummer erzeugt einen teilbaren Link auf diese Zeile', async ({ daemon, page, browser }) => {
  const content = Array.from({ length: 40 }, (_, i) => `Zeile ${i + 1}`).join('\n') + '\n'
  const id = await daemon.session(`turns:\n${write('los', 'liste.txt', content)}`, 'Anker', daemon.workspace({}))
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'los' })
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'fs.changed'), { timeout: 15_000 }).toBe(true)
  await daemon.login(page, `/s/${id}`)
  await rail(page).getByRole('button', { name: 'Link auf Zeile 16 (neu)' }).click()
  await expect(page).toHaveURL(/line=N16/)
  const url = new URL(page.url())
  expect(url.searchParams.get('file')).toBe('liste.txt')
  expect(url.searchParams.get('scope')).toBe('turn')
  expect(url.searchParams.get('turn')).toMatch(/^trn_/)
  await expect(rail(page).locator('[data-line="N16"]')).toHaveAttribute('data-anchored', 'true')
  await expect(rail(page).getByRole('button', { name: 'Link auf diese Zeile kopieren' })).toBeVisible()

  // Der Link öffnet in einem anderen Browser direkt die Zeile.
  const other = await (await browser.newContext({ viewport: { width: 1600, height: 900 } })).newPage()
  await daemon.login(other, `${url.pathname}${url.search}`)
  const line = other.getByTestId('workspace-rail').locator('[data-line="N16"]')
  await expect(line).toHaveAttribute('data-anchored', 'true')
  await expect(line).toBeInViewport()
})

test('WP-24: Datei öffnen, ändern, speichern und den Diff ansehen', async ({ daemon, page }) => {
  const cwd = daemon.workspace({ 'src/math.ts': 'export function add(a: number, b: number) {\n  return a + b\n}\n', 'README.md': '# Mathe\n' }, true)
  const id = await daemon.session('turns: []', 'Speichern', cwd)
  await daemon.login(page, `/s/${id}`)
  await expect(rail(page).getByText('Keine nicht committeten Änderungen')).toBeVisible()
  await tab(page, 'Dateien').click()
  // Über „Datei öffnen ⌘P“.
  await rail(page).getByLabel('Datei öffnen').fill('math')
  await rail(page).getByRole('option', { name: 'src/math.ts' }).click()
  await typeInEditor(page, '\nexport const zwei = add(1, 1)')
  await page.keyboard.press(`${await mod(page)}+s`)
  await expect(page.getByTestId('save-state')).toHaveText('Gespeichert')
  expect(readFileSync(join(cwd, 'src/math.ts'), 'utf8')).toContain('export const zwei = add(1, 1)')
  // Speichern erzeugt `fs.changed` mit dem User als Actor; die Datei ist im Baum markiert.
  await rail(page).getByRole('treeitem', { name: 'src' }).click()
  await expect(rail(page).getByRole('treeitem', { name: /math\.ts/ })).toContainText('M')

  await tab(page, 'Änderungen').click()
  await expect(rail(page).getByRole('tab', { name: 'Nicht committet' })).toHaveAttribute('aria-selected', 'true')
  await expect(rail(page).getByTestId('changed-file')).toContainText(['src/math.ts'])
  await expect(rail(page).getByTestId('diff-list')).toContainText('export const zwei = add(1, 1)')

  // Zeilen markieren → „An Agent anhängen“ landet als Referenz im Composer.
  await tab(page, 'Dateien').click()
  await page.getByTestId('code-editor').locator('.view-lines').click()
  await page.keyboard.press(`${await mod(page)}+Home`)
  await page.keyboard.press('Shift+ArrowDown')
  await page.keyboard.press('Shift+ArrowDown')
  await page.getByRole('button', { name: 'An Agent anhängen' }).click()
  await expect(page.getByTestId('attachments')).toContainText('src/math.ts')
  await expect(page.getByTestId('attachments')).toContainText('Z. 1–2')
})
