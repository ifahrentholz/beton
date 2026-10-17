import AxeBuilder from '@axe-core/playwright'
import type { Page } from '@playwright/test'
import { expect, PUSH_ASK, test } from './fixtures'

// Bedienung (WP-62): Befehlspalette (UX-002), Session-Switcher (UX-003), Tastenkürzel (UX-004),
// Themes (UX-005) und Sprache (WEB-017). Kernflüsse für Palette und Themes laut QA-007.

const palette = (page: Page) => page.getByTestId('command-palette')
const switcher = (page: Page) => page.getByTestId('session-switcher')

/** Ein Turn, der eine Nachricht schreibt. */
const say = (input: string, text: string) => `  - expect_input: ${JSON.stringify(input)}\n    emit:\n      - { message: ${JSON.stringify(text)} }\n`

async function serious(page: Page, selector: string): Promise<string[]> {
  const res = await new AxeBuilder({ page }).include(selector).analyze()
  return res.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical').map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(' ')).join(', ')})`)
}

/** Merkt sich das Fenster: Nach einem Neuladen wäre die Marke weg. */
const mark = (page: Page) => page.evaluate(() => ((window as unknown as { __noReload: boolean }).__noReload = true))
const notReloaded = (page: Page) => page.evaluate(() => (window as unknown as { __noReload?: boolean }).__noReload === true)
/** Fokus aus Eingabefeldern nehmen, damit seitenweite Kürzel greifen. */
const blur = (page: Page) => page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())

test('UX-002 AC1: „fork“ zeigt „Session forken“ als ersten Treffer; Enter führt den Fork aus @kernfluss', async ({ daemon, page }) => {
  const id = await daemon.session(`turns:\n${say('Baue einen Rate-Limiter', 'Rate-Limiter steht.')}`, 'Palette-Fork')
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'Baue einen Rate-Limiter' })
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'turn.completed')).toBe(true)
  await daemon.waitStatus(id, 'idle')
  await daemon.login(page, `/s/${id}`)
  await expect(page.getByText('Rate-Limiter steht.')).toBeVisible()
  // ⌘K wirkt auch im Eingabefeld (UX-004 AC3, `global_in_input`).
  await page.getByLabel('Nachricht').focus()
  await page.keyboard.press('ControlOrMeta+k')
  await expect(palette(page)).toBeVisible()
  await expect(page.getByRole('combobox', { name: 'Befehlspalette' })).toBeFocused()
  await page.keyboard.type('fork')
  const first = palette(page).getByRole('option').first()
  await expect(first).toContainText('Session forken')
  await expect(first).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Enter')
  await expect(palette(page)).toHaveCount(0)
  await expect(page).not.toHaveURL(new RegExp(`/s/${id}$`))
  await expect(page.getByRole('note', { name: 'Herkunft des Forks' })).toContainText('Palette-Fork')
  const fork = page.url().split('/s/')[1] ?? ''
  await expect.poll(async () => (await daemon.events(fork)).find((e) => e.type === 'session.forked')?.payload).toMatchObject({ from_session: id })
})

test('UX-002 AC2: „Unterbrechen“ erscheint nur bei laufendem Turn', async ({ daemon, page }) => {
  const id = await daemon.session('turns:\n  - expect_input: "lang"\n    emit: [{ message: "Ich arbeite." }, { hang: true }]\n', 'Lang')
  await daemon.login(page, `/s/${id}`)
  await expect(page.getByLabel('Nachricht')).toBeEditable()
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type('unterbr')
  await expect(palette(page).getByRole('option', { name: /Unterbrechen/ })).toHaveCount(0)
  await page.keyboard.press('Escape')
  await page.getByLabel('Nachricht').fill('lang')
  await page.getByLabel('Nachricht').press('Enter')
  await expect(page.getByRole('button', { name: 'Unterbrechen' })).toBeVisible()
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type('unterbr')
  await expect(palette(page).getByRole('option').first()).toContainText('Unterbrechen')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'turn.interrupted')).toBe(true)
})

