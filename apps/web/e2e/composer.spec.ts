import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Page } from '@playwright/test'
import { expect, test, type Daemon } from './fixtures'

/** Session in einem eigenen Workspace (eigener Dateiindex). */
async function sessionIn(daemon: Daemon, cwd: string, title: string): Promise<string> {
  const scenario = daemon.scenario(`w${Date.now()}`, 'turns: []\n')
  const s = await daemon.api<{ id: string }>('POST', '/v1/sessions', { target: 'fake', cwd, title, harness_opts: { scenario } })
  await daemon.waitStatus(s.id, 'idle')
  return s.id
}

/** Optional Screenshots für den PR-Bericht (`BETON_E2E_SHOTS=<verzeichnis>`). */
async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.BETON_E2E_SHOTS
  if (dir) await page.screenshot({ path: join(dir, `${name}.png`) })
}

const ECHO_IMAGES = 'capabilities: { images: true }\nturns:\n  - emit:\n      - { echo_input: true }\n'

test('WEB-006 AC1: Ein eingefügter Screenshot erscheint als Vorschau und wird als Blob mit dem Input gesendet', async ({ daemon, page }) => {
  const id = await daemon.session(ECHO_IMAGES, 'Anhänge')
  await daemon.login(page, `/s/${id}`)
  const box = page.getByLabel('Nachricht')
  await box.fill('Der Fehler aus dem Screenshot taucht nur hinter dem Load-Balancer auf.')
  // Einfügen wie aus der Zwischenablage: ein Bild ohne eigenen Dateinamen.
  await box.evaluate(async (el) => {
    const canvas = document.createElement('canvas')
    canvas.width = 320
    canvas.height = 200
    const g = canvas.getContext('2d')!
    g.fillStyle = '#d6d6d0'
    g.fillRect(0, 0, 320, 200)
    g.fillStyle = '#24272b'
    g.fillRect(16, 16, 200, 14)
    g.fillStyle = '#b5483c'
    g.fillRect(16, 44, 140, 14)
    const blob = await new Promise<Blob>((r) => canvas.toBlob((b) => r(b!), 'image/png'))
    const dt = new DataTransfer()
    dt.items.add(new File([blob], 'image.png', { type: 'image/png' }))
    el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }))
  })
  await expect(page.getByAltText(/^Vorschau Bildschirmfoto/)).toBeVisible()
  await expect(page.getByTestId('attachment')).toHaveAttribute('data-state', 'ready')
  await expect(page.getByTestId('attachment-strip')).toContainText('Bildschirmfoto ·')
  await shot(page, 'composer-anhang')
  await box.press('Enter')

  // Im Verlauf: Vorschau aus dem Blob der Session; beim Harness angekommen.
  const sent = page.getByTestId('sent-attachment')
  await expect(sent.getByRole('img')).toBeVisible()
  await expect(page.getByTestId('agent-message')).toContainText(/\[Anhang: Bildschirmfoto .*\.png, image\/png, \d+ Bytes\]/)
  await expect(page.getByTestId('attachment-strip')).toHaveCount(0)
  const events = await daemon.events(id)
  const user = events.find((e) => e.type === 'message.completed' && e.payload?.role === 'user')
  const content = user?.payload?.content as Array<Record<string, unknown>>
  expect(content[1]).toMatchObject({ type: 'attachment', mime: 'image/png' })
  expect(String(content[1]?.blob)).toMatch(/^sha256:[0-9a-f]{64}$/)
})

test('WEB-006 AC2: @mid listet src/middleware.rs und fügt die Referenz ein', async ({ daemon, page }) => {
  const repo = mkdtempSync(join(tmpdir(), 'beton-e2e-repo-'))
  mkdirSync(join(repo, 'src'), { recursive: true })
  for (const f of ['middleware.rs', 'main.rs', 'lib.rs']) writeFileSync(join(repo, 'src', f), '// test\n')
  mkdirSync(join(repo, 'docs', 'adr'), { recursive: true })
  writeFileSync(join(repo, 'docs', 'adr', '0007-middleware-order.md'), '# ADR\n')
  const id = await sessionIn(daemon, repo, 'Erwähnungen')
  await daemon.login(page, `/s/${id}`)
  const box = page.getByLabel('Nachricht')
  await box.click()
  await box.pressSequentially('Wende das Muster aus @mid')
  const menu = page.getByRole('listbox', { name: 'Dateien im Workspace' })
  await expect(menu.getByRole('option').first()).toContainText('src/middleware.rs')
  await expect(menu).toContainText('docs/adr/0007-middleware-order.md')
  await expect(menu).toContainText(/Dateien im Workspace · \d+ von \d+ · \d+ ms/)
  await shot(page, 'composer-mention')
  await box.press('Enter')
  await expect(box).toHaveValue('Wende das Muster aus @src/middleware.rs ')
  await expect(menu).toHaveCount(0)
})

test('WEB-006 AC3: Das Slash-Menü zeigt Skills mit Beschreibung', async ({ daemon, page }) => {
  const skill = join(daemon.work, '.beton', 'skills', 'review')
  mkdirSync(skill, { recursive: true })
  writeFileSync(
    join(skill, 'SKILL.md'),
    '---\nname: review\ndescription: Prüft die Änderungen dieser Session wie ein Reviewer und listet Funde nach Schwere\n---\n# Review\n',
  )
  const id = await daemon.session('turns: []\n', 'Slash-Menü')
  await daemon.login(page, `/s/${id}`)
  const box = page.getByLabel('Nachricht')
  await box.click()
  await box.press('/')
  const menu = page.getByRole('listbox', { name: 'Befehle und Skills' })
  await expect(menu).toContainText('Skills')
  const review = menu.getByRole('option', { name: /^review / })
  await expect(review).toContainText('Prüft die Änderungen dieser Session wie ein Reviewer')
  await expect(menu.getByRole('option', { name: /Forken/ })).toContainText('/fork')
  await shot(page, 'composer-slash')
  // Präfix-Treffer zuerst; ⏎ übernimmt den aktiven Eintrag.
  await box.pressSequentially('review')
  await expect(menu.getByRole('option').first()).toContainText('review')
  await box.press('Enter')
  await expect(box).toHaveValue('/review ')
})

test('WEB-006: Zu große Dateien lehnt der Composer mit Grund ab', async ({ daemon, page }) => {
  const id = await daemon.session(ECHO_IMAGES, 'Grenzen')
  await daemon.login(page, `/s/${id}`)
  await page.getByTestId('attach-input').setInputFiles({
    name: 'login-trace.har',
    mimeType: 'text/plain',
    buffer: Buffer.alloc(21 * 1024 * 1024, 120),
  })
  await expect(page.getByRole('alert')).toContainText('login-trace.har nicht angehängt: 21 MB, erlaubt sind 20 MB pro Datei.')
})
