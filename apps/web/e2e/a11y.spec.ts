import AxeBuilder from '@axe-core/playwright'
import type { Page } from '@playwright/test'
import { expect, test } from './fixtures'

// Barrierefreiheit der Kernflüsse (WEB-016): Session öffnen, Nachricht senden, Freigabe
// entscheiden, Diff lesen – in allen Themes ohne `serious`/`critical`-Verstöße und ohne Maus.

const THEMES: { theme: string; scheme: 'light' | 'dark' }[] = [
  { theme: 'system', scheme: 'light' },
  { theme: 'system', scheme: 'dark' },
  { theme: 'nord', scheme: 'light' },
  { theme: 'nord', scheme: 'dark' },
  { theme: 'hc', scheme: 'light' },
  { theme: 'hc', scheme: 'dark' },
]

/** Ein Turn schreibt eine Datei (Diff) und fragt danach nach einer Freigabe. */
const SCENARIO = `turns:
  - expect_input: "schreiben"
    emit:
      - { message_delta: "Ich lege die Datei an.\\n\\nDer Rate-Limiter begrenzt auf 5 Versuche pro Minute:\\n\\n\`\`\`ts\\nexport const LIMIT = 5\\nexport function allowed(n: number) {\\n  return n < LIMIT\\n}\\n\`\`\`", chunk: 12, delay_ms: 5 }
      - { write_file: { path: "src/limit.ts", content: "export const LIMIT = 5\\nexport function allowed(n: number) {\\n  return n < LIMIT\\n}\\n" } }
      - { message: "Datei angelegt." }
      - { tool_call: { name: Bash, kind: shell, args: { command: "git push origin main" } }, gate: true }
      - { on_gate: { allow: [{ tool_result: "ok" }, { message: "Push erledigt." }], deny: [{ message: "Push abgelehnt." }] } }
`

async function violations(page: Page): Promise<string[]> {
  const res = await new AxeBuilder({ page }).analyze()
  return res.violations
    .filter((v) => v.impact === 'serious' || v.impact === 'critical')
    .map((v) => `${v.id}: ${v.help} – ${v.nodes.slice(0, 3).map((n) => n.target.join(' ')).join(', ')}`)
}

async function setTheme(page: Page, theme: string, scheme: 'light' | 'dark') {
  await page.emulateMedia({ colorScheme: scheme })
  await page.evaluate((t) => localStorage.setItem('beton.appearance.v1', JSON.stringify({ theme: t })), theme)
  await page.reload()
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 })
})

test('WEB-016 AC1: axe ohne serious/critical in den Kernflüssen, in allen Themes', async ({ daemon, page }) => {
  test.setTimeout(180_000)
  const id = await daemon.session(SCENARIO, 'Barrierefrei', daemon.workspace({}))
  await daemon.api('POST', `/v1/sessions/${id}/input`, { text: 'schreiben' })
  await daemon.waitStatus(id, 'waiting_approval')
  await daemon.login(page, '/')
  const found: string[] = []
  for (const { theme, scheme } of THEMES) {
    const label = `${theme}/${scheme}`
    // Session öffnen: Startseite mit Session-Liste.
    await page.goto(`${daemon.url}/`)
    await setTheme(page, theme, scheme)
    await expect(page.locator(`[data-session="${id}"]`)).toBeVisible()
    for (const v of await violations(page)) found.push(`${label} Start: ${v}`)
    // Nachricht senden, Freigabe entscheiden, Diff lesen: die Session mit offener Freigabe und Diff.
    await page.locator(`[data-session="${id}"]`).click()
    await expect(page.getByRole('alertdialog')).toBeVisible()
    await expect(page.getByTestId('workspace-rail').getByTestId('diff-list')).toContainText('LIMIT')
    await expect(page.getByTestId('agent-message').first()).toBeVisible()
    for (const v of await violations(page)) found.push(`${label} Session: ${v}`)
  }
  expect(found).toEqual([])
})

test('WEB-016 AC2: Session öffnen, Nachricht senden, Freigabe entscheiden und Diff lesen nur mit der Tastatur', async ({ daemon, page }) => {
  const id = await daemon.session(SCENARIO, 'Nur Tastatur', daemon.workspace({}))
  await daemon.login(page, '/')
  await expect(page.locator(`[data-session="${id}"]`)).toBeVisible()

  // Session öffnen: Switcher, Suche, Enter.
  await page.keyboard.press('ControlOrMeta+Alt+s')
  await page.keyboard.type('Nur Tastatur')
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(new RegExp(`/s/${id}$`))

  // Nachricht senden: Tab bis ins Eingabefeld, tippen, Enter.
  const box = page.getByLabel('Nachricht')
  for (let i = 0; i < 40 && !(await box.evaluate((el) => el === document.activeElement)); i++) await page.keyboard.press('Tab')
  await expect(box).toBeFocused()
  await page.keyboard.type('schreiben')
  await page.keyboard.press('Enter')
  await expect(page.getByText('Datei angelegt.')).toBeVisible()

  // Freigabe entscheiden: ⌘⏎ aus dem Eingabefeld.
  await expect(page.getByRole('alertdialog')).toBeVisible()
  await page.waitForTimeout(500)
  await page.keyboard.press('ControlOrMeta+Enter')
  await expect(page.getByText('Push erledigt.')).toBeVisible()

  // Diff lesen: Tab „Änderungen“ (Alt+2), dann per Tab in die Diff-Liste und mit Pfeilen scrollen.
  await page.keyboard.press('Alt+2')
  const rail = page.getByTestId('workspace-rail')
  await expect(rail.getByRole('tab', { name: /^Änderungen/ })).toHaveAttribute('aria-selected', 'true')
  const diff = rail.getByTestId('diff-list')
  await expect(diff).toContainText('LIMIT')
  for (let i = 0; i < 40 && !(await diff.evaluate((el) => el.contains(document.activeElement))); i++) await page.keyboard.press('Tab')
  expect(await diff.evaluate((el) => el.contains(document.activeElement))).toBe(true)
  await page.keyboard.press('ArrowDown')
  await expect(diff).toContainText('return n < LIMIT')
})