test('UX-002 AC3: Palette vollständig per Tastatur, ohne kritische axe-Verstöße @kernfluss', async ({ daemon, page }) => {
  const id = await daemon.session('turns: []', 'Tastatur-Palette')
  await daemon.login(page, `/s/${id}`)
  await expect(page.getByLabel('Nachricht')).toBeEditable()
  await page.keyboard.press('ControlOrMeta+k')
  await expect(palette(page)).toBeVisible()
  expect(await serious(page, '[data-testid="command-palette"]')).toEqual([])
  // Pfeile wählen (mit Umlauf), Enter führt aus.
  const input = page.getByRole('combobox', { name: 'Befehlspalette' })
  const active = () => input.getAttribute('aria-activedescendant')
  const before = await active()
  await page.keyboard.press('ArrowDown')
  expect(await active()).not.toBe(before)
  await page.keyboard.press('ArrowUp')
  expect(await active()).toBe(before)
  // Tab bleibt in der Palette, Esc schließt und gibt den Fokus zurück.
  await page.keyboard.press('Tab')
  await expect(input).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(palette(page)).toHaveCount(0)
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type('inbox')
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/\/inbox$/)
})

test('UX-003 AC1 AC2 AC3: Switcher mit MRU, markierten Freigaben, Wechsel < 150 ms aus dem Cache, Reihenfolge übersteht Neuladen', async ({ daemon, page }) => {
  const a = await daemon.session(`turns:\n${say('eins', 'Antwort A')}${say('zwei', 'Live nach dem Wechsel')}`, 'Session A')
  await daemon.api('POST', `/v1/sessions/${a}/input`, { text: 'eins' })
  await expect.poll(async () => (await daemon.events(a)).some((e) => e.type === 'turn.completed')).toBe(true)
  await daemon.waitStatus(a, 'idle')
  const b = await daemon.session('turns: []', 'Session B')
  const waiting = await daemon.session(PUSH_ASK, 'Wartet auf Freigabe')
  await daemon.api('POST', `/v1/sessions/${waiting}/input`, { text: 'Bitte pushen' })
  await daemon.waitStatus(waiting, 'waiting_approval')

  await daemon.login(page, `/s/${a}`)
  await expect(page.getByText('Antwort A')).toBeVisible()
  await page.locator(`[data-session="${b}"]`).first().click()
  await expect(page.getByRole('heading', { name: 'Session B' })).toBeVisible()

  await page.keyboard.press('ControlOrMeta+Alt+s')
  await expect(switcher(page)).toBeVisible()
  // AC2: Session mit offener Freigabe ist markiert.
  const row = (id: string) => switcher(page).locator(`[data-session="${id}"]`)
  await expect(row(waiting)).toHaveAttribute('data-waiting', 'approval')
  await expect(row(waiting)).toContainText('Freigabe offen')
  // MRU: B (aktuell) oben, A vorausgewählt.
  const ids = await switcher(page).getByRole('option').evaluateAll((els) => els.map((e) => e.getAttribute('data-session')))
  expect(ids.slice(0, 2)).toEqual([b, a])
  await expect(row(a)).toHaveAttribute('aria-selected', 'true')
  expect(await serious(page, '[data-testid="session-switcher"]')).toEqual([])

  // AC1: Enter bis zum letzten Stand von A, im Browser gemessen.
  const ms = await page.evaluate(
    ({ title }) =>
      new Promise<number>((resolve) => {
        const ready = () => document.querySelector('h2')?.textContent === title && !!document.querySelector('[data-testid="agent-message"]')
        const start = performance.now()
        const observer = new MutationObserver(() => {
          if (ready()) {
            observer.disconnect()
            resolve(performance.now() - start)
          }
        })
        observer.observe(document.body, { childList: true, subtree: true, characterData: true })
        const input = document.querySelector<HTMLInputElement>('[data-testid="session-switcher"] input')!
        input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
      }),
    { title: 'Session A' },
  )
  test.info().annotations.push({ type: 'Messwerte', description: `Wechsel aus dem Cache: ${Math.round(ms)} ms` })
  expect(ms).toBeLessThan(150)
  await expect(page).toHaveURL(new RegExp(`/s/${a}$`))
  // Live-Events folgen.
  await daemon.api('POST', `/v1/sessions/${a}/input`, { text: 'zwei' })
  await expect(page.getByText('Live nach dem Wechsel')).toBeVisible()

  // AC3: Reihenfolge je Gerät, übersteht Neuladen.
  await page.reload()
  await expect(page.getByText('Antwort A')).toBeVisible()
  await page.keyboard.press('ControlOrMeta+Alt+s')
  const after = await switcher(page).getByRole('option').evaluateAll((els) => els.map((e) => e.getAttribute('data-session')))
  expect(after.slice(0, 2)).toEqual([a, b])
  // Pfeil und Enter wechseln per Tastatur.
  await expect(row(b)).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(new RegExp(`/s/${b}$`))
})

