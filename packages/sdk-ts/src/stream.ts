import { Backoff, shutdownDelay } from './backoff.js'
import type { Event } from './gen/Event.js'
import type { ServerMsg } from './gen/ServerMsg.js'

export const SUBPROTOCOL = 'beton.v1'
export const PROTOCOL_VERSION = '1.0'

/** Minimale WebSocket-Schnittstelle (Browser, Node ≥ 22 oder das Paket `ws`). */
export interface WebSocketLike {
  readonly readyState: number
  send(data: string): void
  close(code?: number, reason?: string): void
  addEventListener(type: 'open', listener: () => void): void
  addEventListener(type: 'message', listener: (ev: { data: unknown }) => void): void
  addEventListener(type: 'close', listener: (ev: { code: number; reason: string }) => void): void
  addEventListener(type: 'error', listener: (ev: unknown) => void): void
}

/** Baut eine Verbindung; `headers` nur außerhalb des Browsers (dort zählt das Cookie). */
export type WebSocketFactory = (
  url: string,
  protocols: string[],
  headers: Record<string, string>,
) => WebSocketLike

/** Standard: `globalThis.WebSocket`; mit Token im Node-Stil (undici) samt Header. */
export const defaultWebSocketFactory: WebSocketFactory = (url, protocols, headers) => {
  const Ctor = (globalThis as { WebSocket?: unknown }).WebSocket as
    | (new (url: string, opts?: unknown) => WebSocketLike)
    | undefined
  if (!Ctor) {
    throw new Error('Kein WebSocket verfügbar; unter Node 20 `webSocketFactory` mit dem Paket `ws` übergeben')
  }
  if (Object.keys(headers).length > 0) {
    return new Ctor(url, { protocols, headers })
  }
  return new Ctor(url, protocols)
}

export interface StreamOptions {
  /** Ab dieser `seq` (exklusiv). */
  fromSeq?: number
  /** Transiente Events (Deltas) mitliefern; Standard `true`. */
  transient?: boolean
  signal?: AbortSignal
  /** Server kennt weniger Events als der Client (`seq_ahead`): Zustand ab 0 neu aufbauen. */
  onReset?: () => void
  /** Verbindung verloren; nächster Versuch nach `delayMs`. */
  onReconnecting?: (attempt: number, delayMs: number) => void
  /** Für Tests: kürzere Basis des Backoffs. */
  backoff?: Backoff
}

interface Transport {
  wsUrl: string
  headers: Record<string, string>
  factory: WebSocketFactory
}

const FATAL_CLOSE = new Set([4401, 4403, 4404])

/**
 * Event-Strom einer Session mit automatischem Resume (API-004 AC2, PROTO-005, PROTO-009):
 * nach einem Abbruch neu verbinden und ab der zuletzt gesehenen `seq` attachen. Dauerhafte
 * Events kommen genau einmal und lückenlos; `overflow` und `seq_ahead` werden behandelt.
 */
export function eventStream(
  transport: Transport,
  sessionId: string,
  options: StreamOptions = {},
): AsyncIterable<Event> {
  return {
    [Symbol.asyncIterator]() {
      return new EventStream(transport, sessionId, options)
    },
  }
}

class EventStream implements AsyncIterator<Event> {
  private lastSeq: number
  private readonly queue: Event[] = []
  private waiter: ((r: IteratorResult<Event>) => void) | undefined
  private failer: ((e: unknown) => void) | undefined
  private error: unknown
  private done = false
  private ws: WebSocketLike | undefined
  private readonly backoff: Backoff
  private attempt = 0
  private timer: ReturnType<typeof setTimeout> | undefined
  private nextId = 0

  constructor(
    private readonly transport: Transport,
    private readonly sessionId: string,
    private readonly options: StreamOptions,
  ) {
    this.lastSeq = options.fromSeq ?? 0
    this.backoff = options.backoff ?? new Backoff()
    options.signal?.addEventListener('abort', () => this.finish())
    this.connect()
  }

