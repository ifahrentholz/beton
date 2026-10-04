import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useQuery } from '@tanstack/react-query'
import type { Capabilities, HarnessInfo } from '@beton/sdk'
import { Menu, PanelRight, Paperclip, WifiOff, X } from 'lucide-react'
import { useSessionStream } from '@/hooks/useSessionStream'
import { client } from '@/lib/client'
import { payloadOf } from '@/lib/events'
import { harnessVisible, useFeatures } from '@/lib/features'
import { cost } from '@/lib/format'
import { queueOf } from '@/lib/queue'
import { cn } from '@/lib/utils'
import { openApprovals, timeline, type Item } from '@/lib/timeline'
import { changedPaths, lineLink, rangeLabel, withAttachments, type WorkspaceSearch } from '@/lib/workspace'
import { useEvents } from '@/store/events'
import { useSessions } from '@/store/sessions'
import { clearAttachments, detach, refresh, useSessionWorkspace, useWorkspace } from '@/store/workspace'
import { Composer } from './composer'
import { continueTargets, ForkBanner, forkOrigin } from './fork'
import { QueueList } from './queue'
import { HarnessBadge, listStatus, StatusMark } from './harness'
import { useLayout } from './layout-state'
import { ConflictHost } from './workspace/editor'
import { WorkspaceRail } from './workspace/rail'
import { AgentMessage, ApprovalCard, ErrorCard, Offloaded, PolicyCard, Reasoning, SystemNote, ToolCard, UserMessage } from './stream'

/** Zwei `Esc` binnen dieser Zeit unterbrechen den Turn. */
export const ESC_ESC_MS = 600

/** Metadaten aus dem Log: Harness, Modell, Capabilities, Titel, Status. */
function useMeta(sessionId: string) {
  const events = useEvents((s) => s.logs[sessionId]?.events)
  const summary = useSessions((s) => s.byId[sessionId])
  return useMemo(() => {
    let harness = summary?.harness ?? ''
    let model: string | null | undefined
    let capabilities: Capabilities | undefined
    let title = summary?.title ?? ''
    let status = summary?.status ?? 'starting'
    for (const e of events ?? []) {
      const created = payloadOf(e, 'session.created')
      if (created) {
        harness = created.harness
        model = created.model
      }
      const started = payloadOf(e, 'session.started')
      if (started) capabilities = started.capabilities as unknown as Capabilities
      const t = payloadOf(e, 'session.title_changed')
      if (t) title = t.title
      const st = payloadOf(e, 'session.status')
      if (st) status = st.status
      const settings = payloadOf(e, 'session.settings_changed')
      if (settings?.model) model = settings.model
    }
    return { harness, model, capabilities, title, status }
  }, [events, summary])
}

function StreamItem({ item, sessionId, harness }: { item: Item; sessionId: string; harness: string }) {
  switch (item.kind) {
    case 'user':
      return <UserMessage text={item.text} />
    case 'assistant':
      return <AgentMessage harness={harness} text={item.text} streaming={item.streaming} />
    case 'reasoning':
      return <Reasoning text={item.text} streaming={item.streaming} />
    case 'tool':
      return <ToolCard item={item} />
    case 'approval':
      return <ApprovalCard sessionId={sessionId} item={item} />
    case 'policy':
      return <PolicyCard rule={item.rule} reason={item.reason} />
    case 'error':
      return <ErrorCard title={item.title} detail={item.detail} />
    case 'note':
      return <SystemNote text={item.text} tone={item.tone} />
    case 'offloaded':
      return <Offloaded sessionId={sessionId} item={item} harness={harness} />
  }
}

/**
 * Geöffnete Dateien folgen `fs.changed`: neu laden oder – bei ungespeicherten Änderungen –
 * Konflikt melden (WEB-009 AC1).
 */
function useOpenFileSync(sessionId: string): void {
  const events = useEvents((s) => s.logs[sessionId]?.events)
  const processed = useRef(0)
  useEffect(() => {
    const last = events?.at(-1)?.seq ?? 0
    if (last <= processed.current) return
    const paths = changedPaths(events, processed.current)
    processed.current = last
    const open = useWorkspace.getState().ws(sessionId).files
    for (const p of paths) if (open[p]) void refresh(sessionId, p)
  }, [sessionId, events])
}

