import { BetonError, type Problem } from './errors.js'
import type { CreateSessionRequest } from './gen/CreateSessionRequest.js'
import type { EventPage } from './gen/EventPage.js'
import type { Info } from './gen/Info.js'
import type { InputAccepted } from './gen/InputAccepted.js'
import type { ApprovalPage } from './gen/ApprovalPage.js'
import type { ResolveApprovalRequest } from './gen/ResolveApprovalRequest.js'
import type { SessionPage } from './gen/SessionPage.js'
import type { SessionSettings } from './gen/SessionSettings.js'
import type { SessionSummary } from './gen/SessionSummary.js'
import type { Event } from './gen/Event.js'
import { defaultWebSocketFactory, eventStream, type StreamOptions, type WebSocketFactory } from './stream.js'

export interface ClientOptions {
  /** Basis-URL des Servers, z. B. `http://127.0.0.1:7420`. */
  baseUrl: string
  /** Lokales Token (Node, CLI-Werkzeuge). Im Browser authentisiert das Session-Cookie. */
  token?: string
  fetch?: typeof fetch
  webSocketFactory?: WebSocketFactory
}

/** Client der beton-API (API-004). */
export class BetonClient {
  readonly baseUrl: string
  private readonly token: string | undefined
  private readonly fetchImpl: typeof fetch
  private readonly wsFactory: WebSocketFactory

  constructor(options: ClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, '')
    this.token = options.token
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis)
    this.wsFactory = options.webSocketFactory ?? defaultWebSocketFactory
  }

  /** `GET /v1/info`. */
  info(): Promise<Info> {
    return this.request('GET', '/v1/info')
  }

  readonly sessions = {
    list: (
      opts: { limit?: number; cursor?: string; includeArchived?: boolean; updatedAfter?: string } = {},
    ): Promise<SessionPage> => {
      const q = new URLSearchParams()
      if (opts.limit !== undefined) q.set('limit', String(opts.limit))
      if (opts.cursor) q.set('cursor', opts.cursor)
      if (opts.includeArchived) q.set('include_archived', 'true')
      // Nur Sessions mit Aktivität danach (Listen-Deltas, WEB-003).
      if (opts.updatedAfter) q.set('updated_after', opts.updatedAfter)
      const qs = q.toString()
      return this.request('GET', `/v1/sessions${qs ? `?${qs}` : ''}`)
    },
    get: (id: string): Promise<SessionSummary> => this.request('GET', `/v1/sessions/${enc(id)}`),
    create: async (req: CreateSessionRequest): Promise<Session> => {
      const created = await this.request<SessionSummary>('POST', '/v1/sessions', req)
      return new Session(this, created.id)
    },
  }

  /** Handle für eine vorhandene Session. */
  session(id: string): Session {
    return new Session(this, id)
  }

  /** Low-Level-Aufruf; Fehler als {@link BetonError}. */
  async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { accept: 'application/json' }
    if (this.token) headers.authorization = `Bearer ${this.token}`
    if (body !== undefined) headers['content-type'] = 'application/json'
    const res = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? null : JSON.stringify(body),
      credentials: 'include',
    })
    const text = await res.text()
    if (!res.ok) {
      let problem: Problem = { status: res.status }
      try {
        problem = JSON.parse(text) as Problem
      } catch {
        problem = { status: res.status, title: res.statusText }
      }
      throw new BetonError(res.status, problem)
    }
    return (text ? JSON.parse(text) : undefined) as T
  }

  /** Transport für WebSocket-Ströme. */
  transport() {
    const wsUrl = `${this.baseUrl.replace(/^http/, 'ws')}/v1/ws`
    const headers: Record<string, string> = {}
    if (this.token) headers.authorization = `Bearer ${this.token}`
    return { wsUrl, headers, factory: this.wsFactory }
  }
}

/** Eine Session. */
export class Session {
  constructor(
    private readonly client: BetonClient,
    readonly id: string,
  ) {}

  summary(): Promise<SessionSummary> {
    return this.client.sessions.get(this.id)
  }

  /** Events ab `fromSeq` (Replay, dann live) mit automatischem Resume. */
  events(options: StreamOptions = {}): AsyncIterable<Event> {
    return eventStream(this.client.transport(), this.id, options)
  }

  /** Eine Seite dauerhafter Events per REST. */
  history(afterSeq = 0, limit = 200): Promise<EventPage> {
    return this.client.request('GET', `/v1/sessions/${enc(this.id)}/events?after_seq=${afterSeq}&limit=${limit}`)
  }

  send(text: string): Promise<InputAccepted> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/input`, { text })
  }

  interrupt(): Promise<void> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/interrupt`)
  }

  resume(): Promise<SessionSummary> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/resume`)
  }

  update(settings: SessionSettings): Promise<SessionSummary> {
    return this.client.request('PATCH', `/v1/sessions/${enc(this.id)}`, settings)
  }

  archive(): Promise<SessionSummary> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/archive`)
  }

  unarchive(): Promise<SessionSummary> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/unarchive`)
  }

  delete(): Promise<void> {
    return this.client.request('DELETE', `/v1/sessions/${enc(this.id)}`)
  }

  /** Ausgelagerte Nutzlast eines Events (PROTO-001 AC4). */
  blob(ref: string): Promise<unknown> {
    return this.client.request('GET', `/v1/sessions/${enc(this.id)}/blobs/${enc(ref)}`)
  }

  approvals(): Promise<ApprovalPage> {
    return this.client.request('GET', `/v1/sessions/${enc(this.id)}/approvals`)
  }

  resolveApproval(approvalId: string, req: ResolveApprovalRequest): Promise<void> {
    return this.client.request(
      'POST',
      `/v1/sessions/${enc(this.id)}/approvals/${enc(approvalId)}/resolve`,
      req,
    )
  }

  /** Fork ab `atSeq` (SES-006). Folgt mit M1. */
  fork(_options: { atSeq: number; harness?: string }): Promise<Session> {
    return Promise.reject(new Error('Fork folgt mit M1 (SES-006)'))
  }
}

function enc(s: string): string {
  return encodeURIComponent(s)
}
