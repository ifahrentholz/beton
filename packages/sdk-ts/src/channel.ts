import { Backoff, shutdownDelay } from './backoff.js'
import type { ChannelAccess } from './gen/ChannelAccess.js'
import type { ChannelKind } from './gen/ChannelKind.js'
import type { ServerMsg } from './gen/ServerMsg.js'
import { PROTOCOL_VERSION, SUBPROTOCOL, type WebSocketFactory, type WebSocketLike } from './stream.js'

/** Version des Binärframings (PROTO-007). */
export const FRAME_VERSION = 1
/** Kopf: Version, Kanal-ID (u32 Big Endian), Flags. */
export const FRAME_HEADER = 6
export const FLAG_FIN = 0x01
export const FLAG_KEYFRAME = 0x02
export const FLAG_INPUT = 0x04
/** Größte Nutzlast eines Frames; längere Eingaben gehen in mehreren. */
export const MAX_PAYLOAD = 64 * 1024

/** Ein Binärframe `[ver u8][channel_id u32 BE][flags u8][payload]`. */
export interface Frame {
  channelId: number
  flags: number
  payload: Uint8Array
}

export function encodeFrame(f: Frame): Uint8Array {
  const out = new Uint8Array(FRAME_HEADER + f.payload.length)
  const view = new DataView(out.buffer)
  view.setUint8(0, FRAME_VERSION)
  view.setUint32(1, f.channelId)
  view.setUint8(5, f.flags)
  out.set(f.payload, FRAME_HEADER)
  return out
}

/** Liest ein Frame; `undefined` bei falscher Version oder zu kurzem Frame. */
export function decodeFrame(data: ArrayBuffer | Uint8Array): Frame | undefined {
  const bytes = data instanceof Uint8Array ? data : new Uint8Array(data)
  if (bytes.length < FRAME_HEADER || bytes[0] !== FRAME_VERSION) return undefined
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return { channelId: view.getUint32(1), flags: view.getUint8(5), payload: bytes.subarray(FRAME_HEADER) }
}

interface Transport {
  wsUrl: string
  headers: Record<string, string>
  factory: WebSocketFactory
}

export interface ChannelOptions {
  /**
   * Daten des Kanals; `keyframe` heißt: vollständiger Zustand (Terminal: Scrollback und
   * Bildschirm, beginnt mit einem Reset), danach laufende Ausgabe. Ohne `manualCredit` gibt der
   * Kanal den Kredit zurück, sobald der Aufruf zurückkehrt.
   */
  onData: (bytes: Uint8Array, keyframe: boolean) => void
  /** Kanal offen (auch nach einem Reconnect) mit dem Recht des Clients. */
  onOpen?: (access: ChannelAccess) => void
  /** Kanal endgültig zu (`closed`, `not_found`, `forbidden` …). */
  onClosed?: (reason: string) => void
  /** Verbindung verloren; nächster Versuch nach `delayMs`. */
  onReconnecting?: (attempt: number, delayMs: number) => void
  /** Kredit erst mit {@link Channel.consumed} zurückgeben (z. B. nach dem Rendern). */
  manualCredit?: boolean
  /** Fenster für Daten des Servers in Bytes (Default 1 MiB). */
  window?: number
  backoff?: Backoff
}

const FATAL_CLOSE = new Set([4401, 4403, 4404])
const CREDIT_STEP = 64 * 1024

/**
 * Ein Binärkanal auf einer eigenen WebSocket-Verbindung (PROTO-007): öffnet ihn, gibt
 * Kredit für die Daten und sendet Eingaben als Frames mit `INPUT`. Bricht die Verbindung ab,
 * baut der Kanal sie neu auf und öffnet sich wieder; das neue Keyframe ersetzt den Zustand.
 */
export class Channel {
  private ws: WebSocketLike | undefined
  private channelId: number | undefined
  private done = false
  private readonly backoff: Backoff
  private attempt = 0
  private timer: ReturnType<typeof setTimeout> | undefined
  private owed = 0
  private inputWindow = 0
  private readonly pending: Uint8Array[] = []
  private keyframe = false
  private nextId = 0
  /** Recht des Clients auf diesem Kanal, sobald offen. */
  access: ChannelAccess | undefined

  constructor(
    private readonly transport: Transport,
    private readonly sessionId: string,
    private readonly kind: ChannelKind,
    private readonly target: string,
    private readonly options: ChannelOptions,
  ) {
    this.backoff = options.backoff ?? new Backoff()
    this.connect()
  }

  /** Eingabe senden (nur mit Recht `drive`); wartet, wenn das Fenster erschöpft ist. */
  send(data: string | Uint8Array): void {
    if (this.done) return
    const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data
    for (let i = 0; i < bytes.length; i += MAX_PAYLOAD) this.pending.push(bytes.subarray(i, i + MAX_PAYLOAD))
    this.flushInput()
  }

  /** Verarbeitete Bytes melden (nur mit `manualCredit`). */
  consumed(bytes: number): void {
    this.owed += bytes
    if (this.owed >= CREDIT_STEP) this.credit()
  }