test('UX-004 AC1 AC2: ⌘/ zeigt alle Kürzel; Umbelegen mit Konflikt fragt und tauscht, die Belegung bleibt nach Neuladen', async ({ daemon, page }) => {
  const id = await daemon.session('turns: []', 'Kürzel')
  await daemon.login(page, `/s/${id}`)
  await expect(page.getByLabel('Nachricht')).toBeEditable()
  await blur(page)
  await page.keyboard.press('ControlOrMeta+/')
  const overlay = page.getByTestId('shortcuts-overlay')
  await expect(overlay.getByRole('heading', { name: 'Tastenkürzel' })).toBeVisible()
  // Gruppen und Einträge wie im Entwurf `inbox-shortcuts`.
  for (const group of ['Allgemein', 'Session', 'Freigaben', 'Inbox']) await expect(overlay.getByRole('group', { name: group })).toBeVisible()
  for (const id of ['palette.open', 'session.new', 'switcher.open', 'goto.inbox', 'goto.settings', 'shortcuts.open', 'session.send', 'session.interrupt', 'session.model', 'session.fork', 'session.rename', 'approval.allow', 'approval.allowSession', 'approval.deny', 'approval.focus', 'approval.step', 'approval.back', 'inbox.step', 'inbox.allow', 'inbox.deny', 'inbox.open']) {
    await expect(overlay.locator(`[data-shortcut="${id}"]`)).toHaveCount(1)
  }
  expect(await serious(page, '[data-testid="shortcuts-overlay"]')).toEqual([])
  // Suche filtert.
  await page.keyboard.type('forken')
  await expect(overlay.locator('[data-shortcut]')).toHaveCount(1)
  await page.getByLabel('Kürzel suchen').fill('')

  // „Modell wechseln“ auf ⌘K legen: Konflikt mit der Befehlspalette, tauschen.
  await overlay.getByRole('button', { name: '„Modell wechseln“ ändern' }).focus()
  await page.keyboard.press('Enter')
  await expect(overlay.getByTestId('shortcut-recording')).toBeVisible()
  await page.keyboard.press('ControlOrMeta+k')
  const conflict = overlay.getByTestId('shortcut-conflict')
  await expect(conflict).toContainText('gehört schon zu „Befehlspalette“')
  await expect(conflict.getByRole('button', { name: 'Tauschen' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(conflict).toHaveCount(0)
  await page.keyboard.press('Escape')
  await expect(overlay).toHaveCount(0)

  // Die Palette liegt jetzt auf ⌘⇧M, auch nach Neuladen.
  await page.reload()
  await expect(page.getByLabel('Nachricht')).toBeEditable()
  await page.keyboard.press('ControlOrMeta+k')
  await expect(palette(page)).toHaveCount(0)
  await page.keyboard.press('ControlOrMeta+Shift+m')
  await expect(palette(page)).toBeVisible()
})

test('UX-004 AC3: Kürzel greifen nicht im Eingabefeld, außer ⌘K', async ({ daemon, page }) => {
  const id = await daemon.session('turns: []', 'Eingabefeld')
  await daemon.login(page, `/s/${id}`)
  const box = page.getByLabel('Nachricht')
  await box.focus()
  await page.keyboard.press('ControlOrMeta+Shift+i')
  await page.keyboard.press('ControlOrMeta+/')
  await expect(page).toHaveURL(new RegExp(`/s/${id}$`))
  await expect(page.getByTestId('shortcuts-overlay')).toHaveCount(0)
  await page.keyboard.press('ControlOrMeta+k')
  await expect(palette(page)).toBeVisible()
  await page.keyboard.press('Escape')
  // Außerhalb des Felds wirkt ⌘⇧I.
  await blur(page)
  await page.keyboard.press('ControlOrMeta+Shift+i')
  await expect(page).toHaveURL(/\/inbox$/)
})

test('UX-005 AC1: Theme-Wechsel wirkt ohne Neuladen auf UI, Monaco und Code-Blöcke @kernfluss', async ({ daemon, page }) => {
  await page.setViewportSize({ width: 1600, height: 900 })
  await page.emulateMedia({ colorScheme: 'light' })
  const code = '```ts\nconst answer: number = 42\nexport function f() {\n  return answer\n}\n```'
  const cwd = daemon.workspace({ 'app.ts': 'export const answer = 42\n' })
  const id = await daemon.session(`turns:\n${say('code', `Hier:\n\n${code}`)}`, 'Themes', cwd)
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'code' })
  await expect.poll(async () => (await daemon.events(id)).some((e) => e.type === 'turn.completed')).toBe(true)
  await daemon.login(page, `/s/${id}`)
  const token = page.locator('.md pre code span span').filter({ hasText: 'const' }).first()
  await expect(token).toBeVisible()
  // Monaco mit einer Datei öffnen.
  const rail = page.getByTestId('workspace-rail')
  await rail.getByRole('tab', { name: /^Dateien/ }).click()
  await rail.getByRole('treeitem', { name: 'app.ts' }).click()
  const editorBg = page.locator('[data-testid="code-editor"] .monaco-editor-background')
  await expect(editorBg).toBeVisible()
  const colors = async () => ({
    page: await page.evaluate(() => getComputedStyle(document.body).backgroundColor),
    code: await token.evaluate((el) => getComputedStyle(el).color),
    editor: await editorBg.evaluate((el) => getComputedStyle(el).backgroundColor),
  })
  const light = await colors()
  await mark(page)

  // Über die Palette: „Theme wechseln“ (hell → dunkel).
  await page.keyboard.press('ControlOrMeta+k')
  await page.keyboard.type('theme')
  await page.keyboard.press('Enter')
  await expect(page.locator('html')).toHaveClass(/\bdark\b/)
  await expect.poll(colors).not.toEqual(light)
  const dark = await colors()
  expect(dark.page).not.toBe(light.page)
  expect(dark.code).not.toBe(light.code)
  expect(dark.editor).not.toBe(light.editor)

  // Hoher Kontrast in den Einstellungen; Editor und Code folgen ebenfalls.
  await page.goto(`${daemon.url}/settings/appearance`)
  await page.getByRole('radio', { name: /Hoher Kontrast/ }).click()
  await expect(page.locator('html')).toHaveClass(/theme-hc/)
  await page.goBack()
  await expect(token).toBeVisible()
  await expect.poll(() => token.evaluate((el) => getComputedStyle(el).color)).not.toBe(dark.code)
  expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe('rgb(255, 255, 255)')
})

test('UX-005 AC3: „System“ folgt einer Änderung der OS-Einstellung live', async ({ daemon, page }) => {
  await page.emulateMedia({ colorScheme: 'light' })
  await daemon.login(page, '/settings/appearance')
  await expect(page.getByRole('radio', { name: /^System/ })).toHaveAttribute('aria-checked', 'true')
  await expect(page.locator('html')).not.toHaveClass(/\bdark\b/)
  await mark(page)
  await page.emulateMedia({ colorScheme: 'dark' })
  await expect(page.locator('html')).toHaveClass(/\bdark\b/)
  await page.emulateMedia({ colorScheme: 'light' })
  await expect(page.locator('html')).toHaveClass(/\blight\b/)
  expect(await notReloaded(page)).toBe(true)
})

test('WEB-017 AC2: Umschalten der Sprache wirkt ohne Neuladen; ohne Wahl gilt die Browsersprache', async ({ daemon, page, browser }) => {
  const id = await daemon.session('turns: []', 'Sprache')
  await daemon.login(page, '/settings/appearance')
  await expect(page.getByRole('heading', { name: 'Darstellung', level: 1 })).toBeVisible()
  await mark(page)
  await page.getByRole('radio', { name: 'English' }).click()
  await expect(page.getByRole('heading', { name: 'Appearance', level: 1 })).toBeVisible()
  await expect(page.getByRole('link', { name: 'Settings' })).toBeVisible()
  await expect(page.locator('html')).toHaveAttribute('lang', 'en')
  await expect(page.getByTestId('format-sample')).toContainText('$4.82')
  expect(await notReloaded(page)).toBe(true)
  // Die Palette spricht dieselbe Sprache.
  await blur(page)
  await page.keyboard.press('ControlOrMeta+k')
  await expect(page.getByRole('combobox', { name: 'Command palette' })).toBeVisible()
  await page.keyboard.press('Escape')
  await page.getByRole('radio', { name: 'Deutsch' }).click()
  await expect(page.getByRole('heading', { name: 'Darstellung', level: 1 })).toBeVisible()

  // Englischer Browser ohne eigene Wahl: Englisch.
  const ctx = await browser.newContext({ locale: 'en-US' })
  const en = await ctx.newPage()
  await daemon.login(en, `/s/${id}`)
  await expect(en.getByRole('navigation', { name: 'Sessions' }).getByPlaceholder('Search sessions')).toBeVisible()
  await ctx.close()
})
