import { createServer, type IncomingMessage, type Server } from 'node:http'
import { type AddressInfo } from 'node:net'
import { afterEach, describe, expect, it } from 'vitest'
import { BetonClient, BetonError } from '../src/index.js'

let http: Server | undefined
afterEach(() => new Promise<void>((r) => (http ? http.close(() => r()) : r())))

function body(req: IncomingMessage): Promise<string> {
  return new Promise((resolve) => {
    let data = ''
    req.on('data', (c) => (data += c))
    req.on('end', () => resolve(data))
  })
}

async function serve(handler: (req: IncomingMessage, text: string) => [number, unknown]): Promise<string> {
  http = createServer(async (req, res) => {
    const [status, payload] = handler(req, await body(req))
    res.writeHead(status, { 'content-type': status >= 400 ? 'application/problem+json' : 'application/json' })
    res.end(payload === undefined ? '' : JSON.stringify(payload))
  })
  await new Promise<void>((r) => http!.listen(0, '127.0.0.1', () => r()))
  return `http://127.0.0.1:${(http.address() as AddressInfo).port}`
}

describe('REST-Client', () => {
  it('legt Sessions an und sendet Eingaben mit Bearer-Token', async () => {
    const seen: string[] = []
    const url = await serve((req, text) => {
      seen.push(`${req.method} ${req.url} ${req.headers.authorization} ${text}`)
      if (req.url === '/v1/sessions') return [201, { id: 'ses_1', title: '', status: 'starting' }]
      return [202, { input_id: 'i', turn_id: 't' }]
    })
    const client = new BetonClient({ baseUrl: url, token: 'tok' })
    const s = await client.sessions.create({ target: 'fake', cwd: '/tmp' })
    expect(s.id).toBe('ses_1')
    const accepted = await s.send('hallo')
    expect(accepted.turn_id).toBe('t')
    expect(seen).toEqual([
      'POST /v1/sessions Bearer tok {"target":"fake","cwd":"/tmp"}',
      'POST /v1/sessions/ses_1/input Bearer tok {"text":"hallo"}',
    ])
  })

  it('SES-006 AC1: forkt eine Session über die API', async () => {
    const seen: string[] = []
    const url = await serve((req, text) => {
      seen.push(`${req.method} ${req.url} ${text}`)
      return [201, { session: { id: 'ses_2', harness: 'codex' }, effective_seq: 120, workspace: 'shared' }]
    })
    const client = new BetonClient({ baseUrl: url, token: 'tok' })
    const forked = await client.session('ses_1').fork({ at_seq: 122, harness: 'codex', workspace: 'shared' })
    expect(forked.session.id).toBe('ses_2')
    expect(forked.effectiveSeq).toBe(120)
    expect(forked.summary.harness).toBe('codex')
    expect(seen).toEqual(['POST /v1/sessions/ses_1/fork {"at_seq":122,"harness":"codex","workspace":"shared"}'])
  })

  it('meldet Fehler als RFC-9457-Problem', async () => {
    const url = await serve(() => [404, { status: 404, code: 'not_found', title: 'Nicht gefunden' }])
    const client = new BetonClient({ baseUrl: url })
    const err = await client.sessions.get('ses_x').catch((e: unknown) => e)
    expect(err).toBeInstanceOf(BetonError)
    expect((err as BetonError).code).toBe('not_found')
    expect((err as BetonError).status).toBe(404)
  })

  it('WEB-006 AC1: lädt Anhänge roh hoch und sendet sie mit der Eingabe', async () => {
    const seen: string[] = []
    const url = await serve((req, text) => {
      seen.push(`${req.method} ${req.url} ${req.headers['content-type']} ${text}`)
      if (req.url?.includes('/attachments')) {
        return [201, { blob: `sha256:${'a'.repeat(64)}`, name: 'bild 1.png', mime: 'image/png', size: 3 }]
      }
      return [202, { input_id: 'i', status: 'started' }]
    })
    const s = new BetonClient({ baseUrl: url, token: 'tok' }).session('ses_1')
    const att = await s.uploadAttachment(new Uint8Array([1, 2, 3]), 'bild 1.png', 'image/png')
    await s.send('Was ist das?', undefined, [att])
    expect(seen[0]).toBe('POST /v1/sessions/ses_1/attachments?name=bild%201.png image/png \u0001\u0002\u0003')
    expect(JSON.parse(seen[1]!.split(' application/json ')[1]!)).toEqual({ text: 'Was ist das?', attachments: [att] })
  })

  it('API-003: URL des SSE-Stroms für Skripte', () => {
    const s = new BetonClient({ baseUrl: 'http://127.0.0.1:7420/' }).session('ses_1')
    expect(s.eventStreamUrl()).toBe('http://127.0.0.1:7420/v1/sessions/ses_1/events/stream?from_seq=0')
    expect(s.eventStreamUrl(42, true)).toBe('http://127.0.0.1:7420/v1/sessions/ses_1/events/stream?from_seq=42&transient=true')
  })
})
