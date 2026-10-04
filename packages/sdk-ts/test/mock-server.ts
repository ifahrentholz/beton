import { type AddressInfo } from 'node:net'
import { WebSocketServer, type WebSocket } from 'ws'

/** Minimaler beton-WebSocket-Server für Tests: hello/welcome, attach mit Replay, live. */
export class MockServer {
  readonly wss: WebSocketServer
  readonly events: Array<{ seq: number }> = []
  readonly attaches: number[] = []
  readonly authHeaders: Array<string | undefined> = []
  private readonly sockets = new Set<WebSocket>()
  private readonly live = new Set<WebSocket>()

  private constructor(wss: WebSocketServer) {
    this.wss = wss
    wss.on('connection', (ws, req) => {
      this.authHeaders.push(req.headers.authorization)
      this.sockets.add(ws)
      ws.on('close', () => {
        this.sockets.delete(ws)
        this.live.delete(ws)
      })
      ws.on('message', (raw) => {
        const msg = JSON.parse(String(raw)) as { t: string; from_seq?: number }
        if (msg.t === 'hello') {
          ws.send(JSON.stringify({ t: 'welcome', protocol: '1.0', server_version: 'mock', session_limits: {} }))
        } else if (msg.t === 'attach') {
          const from = msg.from_seq ?? 0
          this.attaches.push(from)
          const replay = this.events.filter((e) => e.seq > from)
          if (replay.length) ws.send(JSON.stringify({ t: 'events', session_id: 'ses_x', events: replay }))
          ws.send(JSON.stringify({ t: 'live', session_id: 'ses_x', head_seq: this.head() }))
          this.live.add(ws)
        }
      })
    })
  }

  static async start(): Promise<MockServer> {
    const wss = new WebSocketServer({ port: 0, handleProtocols: () => 'beton.v1' })
    await new Promise<void>((resolve) => wss.once('listening', () => resolve()))
    return new MockServer(wss)
  }

  get url(): string {
    return `http://127.0.0.1:${(this.wss.address() as AddressInfo).port}`
  }

  head(): number {
    return this.events.at(-1)?.seq ?? 0
  }

  /** Neues dauerhaftes Event an alle angehängten Clients. */
  produce(n: number): void {
    for (let i = 0; i < n; i += 1) {
      const e = event(this.head() + 1)
      this.events.push(e)
      for (const ws of this.live) ws.send(JSON.stringify({ t: 'events', session_id: 'ses_x', events: [e] }))
    }
  }

  /** Verbindungen hart trennen (simulierter Netzabbruch). */
  dropAll(): void {
    for (const ws of this.sockets) ws.terminate()
  }

  /** Geordnet mit Close-Code schließen. */
  closeAll(code: number): void {
    for (const ws of this.sockets) ws.close(code, 'test')
  }

  stop(): Promise<void> {
    return new Promise((resolve) => this.wss.close(() => resolve()))
  }
}

export function event(seq: number) {
  return {
    v: 1,
    id: `evt_${String(seq).padStart(26, '0')}`,
    session_id: 'ses_x',
    seq,
    ts: '2026-10-04T00:00:00Z',
    actor: { kind: 'system', component: 'runner' },
    type: 'notice',
    payload: { level: 'info', text: `n${seq}` },
  }
}
