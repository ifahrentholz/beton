import { BetonError, type Problem } from './errors.js'
import type { CreateSessionRequest } from './gen/CreateSessionRequest.js'
import type { EventPage } from './gen/EventPage.js'
import type { ForkRequest } from './gen/ForkRequest.js'
import type { ForkResponse } from './gen/ForkResponse.js'
import type { ForkWorkspace } from './gen/ForkWorkspace.js'
import type { Info } from './gen/Info.js'
import type { InputAccepted } from './gen/InputAccepted.js'
import type { ApprovalPage } from './gen/ApprovalPage.js'
import type { ResolveApprovalRequest } from './gen/ResolveApprovalRequest.js'
import type { SessionPage } from './gen/SessionPage.js'
import type { SessionSettings } from './gen/SessionSettings.js'
import type { SessionSummary } from './gen/SessionSummary.js'
import type { InputMode } from './gen/InputMode.js'
import type { QueueView } from './gen/QueueView.js'
import type { Event } from './gen/Event.js'
import type { Attachment } from './gen/Attachment.js'
import type { SessionSkills } from './gen/SessionSkills.js'
import type { ChangeScope } from './gen/ChangeScope.js'
import type { ChangesPage } from './gen/ChangesPage.js'
import type { FileDiff } from './gen/FileDiff.js'
import type { SearchPage } from './gen/SearchPage.js'
import type { TreePage } from './gen/TreePage.js'
import type { WorkspaceFile } from './gen/WorkspaceFile.js'
import type { WorkspaceInfo } from './gen/WorkspaceInfo.js'
import type { WrittenFile } from './gen/WrittenFile.js'
import { defaultWebSocketFactory, eventStream, type StreamOptions, type WebSocketFactory } from './stream.js'

export interface ClientOptions {
  /** Basis-URL des Servers, z. B. `http://127.0.0.1:7420`. */
  baseUrl: string
  /** Lokales Token (Node, CLI-Werkzeuge). Im Browser authentisiert das Session-Cookie. */
  token?: string
  fetch?: typeof fetch
  webSocketFactory?: WebSocketFactory
}

