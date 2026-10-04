import { createServer, type Server } from 'node:http'
import { expect, test } from './fixtures'

/**
 * AUTH-002 AC4: Eine fremde Seite (`http://evil.localhost:<x>`) kann weder per `fetch` noch
 * per Formular eine Session im lokalen Daemon starten, auch nicht, wenn derselbe Browser
 * angemeldet ist. Variante DNS-Rebinding: dieselbe Seite unter dem Port des Daemons.
 */
test('AUTH-002 AC4: evil.localhost kann keine Session starten', async ({ daemon, page, browserName }) => {
  test.skip(browserName !== 'chromium', '*.localhost löst Chromium zuverlässig auf Loopback auf')
  const before = (await daemon.api<{ items: unknown[] }>('GET', '/v1/sessions?limit=200')).items.length
  const target = daemon.url
  const body = JSON.stringify({ target: 'fake', cwd: '/tmp', harness_opts: { scenario: '/tmp/x.yaml' } })
  const evilHtml = `<!doctype html><title>evil</title><body>
    <form id="f" method="POST" action="${target}/v1/sessions" enctype="text/plain">
      <input name='${body.slice(0, -1)},"x":"' value='"}'>
    </form>
    <script>
      window.results = []
      fetch('${target}/v1/sessions', { method: 'POST', credentials: 'include', headers: { 'content-type': 'application/json' }, body: ${JSON.stringify(body)} })
        .then((r) => window.results.push('fetch:' + r.status), (e) => window.results.push('fetch:blocked'))
      fetch('${target}/v1/sessions', { method: 'POST', mode: 'no-cors', credentials: 'include', headers: { 'content-type': 'text/plain' }, body: ${JSON.stringify(body)} })
        .then(() => window.results.push('no-cors:sent'), () => window.results.push('no-cors:blocked'))
      setTimeout(() => document.getElementById('f').submit(), 300)
    </script></body>`
  const evil: Server = createServer((_req, res) => {
    res.writeHead(200, { 'content-type': 'text/html' })
    res.end(evilHtml)
  })
  await new Promise<void>((r) => evil.listen(0, '127.0.0.1', () => r()))
  const port = (evil.address() as { port: number }).port
  try {
    // Der Browser ist beim Daemon angemeldet (Cookie vorhanden).
    await daemon.login(page, '/')
    await expect(page.getByRole('navigation', { name: 'Sessions' })).toBeVisible()
    const attacker = await page.context().newPage()
    await attacker.goto(`http://evil.localhost:${port}/`)
    await attacker.waitForTimeout(1500)

    // DNS-Rebinding: evil.localhost auf dem Port des Daemons → Host-Allowlist greift.
    const rebind = await page.context().newPage()
    const res = await rebind.goto(`http://evil.localhost:${new URL(target).port}/`)
    expect(res?.status()).toBe(403)
    const rebindFetch = await rebind.evaluate(async (b) => {
      const r = await fetch('/v1/sessions', { method: 'POST', credentials: 'include', headers: { 'content-type': 'application/json' }, body: b })
      return r.status
    }, body)
    expect(rebindFetch).toBe(403)

    const after = (await daemon.api<{ items: unknown[] }>('GET', '/v1/sessions?limit=200')).items.length
    expect(after).toBe(before)
  } finally {
    await new Promise((r) => evil.close(r))
  }
})