  close(): void {
    if (this.done) return
    this.done = true
    if (this.timer) clearTimeout(this.timer)
    const ws = this.ws
    this.ws = undefined
    if (ws && ws.readyState === 1 && this.channelId !== undefined) {
      ws.send(JSON.stringify({ t: 'close_channel', channel_id: this.channelId }))
    }
    ws?.close(1000)
  }

  private credit(): void {
    if (!this.ws || this.channelId === undefined || this.owed === 0) return
    this.ws.send(JSON.stringify({ t: 'credit', channel_id: this.channelId, bytes: this.owed }))
    this.owed = 0
  }

  private flushInput(): void {
    const ws = this.ws
    if (!ws || this.channelId === undefined || this.access !== 'drive') return
    while (this.pending.length > 0 && this.pending[0]!.length <= this.inputWindow) {
      const chunk = this.pending.shift()!
      this.inputWindow -= chunk.length
      ws.send(encodeFrame({ channelId: this.channelId, flags: FLAG_INPUT | FLAG_FIN, payload: chunk }))
    }
  }

  private connect(): void {
    if (this.done) return
    let ws: WebSocketLike
    try {
      ws = this.transport.factory(this.transport.wsUrl, [SUBPROTOCOL], this.transport.headers)
    } catch {
      this.retry(0)
      return
    }
    ws.binaryType = 'arraybuffer'
    this.ws = ws
    this.channelId = undefined
    ws.addEventListener('open', () => {
      ws.send(JSON.stringify({ t: 'hello', protocol: PROTOCOL_VERSION, client: { kind: 'sdk-ts', version: '0.0.0' } }))
    })
    ws.addEventListener('message', (ev) => this.onMessage(ws, ev.data))
    ws.addEventListener('close', (ev) => {
      if (ws !== this.ws || this.done) return
      this.ws = undefined
      if (FATAL_CLOSE.has(ev.code)) {
        this.finish(`close_${ev.code}`)
        return
      }
      this.retry(ev.code)
    })
    ws.addEventListener('error', () => {
      // Es folgt immer ein `close`.
    })
  }

  private retry(code: number): void {
    this.attempt += 1
    const delay = code === 4503 ? shutdownDelay() : this.backoff.next()
    this.options.onReconnecting?.(this.attempt, delay)
    this.timer = setTimeout(() => this.connect(), delay)
  }

  private finish(reason: string): void {
    if (this.done) return
    this.done = true
    this.ws?.close(1000)
    this.ws = undefined
    this.options.onClosed?.(reason)
  }

  private onMessage(ws: WebSocketLike, data: unknown): void {
    if (ws !== this.ws) return
    if (typeof data !== 'string') {
      const bytes = data instanceof ArrayBuffer ? data : data instanceof Uint8Array ? data : undefined
      const f = bytes ? decodeFrame(bytes) : undefined
      if (!f || f.channelId !== this.channelId) return
      const isKey = (f.flags & FLAG_KEYFRAME) !== 0
      // Erstes Fragment eines Keyframes: Zustand neu.
      this.options.onData(f.payload, isKey && !this.keyframe)
      this.keyframe = isKey && (f.flags & FLAG_FIN) === 0
      if (!this.options.manualCredit) this.consumed(f.payload.length)
      return
    }
    let msg: ServerMsg
    try {
      msg = JSON.parse(data) as ServerMsg
    } catch {
      return
    }
    switch (msg.t) {
      case 'welcome':
        this.backoff.reset()
        this.attempt = 0
        this.nextId += 1
        ws.send(
          JSON.stringify({ t: 'open_channel', id: `c${this.nextId}`, session_id: this.sessionId, kind: this.kind, ref: this.target }),
        )
        break
      case 'channel_opened':
        this.channelId = msg.channel_id
        this.access = msg.access
        this.inputWindow = msg.window
        this.keyframe = false
        ws.send(JSON.stringify({ t: 'credit', channel_id: msg.channel_id, bytes: this.options.window ?? 1024 * 1024 }))
        this.options.onOpen?.(msg.access)
        this.flushInput()
        break
      case 'credit':
        if (msg.channel_id === this.channelId) {
          this.inputWindow += msg.bytes
          this.flushInput()
        }
        break
      case 'channel_closed':
        if (msg.channel_id !== this.channelId) break
        this.channelId = undefined
        // Runner kurz getrennt: neu öffnen, sobald er den Kanal wieder anmeldet.
        if (msg.reason === 'detached' || msg.reason === 'stalled') {
          ws.close(1000)
          this.ws = undefined
          this.retry(0)
        } else {
          this.finish(msg.reason ?? 'closed')
        }
        break
      case 'nack': {
        const problem = msg.problem as { code?: string; status?: number } | null
        if (this.channelId === undefined) {
          // Öffnen abgelehnt: Kanal gibt es nicht (mehr) bzw. kein Recht.
          this.finish(problem?.code ?? 'rejected')
        }
        break
      }
      default:
        break
    }
  }
}