  private connect(): void {
    if (this.done) return
    let ws: WebSocketLike
    try {
      ws = this.transport.factory(this.transport.wsUrl, [SUBPROTOCOL], this.transport.headers)
    } catch (e) {
      this.fail(e)
      return
    }
    this.ws = ws
    ws.addEventListener('open', () => {
      ws.send(
        JSON.stringify({ t: 'hello', protocol: PROTOCOL_VERSION, client: { kind: 'sdk-ts', version: '0.0.0' } }),
      )
    })
    ws.addEventListener('message', (ev) => this.onMessage(ws, ev.data))
    ws.addEventListener('close', (ev) => this.onClose(ws, ev.code))
    ws.addEventListener('error', () => {
      // Folgt immer ein `close`; dort wird neu verbunden.
    })
  }

  private attach(ws: WebSocketLike, fromSeq: number): void {
    this.nextId += 1
    ws.send(
      JSON.stringify({
        t: 'attach',
        id: `a${this.nextId}`,
        session_id: this.sessionId,
        from_seq: fromSeq,
        transient: this.options.transient ?? true,
      }),
    )
  }

  private onMessage(ws: WebSocketLike, data: unknown): void {
    if (ws !== this.ws || typeof data !== 'string') return
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
        this.attach(ws, this.lastSeq)
        break
      case 'events':
        for (const e of msg.events) {
          if (e.transient) {
            this.push(e)
          } else if (e.seq > this.lastSeq) {
            this.lastSeq = e.seq
            this.push(e)
          }
        }
        break
      case 'overflow':
        // Ab dem zuletzt Gesehenen neu anfordern (PROTO-008).
        this.attach(ws, this.lastSeq)
        break
      case 'nack': {
        const problem = msg.problem as { code?: string; detail?: string } | null
        if (problem?.code === 'seq_ahead') {
          this.lastSeq = 0
          this.options.onReset?.()
          this.attach(ws, 0)
        } else {
          this.fail(new Error(problem?.detail ?? 'nack'))
        }
        break
      }
      default:
        break
    }
  }

  private onClose(ws: WebSocketLike, code: number): void {
    if (ws !== this.ws || this.done) return
    this.ws = undefined
    if (FATAL_CLOSE.has(code)) {
      this.fail(new Error(`WebSocket mit Code ${code} geschlossen`))
      return
    }
    this.attempt += 1
    const delay = code === 4503 ? shutdownDelay() : this.backoff.next()
    this.options.onReconnecting?.(this.attempt, delay)
    this.timer = setTimeout(() => this.connect(), delay)
  }

  private push(e: Event): void {
    if (this.waiter) {
      const w = this.waiter
      this.waiter = undefined
      this.failer = undefined
      w({ value: e, done: false })
    } else {
      this.queue.push(e)
    }
  }

  private fail(e: unknown): void {
    this.error = e
    this.done = true
    if (this.timer) clearTimeout(this.timer)
    this.ws?.close()
    if (this.failer) {
      const f = this.failer
      this.waiter = undefined
      this.failer = undefined
      f(e)
    }
  }

  private finish(): void {
    if (this.done) return
    this.done = true
    if (this.timer) clearTimeout(this.timer)
    this.ws?.close(1000)
    this.ws = undefined
    if (this.waiter) {
      const w = this.waiter
      this.waiter = undefined
      this.failer = undefined
      w({ value: undefined, done: true })
    }
  }

  next(): Promise<IteratorResult<Event>> {
    const queued = this.queue.shift()
    if (queued) return Promise.resolve({ value: queued, done: false })
    if (this.error) return Promise.reject(this.error)
    if (this.done) return Promise.resolve({ value: undefined, done: true })
    return new Promise((resolve, reject) => {
      this.waiter = resolve
      this.failer = reject
    })
  }

  return(): Promise<IteratorResult<Event>> {
    this.finish()
    return Promise.resolve({ value: undefined, done: true })
  }
}
