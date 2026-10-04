import { useEffect, useMemo, useRef, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { SessionSummary } from '@beton/sdk'
import { Archive, Plus, Search } from 'lucide-react'
import { client } from '@/lib/client'
import { ago } from '@/lib/format'
import { cn } from '@/lib/utils'
import { sortSessions, useSessions } from '@/store/sessions'
import { harnessName, listStatus, StatusMark, VoiceDot, voiceOf } from './harness'
import { useLayout } from './layout-state'

/** Wartezeit nach dem Tippen, bevor die Volltextsuche läuft. */
const SEARCH_DEBOUNCE_MS = 200

type Row = { kind: 'heading'; label: string } | { kind: 'session'; session: SessionSummary }

/**
 * Zeilen der Liste: angepinnte Sessions als eigene Gruppe oben (SES-012 AC3), danach alle
 * übrigen nach jüngster Aktivität. Bei einer Suche nur die Treffer des Servers.
 */
export function listRows(sessions: SessionSummary[], archived: boolean, hits?: Set<string>): Row[] {
  const list = sortSessions(sessions).filter((s) => s.archived === archived && (!hits || hits.has(s.id)))
  const pinned = list.filter((s) => s.pinned)
  if (pinned.length === 0) return list.map((session) => ({ kind: 'session', session }))
  return [
    { kind: 'heading', label: 'Angepinnt' },
    ...pinned.map((session) => ({ kind: 'session' as const, session })),
    ...list.filter((s) => !s.pinned).map((session) => ({ kind: 'session' as const, session })),
  ]
}

/** Treffer der Volltextsuche über Titel und Nachrichten (SES-012). */
function useSearch(query: string, archived: boolean): Set<string> | undefined {
  const [hits, setHits] = useState<Set<string> | undefined>()
  useEffect(() => {
    const q = query.trim()
    if (!q) {
      setHits(undefined)
      return
    }
    let cancelled = false
    const timer = setTimeout(() => {
      client.sessions
        .list({ q, limit: 200, filter: archived ? 'archived' : undefined })
        .then((page) => {
          if (cancelled) return
          useSessions.getState().upsert(page.items)
          setHits(new Set(page.items.map((s) => s.id)))
        })
        .catch(() => undefined)
    }, SEARCH_DEBOUNCE_MS)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [query, archived])
  return query.trim() ? hits : undefined
}

/** Linke Spalte (WEB-003): Sessions nach Aktivität, angepinnte oben, Suche, Archiv; virtualisiert. */
export function SessionList({ active, onNew }: { active?: string | undefined; onNew: () => void }) {
  const byId = useSessions((s) => s.byId)
  const loaded = useSessions((s) => s.loaded)
  const [query, setQuery] = useState('')
  const [archived, setArchived] = useState(false)
  const closeList = useLayout((s) => s.closeList)
  const hits = useSearch(query, archived)
  const rows = useMemo(() => listRows(Object.values(byId), archived, hits), [byId, archived, hits])
  const scrollRef = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => (rows[i]?.kind === 'heading' ? 28 : 48),
    overscan: 12,
    getItemKey: (i) => {
      const r = rows[i]
      return r?.kind === 'session' ? r.session.id : `h:${r?.label ?? i}`
    },
  })
  const sessionCount = rows.filter((r) => r.kind === 'session').length
  return (
    <nav aria-label="Sessions" className="flex h-full w-full flex-col bg-sidebar">
      <div className="flex items-center gap-1 p-2">
        <label className="flex h-8 flex-1 items-center gap-2 rounded-md border border-input bg-card px-2 text-xs text-muted-foreground">
          <Search className="size-3.5 shrink-0" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Sessions durchsuchen"
            aria-label="Sessions durchsuchen"
            className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground"
          />
        </label>
        <button onClick={onNew} title="Neue Session" aria-label="Neue Session" className="flex size-8 items-center justify-center rounded-md bg-foreground text-background">
          <Plus className="size-4" />
        </button>
      </div>
      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto pb-2" data-testid="session-list">
        <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((v) => {
            const row = rows[v.index]
            if (!row) return null
            if (row.kind === 'heading') {
              return (
                <div
                  key={v.key}
                  data-testid="session-group"
                  className="absolute left-0 flex h-7 w-full items-end px-3 pb-1 text-xs font-semibold"
                  style={{ transform: `translateY(${v.start}px)` }}
                >
                  {row.label}
                </div>
              )
            }
            const s = row.session
            return (
              <Link
                key={v.key}
                to="/s/$sessionId"
                params={{ sessionId: s.id }}
                onClick={closeList}
                data-testid="session-row"
                data-session={s.id}
                data-pinned={s.pinned || undefined}
                data-unread={s.unread || undefined}
                className={cn(
                  'absolute left-0 flex h-12 w-full items-center gap-2 border-l-2 px-3',
                  s.id === active ? 'border-signal bg-accent' : 'border-transparent hover:bg-accent/60',
                )}
                style={{ transform: `translateY(${v.start}px)` }}
              >
                <StatusMark status={listStatus(s.status)} />
                <div className="min-w-0 flex-1">
                  <div className={cn('truncate text-[13px]', s.unread && 'font-semibold')}>
                    {s.title || 'Neue Session'}
                    {s.unread && <span className="sr-only"> (ungelesen)</span>}
                  </div>
                  <div className="flex items-center gap-1.5 overflow-hidden text-[11px] whitespace-nowrap text-muted-foreground">
                    <VoiceDot voice={voiceOf(s.harness)} className="size-1.5" />
                    <span>{harnessName(s.harness)}</span>
                    <span className="ml-auto shrink-0 pl-1">{ago(s.last_activity_at)}</span>
                  </div>
                </div>
              </Link>
            )
          })}
        </div>
        {loaded && sessionCount === 0 && (
          <p className="px-3 py-4 text-[12px] text-muted-foreground">
            {query ? 'Keine Session passt zur Suche.' : archived ? 'Keine archivierten Sessions.' : 'Noch keine Sessions. Starte eine mit dem Plus oder `beton run`.'}
          </p>
        )}
      </div>
      <button
        onClick={() => setArchived((v) => !v)}
        className="flex items-center gap-2 border-t border-border px-3 py-2 text-[12px] text-muted-foreground hover:text-foreground"
        aria-pressed={archived}
      >
        <Archive className="size-3.5" /> {archived ? 'Aktive Sessions' : 'Archiv'}
      </button>
    </nav>
  )
}
