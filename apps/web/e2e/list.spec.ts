import type { Locator } from '@playwright/test'
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
  // Aufwärmen (nicht gewertet): Layout, Virtualisierung und JIT fallen nicht in die Messung.
  await frames(list, 1000)
  // Drei Messrunden; gewertet wird die mittlere (nach Zahl der Frames). Unter fremder Last
  // fallen einzelne Runden ab (#261: 43 statt > 100 Frames); ein echter Einbruch beim Scrollen
  // trifft jede Runde. Die Kriterien bleiben: Median ≥ 55 fps, mehr als 100 Frames in 3 s,
  // virtualisiert (wenige Zeilen im DOM, nach jeder Runde geprüft).
  const rounds = []
  for (let i = 0; i < 3; i++) {
    rounds.push(await frames(list, 3000))
    expect(await page.locator('[data-session]').count()).toBeLessThan(200)
  }
  const fps = [...rounds].sort((a, b) => a.frames - b.frames)[1]!
  const shown = (r: { median: number; frames: number }) => ({ fpsMedian: Math.round(r.median), frames: r.frames })
  test.info().annotations.push({ type: 'Messwerte', description: JSON.stringify({ gewertet: shown(fps), runden: rounds.map(shown) }) })
  console.log('WEB-003 Messwerte', test.info().annotations.at(-1)?.description)
  expect(fps.frames).toBeGreaterThan(100)
  expect(fps.median).toBeGreaterThanOrEqual(55)
})

/** Scrollt `list` für `ms` Millisekunden je Frame um 120 px und misst die Frame-Abstände. */
async function frames(list: Locator, ms: number) {
  return list.evaluate(
    (el, duration) =>
      new Promise<{ median: number; frames: number }>((resolve) => {
        const times: number[] = []
        let last = performance.now()
        const end = last + duration
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
    ms,
  )
}
