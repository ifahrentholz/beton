import { type AddressInfo } from 'node:net'
import { afterEach, describe, expect, it } from 'vitest'
import { WebSocketServer, type WebSocket } from 'ws'
import { Backoff, BetonClient, decodeFrame, encodeFrame, FLAG_FIN, FLAG_INPUT, FLAG_KEYFRAME } from '../src/index.js'

/** Minimaler Kanal-Server: hello, open_channel, credit, INPUT-Frames. */
class ChannelServer {
  readonly credits: number[] = []
  readonly inputs: Uint8Array[] = []
  readonly opens: string[] = []
  access: 'read' | 'drive' = 'drive'
  sockets = new Set<WebSocket>()

  private constructor(readonly wss: WebSocketServer) {
    wss.on('connection', (ws) => {
      this.sockets.add(ws)
      ws.on('close', () => this.sockets.delete(ws))
      ws.on('message', (raw, isBinary) => {
        if (isBinary) {
          const f = decodeFrame(new Uint8Array(raw as Buffer))!
          if (this.access === 'read') {
            ws.send(JSON.stringify({ t: 'nack', id: `channel:${f.channelId}`, problem: { status: 403 } }))
            return
          }
          this.inputs.push(f.payload)
          ws.send(JSON.stringify({ t: 'credit', channel_id: f.channelId, bytes: f.payload.length }))
          return
        }
        const msg = JSON.parse(String(raw)) as { t: string; ref?: string; bytes?: number }
        if (msg.t === 'hello') ws.send(JSON.stringify({ t: 'welcome', protocol: '1.1', server_version: 'mock', session_limits: {} }))
        if (msg.t === 'open_channel') {
          this.opens.push(msg.ref!)
          if (msg.ref === 'term_weg') {
            ws.send(JSON.stringify({ t: 'nack', id: 'c', problem: { code: 'not_found', status: 404 } }))
            return
          }
          ws.send(JSON.stringify({ t: 'channel_opened', id: 'c', channel_id: 7, window: this.access === 'drive' ? 8 : 0, access: this.access }))
          ws.send(encodeFrame({ channelId: 7, flags: FLAG_KEYFRAME, payload: new TextEncoder().encode('\x1bcAB') }))
          ws.send(encodeFrame({ channelId: 7, flags: FLAG_KEYFRAME | FLAG_FIN, payload: new TextEncoder().encode('CD') }))
          ws.send(encodeFrame({ channelId: 7, flags: FLAG_FIN, payload: new TextEncoder().encode('$ ') }))
        }
        if (msg.t === 'credit') this.credits.push(msg.bytes!)
      })
    })
  }

  static async start(): Promise<ChannelServer> {
    const wss = new WebSocketServer({ port: 0, handleProtocols: () => 'beton.v1' })
    await new Promise<void>((r) => wss.once('listening', () => r()))
    return new ChannelServer(wss)
  }

  get url(): string {
    return `http://127.0.0.1:${(this.wss.address() as AddressInfo).port}`
  }

  stop(): Promise<void> {
    for (const s of this.sockets) s.terminate()
    return new Promise((r) => this.wss.close(() => r()))
  }
}

let server: ChannelServer | undefined
afterEach(async () => {
  await server?.stop()
  server = undefined
})

const until = async (cond: () => boolean) => {
  const end = Date.now() + 5000
  while (!cond()) {
    if (Date.now() > end) throw new Error('Zeitüberschreitung')
    await new Promise((r) => setTimeout(r, 10))
  }
}

describe('Terminal-Kanal', () => {
  it('PROTO-007: Keyframe zuerst, Kredit zurück, Eingaben im Fenster des Servers', async () => {
    server = await ChannelServer.start()
    const client = new BetonClient({ baseUrl: server.url, token: 'tok' })
    const chunks: Array<[string, boolean]> = []
    let access = ''
    const ch = client.session('ses_x').terminals.attach('term_1', {
      onData: (b, keyframe) => chunks.push([new TextDecoder().decode(b), keyframe]),
      onOpen: (a) => (access = a),
      window: 4096,
    })
    await until(() => chunks.length === 3)
    expect(access).toBe('drive')
    // Nur das erste Fragment des Keyframes setzt den Zustand neu.
    expect(chunks).toEqual([
      ['\x1bcAB', true],
      ['CD', false],
      ['$ ', false],
    ])
    expect(server.credits[0]).toBe(4096)
    // 12 Bytes bei 8 Bytes Fenster: der Rest wartet auf den Kredit des Servers.
    ch.send('ls -la\r')
    ch.send('pwd\r')
    await until(() => server!.inputs.length === 2)
    expect(server.inputs.map((b) => new TextDecoder().decode(b))).toEqual(['ls -la\r', 'pwd\r'])
    ch.close()
  })

  it('WEB-010 AC3: mit Leserecht gehen keine Eingaben hinaus', async () => {
    server = await ChannelServer.start()
    server.access = 'read'
    const client = new BetonClient({ baseUrl: server.url })
    let access = ''
    const ch = client.session('ses_x').terminals.attach('term_1', { onData: () => {}, onOpen: (a) => (access = a) })
    await until(() => access === 'read')
    ch.send('rm -rf ~\r')
    await new Promise((r) => setTimeout(r, 100))
    expect(server.inputs).toEqual([])
    ch.close()
  })

  it('PROTO-007: Abbruch der Verbindung öffnet den Kanal neu, ein fehlendes Terminal beendet ihn', async () => {
    server = await ChannelServer.start()
    const client = new BetonClient({ baseUrl: server.url })
    let keyframes = 0
    const ch = client.session('ses_x').terminals.attach('term_1', {
      onData: (_b, k) => (keyframes += k ? 1 : 0),
      backoff: new Backoff(20, 100),
    })
    await until(() => keyframes === 1)
    for (const s of server.sockets) s.terminate()
    await until(() => keyframes === 2)
    expect(server.opens).toEqual(['term_1', 'term_1'])
    ch.close()
    let reason = ''
    client.session('ses_x').terminals.attach('term_weg', { onData: () => {}, onClosed: (r) => (reason = r) })
    await until(() => reason !== '')
    expect(reason).toBe('not_found')
  })

  it('Frames: Kopf mit Version, Kanal-ID Big Endian und Flags', () => {
    const bytes = encodeFrame({ channelId: 0x01020304, flags: FLAG_INPUT, payload: new Uint8Array([9]) })
    expect([...bytes]).toEqual([1, 1, 2, 3, 4, FLAG_INPUT, 9])
    expect(decodeFrame(bytes)).toEqual({ channelId: 0x01020304, flags: FLAG_INPUT, payload: new Uint8Array([9]) })
    expect(decodeFrame(new Uint8Array([2, 0, 0, 0, 1, 0]))).toBeUndefined()
  })
})
