import { create } from 'zustand'
import { BetonError, type SessionSummary } from '@beton/sdk'
import { client } from '@/lib/client'

/** Abstand der Delta-Abfragen; Statuswechsel erscheinen so binnen 1 s (WEB-003 AC1). */
export const POLL_MS = 500

interface SessionsState {
  byId: Record<string, SessionSummary>
  loaded: boolean
  /** Server antwortet 401: Browser nicht angemeldet. */
  unauthorized: boolean
  /** Daemon nicht erreichbar. */
  offline: boolean
  upsert: (items: SessionSummary[]) => void
  remove: (id: string) => void
}

export const useSessions = create<SessionsState>()((set) => ({
  byId: {},
  loaded: false,
  unauthorized: false,
  offline: false,
  upsert: (items) =>
    set((s) => {
      if (items.length === 0) return s
      const byId = { ...s.byId }
      for (const it of items) byId[it.id] = it
      return { byId }
    }),
  remove: (id) =>
    set((s) => {
      const byId = { ...s.byId }
      delete byId[id]
      return { byId }
    }),
}))

/** Sortierung der Liste: jüngste Aktivität zuerst. */
export function sortSessions(list: SessionSummary[]): SessionSummary[] {
  return [...list].sort((a, b) =>
    a.last_activity_at === b.last_activity_at ? (a.id < b.id ? 1 : -1) : a.last_activity_at < b.last_activity_at ? 1 : -1,
  )
}

function noteError(e: unknown): void {
  if (e instanceof BetonError && e.status === 401) useSessions.setState({ unauthorized: true })
  else if (!(e instanceof BetonError)) useSessions.setState({ offline: true })
}

/** Lädt alle Sessions (auch archivierte) und hält die Liste über Deltas aktuell. */
export function startSessionSync(): () => void {
  let stopped = false
  let since = ''
  let timer: ReturnType<typeof setTimeout> | undefined
  const track = (items: SessionSummary[]) => {
    for (const it of items) if (it.last_activity_at > since) since = it.last_activity_at
  }
  const poll = async () => {
    if (stopped) return
    try {
      const page = await client.sessions.list({ limit: 200, updatedAfter: since })
      useSessions.getState().upsert(page.items)
      track(page.items)
      useSessions.setState({ offline: false })
    } catch (e) {
      noteError(e)
    }
    if (!stopped) timer = setTimeout(poll, POLL_MS)
  }
  const load = async () => {
    try {
      let cursor: string | undefined
      const all: SessionSummary[] = []
      do {
        const page = await client.sessions.list({ limit: 200, includeArchived: true, ...(cursor ? { cursor } : {}) })
        all.push(...page.items)
        cursor = page.next_cursor ?? undefined
      } while (cursor && !stopped)
      useSessions.getState().upsert(all)
      track(all)
      useSessions.setState({ loaded: true, offline: false })
      if (!since) since = new Date(0).toISOString()
      timer = setTimeout(poll, POLL_MS)
    } catch (e) {
      noteError(e)
      if (!stopped) timer = setTimeout(load, 2000)
    }
  }
  void load()
  return () => {
    stopped = true
    if (timer) clearTimeout(timer)
  }
}
