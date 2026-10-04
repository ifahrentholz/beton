import { expect, PUSH_ASK, test } from './fixtures'

test('WEB-003 AC1: Statuswechsel aktualisiert den Indikator ohne Reload binnen 1 s', async ({ daemon, page }) => {
  const other = await daemon.session('turns: []', 'Andere')
  const watched = await daemon.session(PUSH_ASK, 'Beobachtet')
  await daemon.login(page, `/s/${other}`)
  const row = page.locator(`[data-session="${watched}"]`)
  await expect(row.locator('[data-status="idle"]')).toBeVisible()
  await daemon.api('POST', `/v1/sessions/${watched}/input`, { text: 'Bitte pushen' })
  await daemon.waitStatus(watched, 'waiting_approval')
  const changed = Date.now()
  await expect(row.locator('[data-status="waiting"]')).toBeVisible({ timeout: 1000 })
  expect(Date.now() - changed).toBeLessThanOrEqual(1000)
})

test('WEB-003 AC2: 5 000 Sessions bleiben flüssig scrollbar (≥ 55 fps)', async ({ daemon, page }) => {
  const total = 5000
  const now = Date.now()
  const sessions = Array.from({ length: total }, (_, i) => ({
    id: `ses_${String(total - i).padStart(26, '0')}`,
    title: `Session ${total - i}: Refactoring im Modul ${i % 97}`,
    status: ['idle', 'running', 'waiting_approval', 'stopped', 'failed'][i % 5],
    kind: 'main',
    harness: i % 3 === 0 ? 'codex' : 'claude',
    archived: false,
    head_seq: 10,
    cost_micro: 0,
    created_at: new Date(now - i * 60_000).toISOString(),
    last_activity_at: new Date(now - i * 60_000).toISOString(),
  }))
  await page.route('**/v1/sessions?*', async (route) => {
    const url = new URL(route.request().url())
    if (url.searchParams.get('updated_after')) return route.fulfill({ json: { items: [], next_cursor: null } })
    const start = Number(url.searchParams.get('cursor') ?? 0)
    const limit = Number(url.searchParams.get('limit') ?? 50)
    const end = Math.min(start + limit, total)
    return route.fulfill({ json: { items: sessions.slice(start, end), next_cursor: end < total ? String(end) : null } })
  })
  await daemon.login(page, '/')
  const list = page.getByTestId('session-list')
  await expect(page.locator('[data-session]').first()).toBeVisible()
  await expect.poll(() => list.evaluate((el) => el.scrollHeight)).toBeGreaterThan(total * 40)
  const fps = await list.evaluate(
    (el) =>
      new Promise<{ median: number; frames: number }>((resolve) => {
        const times: number[] = []
        let last = performance.now()
        const end = last + 3000
        const step = (t: number) => {
          times.push(t - last)
          last = t
          el.scrollTop = (el.scrollTop + 120) % (el.scrollHeight - el.clientHeight)
          if (t < end) requestAnimationFrame(step)
          else {
            const sorted = times.slice(1).sort((a, b) => a - b)
            resolve({ median: 1000 / sorted[Math.floor(sorted.length / 2)]!, frames: sorted.length })
          }
        }
        requestAnimationFrame(step)
      }),
  )
  expect(fps.frames).toBeGreaterThan(100)
  expect(fps.median).toBeGreaterThanOrEqual(55)
})
