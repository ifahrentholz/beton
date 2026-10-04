import { afterEach, describe, expect, it } from 'vitest'
import { Backoff, BetonClient } from '../src/index.js'
import { MockServer } from './mock-server.js'

let server: MockServer | undefined

afterEach(async () => {
  await server?.stop()
  server = undefined
})

async function take(iter: AsyncIterator<{ seq: number; transient?: boolean }>, until: number) {
  const seqs: number[] = []
  while ((seqs.at(-1) ?? 0) < until) {
    const r = await iter.next()
    if (r.done) break
    if (!r.value.transient) seqs.push(r.value.seq)
  }
  return seqs
}

describe('Event-Strom', () => {
  it('API-004 AC2: WS-Abbruch während des Streamings wird mit Resume überbrückt', async () => {
    server = await MockServer.start()
    server.produce(5)
    const client = new BetonClient({ baseUrl: server.url, token: 'tok' })
    const iter = client
      .session('ses_x')
      .events({ fromSeq: 0, backoff: new Backoff(20, 200) })
      [Symbol.asyncIterator]()
    const first = await take(iter, 5)
    server.produce(5)
    const second = await take(iter, 10)
    // Simulierter Abbruch mitten im Strom; währenddessen entstehen weitere Events.
    server.dropAll()
    server.produce(5)
    const third = await take(iter, 15)
    await iter.return?.()
    const all = [...first, ...second, ...third]
    expect(all).toEqual(Array.from({ length: 15 }, (_, i) => i + 1))
    // Nach dem Abbruch wurde ab der zuletzt gesehenen seq neu angehängt.
    expect(server.attaches).toEqual([0, 10])
    expect(server.authHeaders.every((h) => h === 'Bearer tok')).toBe(true)
  })

  it('PROTO-009: nach Close 4503 sofort mit Jitter neu verbinden, ohne Lücke', async () => {
    server = await MockServer.start()
    server.produce(3)
    const delays: number[] = []
    const client = new BetonClient({ baseUrl: server.url, token: 'tok' })
    const iter = client
      .session('ses_x')
      .events({ onReconnecting: (_a, d) => delays.push(d) })
      [Symbol.asyncIterator]()
    await take(iter, 3)
    server.closeAll(4503)
    server.produce(2)
    const rest = await take(iter, 5)
    await iter.return?.()
    expect(rest).toEqual([4, 5])
    expect(delays).toHaveLength(1)
    expect(delays[0]).toBeLessThanOrEqual(2000)
  })

  it('fataler Close-Code beendet den Strom mit Fehler', async () => {
    server = await MockServer.start()
    const client = new BetonClient({ baseUrl: server.url, token: 'tok' })
    const iter = client.session('ses_x').events()[Symbol.asyncIterator]()
    const pending = iter.next()
    await new Promise((r) => setTimeout(r, 100))
    server.closeAll(4404)
    await expect(pending).rejects.toThrow(/4404/)
  })
})
