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

  it('AGT-009: liest den Sub-Agent-Baum einer Session', async () => {
    const seen: string[] = []
    const url = await serve((req) => {
      seen.push(`${req.method} ${req.url}`)
      return [
        200,
        {
          root: 'ses_1',
          nodes: [
            { id: 'ses_1', depth: 0, title: 'maestra', harness: 'claude', status: 'running', cost_micro: 0, subtree_cost_micro: 0, tokens: 10, subtree_tokens: 30, created_at: 't' },
            { id: 'ses_2', parent_id: 'ses_1', depth: 1, title: 'impl-codex', harness: 'codex', status: 'idle', task: 'completed', cost_micro: 0, subtree_cost_micro: 0, tokens: 20, subtree_tokens: 20, created_at: 't' },
          ],
        },
      ]
    })
    const tree = await new BetonClient({ baseUrl: url }).session('ses_1').subagents()
    expect(tree.nodes.map((n) => n.harness)).toEqual(['claude', 'codex'])
    expect(tree.nodes[1]!.parent_id).toBe('ses_1')
    expect(seen).toEqual(['GET /v1/sessions/ses_1/subagents'])
  })

  it('meldet Fehler als RFC-9457-Problem', async () => {
    const url = await serve(() => [404, { status: 404, code: 'not_found', title: 'Nicht gefunden' }])
    const client = new BetonClient({ baseUrl: url })
    const err = await client.sessions.get('ses_x').catch((e: unknown) => e)
    expect(err).toBeInstanceOf(BetonError)
    expect((err as BetonError).code).toBe('not_found')
    expect((err as BetonError).status).toBe(404)
  })

  it('SES-017: Workspace-Pfade segmentweise kodiert, Schreiben mit If-Match, 412 als BetonError', async () => {
    const seen: string[] = []
    const url = await serve((req, text) => {
      seen.push(`${req.method} ${req.url} ${req.headers['if-match'] ?? '-'} ${text}`)
      if (req.method === 'PUT') return [412, { status: 412, code: 'precondition_failed', title: 'Veraltet' }]
      if (req.url?.startsWith('/v1/sessions/ses_1/workspace/files/')) return [200, { path: 'a b/c#.ts', size: 1, sha256: 'abc', binary: false, too_large: false, content: 'x' }]
      return [200, { items: [], next_cursor: null, path: '', scope: 'turn', base_sha: 'b' }]
    })
    const s = new BetonClient({ baseUrl: url }).session('ses_1')
    const f = await s.workspace.readFile('a b/c#.ts')
    expect(f.sha256).toBe('abc')
    const err = await s.workspace.writeFile('a b/c#.ts', 'neu', 'abc').catch((e: unknown) => e)
    expect((err as BetonError).status).toBe(412)
    await s.workspace.info()
    await s.workspace.tree('src')
    await s.workspace.search('Retry-After', 'content')
    await s.workspace.changes('turn', { turn: 'trn_1' })
    await s.workspace.diff('src/a.ts', 'uncommitted')
    expect(seen).toEqual([
      'GET /v1/sessions/ses_1/workspace/files/a%20b/c%23.ts - ',
      'PUT /v1/sessions/ses_1/workspace/files/a%20b/c%23.ts "abc" {"content":"neu"}',
      'GET /v1/sessions/ses_1/workspace - ',
      'GET /v1/sessions/ses_1/workspace/tree?path=src - ',
      'GET /v1/sessions/ses_1/workspace/search?q=Retry-After&mode=content - ',
      'GET /v1/sessions/ses_1/workspace/changes?scope=turn&turn=trn_1 - ',
      'GET /v1/sessions/ses_1/workspace/diff?path=src%2Fa.ts&scope=uncommitted - ',
    ])
  })
})
