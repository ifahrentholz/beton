import { useEffect, useMemo, useRef, useState } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import type { Capabilities, HarnessInfo } from '@beton/sdk'
import { Menu, WifiOff } from 'lucide-react'
import { useSessionStream } from '@/hooks/useSessionStream'
import { client } from '@/lib/client'
import { payloadOf } from '@/lib/events'
import { harnessVisible, useFeatures } from '@/lib/features'
import { cost } from '@/lib/format'
import { queueOf } from '@/lib/queue'
import { openApprovals, timeline, type Item } from '@/lib/timeline'
import { useEvents } from '@/store/events'
import { useSessions } from '@/store/sessions'
import { Composer } from './composer'
import { continueTargets, ForkBanner, forkOrigin } from './fork'
import { QueueList } from './queue'
import { HarnessBadge, listStatus, StatusMark } from './harness'
import { useLayout } from './layout-state'
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

export function SessionView({ sessionId }: { sessionId: string }) {
  useSessionStream(sessionId)
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
  const navigate = useNavigate()
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

  return (
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
          client
            .session(sessionId)
            .send(text)
            .catch((e: unknown) => setSendError(e instanceof Error ? e.message : 'Senden fehlgeschlagen'))
        }}
        onInterrupt={() => void client.session(sessionId).interrupt().catch(() => undefined)}
      />
    </div>
  )
}
