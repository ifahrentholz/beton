import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { gzipSync } from 'node:zlib'
import { expect, test } from './fixtures'

/**
 * Performance-Budgets (WEB-015 AC1). Die Streaming-Dauer ist in CI auf
 * `BETON_PERF_STREAM_SECONDS` (Standard 20 s) verkürzt; Budget und Rate bleiben gleich.
 */
const STREAM_SECONDS = Number(process.env.BETON_PERF_STREAM_SECONDS ?? 20)

async function frames(page: import('@playwright/test').Page, ms: number) {
  return page.evaluate(
    (duration) =>
      new Promise<{ median: number; slowShare: number; count: number }>((resolve) => {
        const deltas: number[] = []
        let last = performance.now()
        const end = last + duration
        const step = (t: number) => {
          deltas.push(t - last)
          last = t
          if (t < end) requestAnimationFrame(step)
          else {
            const d = deltas.slice(1)
            const sorted = [...d].sort((a, b) => a - b)
            resolve({
              median: 1000 / sorted[Math.floor(sorted.length / 2)]!,
              slowShare: d.filter((x) => x > 32).length / d.length,
              count: d.length,
            })
          }
        }
        requestAnimationFrame(step)
      }),
    ms,
  )
}

test('WEB-015 AC1: Budgets für Streaming, Session-Wechsel, Bundle, Speicher und Start', async ({ daemon, page }) => {
  test.setTimeout(180_000 + STREAM_SECONDS * 1000)

  // Initial-JS (gzip) ≤ 450 KB.
  const dist = join(import.meta.dirname, '..', 'dist')
  const html = readFileSync(join(dist, 'index.html'), 'utf8')
  const scripts = [...new Set([...html.matchAll(/(?:src|href)="\/(assets\/[^"]+\.js)"/g)].map((m) => m[1]!))]
  const initialKb = scripts.reduce((sum, s) => sum + gzipSync(readFileSync(join(dist, s))).length, 0) / 1024
  expect(initialKb).toBeLessThanOrEqual(450)

  // Session mit 10 000 Events (kalt ≤ 800 ms, gecacht ≤ 150 ms bis erster Inhalt).
  const steps = Array.from({ length: 10_000 }, (_, i) => `      - { message: "Nachricht ${i}" }`).join('\n')
  const big = await daemon.session(`turns:\n  - expect_input: "viel"\n    emit:\n${steps}\n`, 'Groß')
  await daemon.api('POST', `/v1/sessions/${big}/input`, { text: 'viel' })
  await daemon.waitStatus(big, 'idle')
  const small = await daemon.session('turns: []', 'Klein')

  // Start bis bedienbar ≤ 1,5 s.
  await daemon.login(page, `/s/${small}`)
  const t0 = Date.now()
  await page.goto(`${daemon.url}/s/${small}`)
  await expect(page.getByLabel('Nachricht')).toBeEditable()
  expect(Date.now() - t0).toBeLessThanOrEqual(1500)

  // Im Browser gemessen: Klick bis der erste Inhalt im DOM steht. Playwrights `expect`
  // pollt in Stufen (0/20/100/500 ms) und würde die Messung verfälschen.
  const open = (id: string) =>
    page.evaluate(
      ({ id, title, needsMessage }) =>
        new Promise<number>((resolve) => {
          const ready = () => {
            const h = document.querySelector('h2')
            if (h?.textContent !== title) return false
            return !needsMessage || document.querySelector('[data-testid="agent-message"]') !== null
          }
          const start = performance.now()
          const observer = new MutationObserver(() => {
            if (ready()) {
              observer.disconnect()
              resolve(performance.now() - start)
            }
          })
          observer.observe(document.body, { childList: true, subtree: true, characterData: true })
          document.querySelector<HTMLElement>(`[data-session="${id}"]`)!.click()
        }),
      { id, title: id === big ? 'Groß' : 'Klein', needsMessage: id === big },
    )
  const cold = await open(big)
  expect(cold).toBeLessThanOrEqual(800)
  await open(small)
  const cached = await open(big)
  expect(cached).toBeLessThanOrEqual(150)

  // Speicher bei 10 000 Events ≤ 300 MB.
  const heap = await page.evaluate(() => (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? 0)
  expect(heap / 1024 / 1024).toBeLessThanOrEqual(300)

  // Streaming mit 500 Tokens/s (≈ 4 Zeichen je Token): Median ≥ 58 fps, < 5 % Frames > 32 ms.
  const paragraph = 'Das ist ein Absatz mit genug Text, damit die Ausgabe wie eine echte Antwort aussieht. '
  const chars = 500 * 4 * STREAM_SECONDS
  let text = ''
  while (text.length < chars) text += paragraph + (text.length % 600 < paragraph.length ? '\\n\\n' : '')
  const streamer = await daemon.session(
    `turns:\n  - expect_input: "los"\n    emit:\n      - { message_delta: "${text}", chunk: 8, chunk_delay_ms: 4 }\n`,
    'Strom',
  )
  await page.locator(`[data-session="${streamer}"]`).click()
  await expect(page.getByRole('heading', { name: 'Strom' })).toBeVisible()
  await daemon.api('POST', `/v1/sessions/${streamer}/input`, { text: 'los' })
  await expect(page.getByTestId('agent-message').last()).toBeVisible()
  const f = await frames(page, STREAM_SECONDS * 1000 - 2000)
  test.info().annotations.push({
    type: 'Messwerte',
    description: JSON.stringify({
      initialJsKb: Math.round(initialKb),
      coldMs: cold,
      cachedMs: cached,
      heapMb: Math.round(heap / 1024 / 1024),
      streamFpsMedian: Math.round(f.median),
      slowFramesPct: Math.round(f.slowShare * 1000) / 10,
    }),
  })
  console.log('WEB-015 Messwerte', test.info().annotations.at(-1)?.description)
  expect(f.count).toBeGreaterThan(100)
  expect(f.median).toBeGreaterThanOrEqual(58)
  expect(f.slowShare).toBeLessThan(0.05)
})