/** Referenzen für den Agent über dem Eingabefeld (WEB-009). */
function AttachmentChips({ sessionId }: { sessionId: string }) {
  const attachments = useSessionWorkspace(sessionId).attachments
  if (attachments.length === 0) return null
  return (
    <div className="flex flex-wrap items-center gap-1.5 px-4 pt-2" data-testid="attachments">
      {attachments.map((a) => (
        <span key={a.id} className="inline-flex items-center gap-1.5 rounded-md border border-border bg-card py-0.5 pr-1 pl-2 text-[12px]">
          <Paperclip className="size-3 text-muted-foreground" />
          <span className="font-mono">{a.path}</span>
          <span className="text-muted-foreground">{rangeLabel(a.from, a.to)}</span>
          <button onClick={() => detach(sessionId, a.id)} aria-label="Referenz entfernen" className="rounded-sm text-muted-foreground hover:text-foreground">
            <X className="size-3" />
          </button>
        </span>
      ))}
      <span className="text-[11px] text-muted-foreground">Ausschnitt wird mitgeschickt</span>
    </div>
  )
}

const desktop = () => window.matchMedia('(min-width: 1024px)').matches

export function SessionView({ sessionId }: { sessionId: string }) {
  useSessionStream(sessionId)
  useOpenFileSync(sessionId)
  const search = useSearch({ strict: false }) as WorkspaceSearch
  const navigate = useNavigate()
  const railOpen = useWorkspace((s) => s.railOpen)
  const railSheet = useWorkspace((s) => s.railSheet)
  const worktree = useSessions((s) => s.byId[sessionId]?.worktree)
  const readSeq = useSessions((s) => s.byId[sessionId]?.read_seq)
  const log = useEvents((s) => s.logs[sessionId])
  const meta = useMeta(sessionId)
  const items = useMemo(
    () => (log ? timeline(log) : []),
    [log],
  )
  const waiting = openApprovals(items).length > 0
  const running = meta.status === 'running' || meta.status === 'waiting_approval'
  const queue = useMemo(() => queueOf(log?.events), [log?.events])
  const [sendError, setSendError] = useState<string | undefined>()
  const toggleList = useLayout((s) => s.toggleList)
  const costMicro = useSessions((s) => s.byId[sessionId]?.cost_micro ?? 0)
  const lastSeq = log?.events.at(-1)?.seq ?? 0
  const origin = useMemo(() => forkOrigin(log?.events), [log?.events])
  const features = useFeatures()
  const catalog = useQuery({
    queryKey: ['harnesses'],
    queryFn: () => client.request<{ items: HarnessInfo[] }>('GET', '/v1/harnesses'),
  })
  const targets = continueTargets(catalog.data?.items ?? [], meta.harness, (h) => harnessVisible(h, features))
  // „Weiter mit …“: Fork ab dem letzten `seq`, die neue Session öffnet direkt (SES-007 AC5).
  const continueWith = (harness: string) => {
    setSendError(undefined)
    client
      .session(sessionId)
      .fork({ harness })
      .then((f) => {
        useSessions.getState().upsert([f.summary])
        void navigate({ to: '/s/$sessionId', params: { sessionId: f.session.id } })
      })
      .catch((e: unknown) => setSendError(e instanceof Error ? e.message : 'Fork fehlgeschlagen'))
  }

  // Gelesen bis zur angezeigten `seq`, auf allen Geräten (SES-012 AC2). Während des Replays
  // wartet die Meldung, bis keine neuen Events mehr nachkommen.
  useEffect(() => {
    if (lastSeq === 0) return
    if ((useSessions.getState().byId[sessionId]?.read_seq ?? 0) >= lastSeq) return
    const timer = setTimeout(() => {
      if (document.visibilityState !== 'visible') return
      client
        .session(sessionId)
        .markRead(lastSeq)
        .then((s) => useSessions.getState().upsert([s]))
        .catch(() => undefined)
    }, 300)
    return () => clearTimeout(timer)
  }, [sessionId, lastSeq])

  // `Esc` zweimal kurz hintereinander unterbricht den laufenden Turn (WEB-005, UX-004).
  useEffect(() => {
    if (!running) return
    let last = 0
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return
      const now = Date.now()
      if (now - last < ESC_ESC_MS) {
        last = 0
        void client.session(sessionId).interrupt().catch(() => undefined)
      } else {
        last = now
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [running, sessionId])

  const scrollRef = useRef<HTMLDivElement>(null)
  const atBottom = useRef(true)
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 64,
    overscan: 8,
    getItemKey: (i) => items[i]?.key ?? i,
  })
  useEffect(() => {
    if (atBottom.current && items.length > 0) virtualizer.scrollToIndex(items.length - 1, { align: 'end' })
  }, [items, virtualizer])

  // Ein geteilter Link auf eine Diff-Zeile öffnet die Rail auch auf kleinen Bildschirmen.
  useEffect(() => {
    if (!search.tab) return
    if (desktop()) {
      if (!useWorkspace.getState().railOpen) useWorkspace.getState().setRailOpen(true)
    } else useWorkspace.getState().setRailSheet(true)
  }, [search.tab])

  const toggleRail = () => {
    const st = useWorkspace.getState()
    if (desktop()) st.setRailOpen(!st.railOpen)
    else st.setRailSheet(!st.railSheet)
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <header className="@container flex h-12 shrink-0 items-center gap-3 border-b border-border px-3 sm:px-4">
        <button onClick={toggleList} className="flex size-8 items-center justify-center rounded-md hover:bg-accent lg:hidden" aria-label="Sessions anzeigen">
          <Menu className="size-4" />
        </button>
        <StatusMark status={waiting ? 'waiting' : listStatus(meta.status)} />
        <h2 className="min-w-0 flex-1 truncate text-[15px] font-semibold" title={meta.title}>
          {meta.title || 'Neue Session'}
        </h2>
        {meta.harness && <HarnessBadge harness={meta.harness} model={meta.model} className="hidden shrink-0 sm:inline-flex" />}
        {costMicro > 0 && <span className="hidden text-[11px] text-muted-foreground tabular-nums md:inline">{cost(costMicro)}</span>}
        <button
          onClick={toggleRail}
          aria-label="Workspace"
          aria-pressed={railOpen}
          title="Workspace ein- oder ausblenden"
          className={cn('flex size-8 shrink-0 items-center justify-center rounded-md hover:bg-accent', (railOpen || railSheet) && 'text-foreground')}
        >
          <PanelRight className="size-4" />
        </button>
      </header>
      {origin && <ForkBanner origin={origin} />}
      {log?.reconnecting && (
        <div className="flex items-center gap-2 border-b border-border bg-sunken px-4 py-1 text-[12px] text-muted-foreground" role="status">
          <WifiOff className="size-3.5" /> Verbindung zum Daemon unterbrochen – verbinde neu …
        </div>
      )}
      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto"
        onScroll={(e) => {
          const el = e.currentTarget
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80
        }}
        data-testid="stream"
      >
        <div className="relative mx-auto w-full max-w-3xl" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((v) => {
            const item = items[v.index]
            if (!item) return null
            return (
              <div key={v.key} data-index={v.index} ref={virtualizer.measureElement} className="absolute top-0 left-0 w-full px-3 py-2 sm:px-6" style={{ transform: `translateY(${v.start}px)` }}>
                <StreamItem item={item} sessionId={sessionId} harness={meta.harness} />
              </div>
            )
          })}
        </div>
        {items.length === 0 && (
          <p className="mx-auto max-w-3xl px-6 py-10 text-[13px] text-muted-foreground">
            {meta.status === 'starting' ? 'Session startet …' : 'Noch keine Nachrichten. Schreib unten, was der Agent tun soll.'}
          </p>
        )}
      </div>
      {sendError && (
        <div className="border-t border-deny/40 bg-deny-soft px-4 py-1.5 text-[12px]" role="alert">
          {sendError}
        </div>
      )}
      <AttachmentChips sessionId={sessionId} />
      <Composer
        sessionId={sessionId}
        harness={meta.harness || 'claude'}
        model={meta.model}
        capabilities={meta.capabilities}
        running={running}
        continueWith={targets}
        onContinue={meta.harness ? continueWith : undefined}
        queue={
          <QueueList
            sessionId={sessionId}
            items={queue.items}
            harness={meta.harness}
            steering={meta.capabilities?.steering ?? false}
            running={running}
            onError={setSendError}
          />
        }
        onSend={(text) => {
          setSendError(undefined)
          // Der Server entscheidet: sofort starten oder einreihen (SES-004).
          const attachments = useWorkspace.getState().ws(sessionId).attachments
          clearAttachments(sessionId)
          client
            .session(sessionId)
            .send(withAttachments(text, attachments))
            .catch((e: unknown) => setSendError(e instanceof Error ? e.message : 'Senden fehlgeschlagen'))
        }}
        onInterrupt={() => void client.session(sessionId).interrupt().catch(() => undefined)}
      />
    </div>
      <WorkspaceRail
        sessionId={sessionId}
        events={log?.events}
        capabilities={meta.capabilities}
        worktree={worktree}
        readSeq={readSeq}
        search={search}
        onAnchor={({ scope, turn, file, line }) =>
          void navigate({ to: '/s/$sessionId', params: { sessionId }, search: { tab: 'changes', scope, turn, file, line }, replace: true })
        }
        copyLink={async (p) => {
          await navigator.clipboard?.writeText(lineLink(window.location.origin, sessionId, { scope: p.scope, turn: p.turn, file: p.file, anchor: p.line }))
        }}
      />
      <ConflictHost sessionId={sessionId} harness={meta.harness} />
    </div>
  )
}