/** Optionen für `GET /v1/sessions`. */
export interface SessionListOptions {
  limit?: number
  cursor?: string
  includeArchived?: boolean
  /** Nur Sessions, die sich danach für den User geändert haben (Listen-Deltas, WEB-003). */
  updatedAfter?: string
  filter?: 'own' | 'shared' | 'archived' | 'all'
  /** Volltext über Titel und Nachrichten. */
  q?: string
  harness?: string
  status?: string
  projectId?: string
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
    list: (opts: SessionListOptions = {}): Promise<SessionPage> => {
      const q = new URLSearchParams()
      if (opts.limit !== undefined) q.set('limit', String(opts.limit))
      if (opts.cursor) q.set('cursor', opts.cursor)
      if (opts.includeArchived) q.set('include_archived', 'true')
      // Segment, Filter und Volltextsuche (SES-012).
      if (opts.filter) q.set('filter', opts.filter)
      if (opts.q) q.set('q', opts.q)
      if (opts.harness) q.set('harness', opts.harness)
      if (opts.status) q.set('status', opts.status)
      if (opts.projectId) q.set('project_id', opts.projectId)
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
  async request<T>(method: string, path: string, body?: unknown, extraHeaders: Record<string, string> = {}): Promise<T> {
    const headers: Record<string, string> = { accept: 'application/json', ...extraHeaders }
    if (this.token) headers.authorization = `Bearer ${this.token}`
    if (body !== undefined) headers['content-type'] = 'application/json'
    const res = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? null : JSON.stringify(body),
      credentials: 'include',
    })
    return (await this.parse(res)) as T
  }

  /** Rohdaten senden (z. B. Anhänge, WEB-006); Antwort als JSON. */
  async requestRaw<T>(method: string, path: string, body: Blob | ArrayBuffer | Uint8Array, contentType: string): Promise<T> {
    const headers: Record<string, string> = { accept: 'application/json', 'content-type': contentType }
    if (this.token) headers.authorization = `Bearer ${this.token}`
    const res = await this.fetchImpl(`${this.baseUrl}${path}`, { method, headers, body: body as BodyInit, credentials: 'include' })
    return (await this.parse(res)) as T
  }

  /** Binärinhalt per `GET` (z. B. Blobs einer Session). */
  async fetchBlob(path: string): Promise<Blob> {
    const headers: Record<string, string> = {}
    if (this.token) headers.authorization = `Bearer ${this.token}`
    const res = await this.fetchImpl(`${this.baseUrl}${path}`, { method: 'GET', headers, credentials: 'include' })
    if (!res.ok) await this.parse(res)
    return res.blob()
  }

  private async parse(res: Response): Promise<unknown> {
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
    return text ? JSON.parse(text) : undefined
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

  /**
   * Eingabe: sofort oder eingereiht; mit `steer` in den laufenden Turn (SES-004). Anhänge
   * vorher mit {@link uploadAttachment} hochladen (WEB-006).
   */
  send(text: string, mode?: InputMode, attachments?: Attachment[]): Promise<InputAccepted> {
    const body: { text: string; mode?: InputMode; attachments?: Attachment[] } = { text }
    if (mode) body.mode = mode
    if (attachments && attachments.length > 0) body.attachments = attachments
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/input`, body)
  }

  /** Anhang in den Blob-Store der Session laden (WEB-006); Ergebnis für {@link send}. */
  uploadAttachment(data: Blob | ArrayBuffer | Uint8Array, name: string, mime: string): Promise<Attachment> {
    return this.client.requestRaw('POST', `/v1/sessions/${enc(this.id)}/attachments?name=${enc(name)}`, data, mime)
  }

  /** Inhalt eines Blobs der Session, z. B. die Vorschau eines Anhangs. */
  blobData(ref: string): Promise<Blob> {
    return this.client.fetchBlob(`/v1/sessions/${enc(this.id)}/blobs/${enc(ref)}`)
  }

  /** Aufrufbare Skills des aktiven Agents für das Slash-Menü (WEB-006). */
  skills(): Promise<SessionSkills> {
    return this.client.request('GET', `/v1/sessions/${enc(this.id)}/skills`)
  }

  /** Titel setzen (UX-009); danach erzeugt beton keinen Titel mehr. */
  rename(title: string): Promise<SessionSummary> {
    return this.update({ title })
  }

  /**
   * URL des read-only SSE-Stroms (PROTO-012, API-003) für Werkzeuge ohne WebSocket, z. B.
   * `curl -N -H "Authorization: Bearer …" <url>`.
   */
  eventStreamUrl(fromSeq = 0, transient = false): string {
    return `${this.client.baseUrl}/v1/sessions/${enc(this.id)}/events/stream?from_seq=${fromSeq}${transient ? '&transient=true' : ''}`
  }

  /** Serverseitige Queue (SES-004). */
  queue(): Promise<QueueView> {
    return this.client.request('GET', `/v1/sessions/${enc(this.id)}/queue`)
  }

  editQueued(itemId: string, text: string): Promise<void> {
    return this.client.request('PATCH', `/v1/sessions/${enc(this.id)}/queue/${enc(itemId)}`, { text })
  }

  deleteQueued(itemId: string): Promise<void> {
    return this.client.request('DELETE', `/v1/sessions/${enc(this.id)}/queue/${enc(itemId)}`)
  }

  /** Verschiebt einen Eintrag an `position` (0 = als Nächstes). */
  moveQueued(itemId: string, position: number): Promise<void> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/queue/${enc(itemId)}/move`, { position })
  }

  /** „Als Steer senden“: Eintrag in den laufenden Turn einspeisen. */
  steerQueued(itemId: string): Promise<InputAccepted> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/queue/${enc(itemId)}/steer`)
  }

  resumeQueue(): Promise<void> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/queue/resume`)
  }

  /** Gelesen bis `seq`, gilt auf allen Geräten (SES-012). */
  markRead(seq: number): Promise<SessionSummary> {
    return this.client.request('PUT', `/v1/sessions/${enc(this.id)}/read-state`, { seq })
  }

  pin(pinned: boolean): Promise<SessionSummary> {
    return this.client.request('PUT', `/v1/sessions/${enc(this.id)}/pin`, { pinned })
  }

  /** Kontext kompaktieren (SES-011); Ergebnis als `compaction.*`-Events. */
  compact(): Promise<unknown> {
    return this.client.request('POST', `/v1/sessions/${enc(this.id)}/compact`)
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

  /** Workspace der Session: Dateien, Suche, Änderungen und Diffs (SES-017, SES-018). */
  readonly workspace = {
    /** Überblick, z. B. ob der Workspace ein Git-Repository ist. */
    info: (): Promise<WorkspaceInfo> => this.client.request('GET', `${this.base()}/workspace`),
    /** Eine Ebene des Dateibaums (`path` leer = Wurzel). */
    tree: (path = '', opts: { cursor?: string; limit?: number } = {}): Promise<TreePage> => {
      const q = new URLSearchParams()
      if (path) q.set('path', path)
      if (opts.cursor) q.set('cursor', opts.cursor)
      if (opts.limit !== undefined) q.set('limit', String(opts.limit))
      const qs = q.toString()
      return this.client.request('GET', `${this.base()}/workspace/tree${qs ? `?${qs}` : ''}`)
    },
    /** Datei lesen; `sha256` ist zugleich das ETag für {@link writeFile}. */
    readFile: (path: string): Promise<WorkspaceFile> =>
      this.client.request('GET', `${this.base()}/workspace/files/${encPath(path)}`),
    /** URL für den Download (Dateien über 5 MiB). */
    downloadUrl: (path: string): string => `${this.client.baseUrl}${this.base()}/workspace/files/${encPath(path)}?download=true`,
    /**
     * Datei schreiben. Mit `ifMatch` (SHA-256 des zuletzt gelesenen Stands) nur, wenn sie
     * seitdem unverändert ist; sonst {@link BetonError} mit Status 412.
     */
    writeFile: (path: string, content: string, ifMatch?: string): Promise<WrittenFile> =>
      this.client.request('PUT', `${this.base()}/workspace/files/${encPath(path)}`, { content }, ifMatch ? { 'if-match': `"${ifMatch}"` } : {}),
    /**
     * Dateinamen- oder Inhaltssuche; `fuzzy` sucht Dateinamen unscharf aus dem Dateiindex und
     * sortiert nach Treffergüte (`@` im Composer, WEB-006), mit `total` = Dateien im Index.
     */
    search: (query: string, mode: 'name' | 'content' | 'fuzzy' = 'name', opts: { cursor?: string; limit?: number } = {}): Promise<SearchPage> => {
      const q = new URLSearchParams({ q: query, mode })
      if (opts.cursor) q.set('cursor', opts.cursor)
      if (opts.limit !== undefined) q.set('limit', String(opts.limit))
      return this.client.request('GET', `${this.base()}/workspace/search?${q.toString()}`)
    },
    /** Geänderte Dateien in einer Sicht; `turn` nur bei `scope=turn` (ohne: letzter Turn). */
    changes: (scope: ChangeScope, opts: { turn?: string; cursor?: string; limit?: number } = {}): Promise<ChangesPage> => {
      const q = new URLSearchParams({ scope })
      if (opts.turn) q.set('turn', opts.turn)
      if (opts.cursor) q.set('cursor', opts.cursor)
      if (opts.limit !== undefined) q.set('limit', String(opts.limit))
      return this.client.request('GET', `${this.base()}/workspace/changes?${q.toString()}`)
    },
    /** Zeilengenauer Diff einer Datei. */
    diff: (path: string, scope: ChangeScope, turn?: string): Promise<FileDiff> => {
      const q = new URLSearchParams({ path, scope })
      if (turn) q.set('turn', turn)
      return this.client.request('GET', `${this.base()}/workspace/diff?${q.toString()}`)
    },
  }

  private base(): string {
    return `/v1/sessions/${enc(this.id)}`
  }

  /**
   * Neue Session ab einem Event abzweigen (SES-006), optional auf einem anderen Harness
   * (SES-007). Ohne `at_seq` ab dem Ende; mitten in einem Turn beginnt der Fork am
   * vorherigen Turn-Ende (`effectiveSeq`).
   */
  async fork(options: ForkRequest = {}): Promise<Forked> {
    const res: ForkResponse = await this.client.request('POST', `/v1/sessions/${enc(this.id)}/fork`, options)
    return {
      session: new Session(this.client, res.session.id),
      summary: res.session,
      effectiveSeq: res.effective_seq,
      workspace: res.workspace,
    }
  }
}

/** Ergebnis von `Session.fork()`. */
export interface Forked {
  session: Session
  summary: SessionSummary
  /** Tatsächlicher Fork-Punkt in der Quelle. */
  effectiveSeq: number
  workspace: ForkWorkspace
}

function enc(s: string): string {
  return encodeURIComponent(s)
}

/** Workspace-Pfad: jedes Segment einzeln kodiert, `/` bleibt Trenner. */
function encPath(path: string): string {
  return path.split('/').map(enc).join('/')
}
