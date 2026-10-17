import { useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useQuery } from '@tanstack/react-query'
import type { Capabilities, HarnessInfo, PermissionMode, SessionSandbox } from '@beton/sdk'
import { GitFork, Link2, Menu, Minimize2, PanelRight, Paperclip, Square, WifiOff, X } from 'lucide-react'
import { useSessionStream } from '@/hooks/useSessionStream'
import { client } from '@/lib/client'
import { tk, useCommand, useCommands } from '@/lib/commands'
import { compacting as compactingOf, contextLevel, contextOf } from '@/lib/context'
import { payloadOf } from '@/lib/events'
import { harnessVisible, useFeatures } from '@/lib/features'
import { cost } from '@/lib/format'
import { queueOf } from '@/lib/queue'
import { cn } from '@/lib/utils'
import { openApprovals, timeline, type ApprovalItem, type Item } from '@/lib/timeline'
import { titleFrom, windowTitle } from '@/lib/title'
import { indexForSeq, linkOf, type SessionSearch } from '@/lib/deeplink'
import { useMru } from '@/lib/mru'
import { changedPaths, lineLink, rangeLabel, withAttachments } from '@/lib/workspace'
import { useEvents } from '@/store/events'
import { useSessions } from '@/store/sessions'
import { clearAttachments, detach, refresh, useSessionWorkspace, useWorkspace } from '@/store/workspace'
import { Composer, type BetonCommand } from './composer'
import { continueTargets, ForkBanner, forkOrigin } from './fork'
import { QueueList } from './queue'
import { HarnessBadge, harnessName, listStatus, StatusMark, VoiceDot, voiceOf } from './harness'
import { useLayout } from './layout-state'
import { SessionTitle } from './session-title'
import { ConflictHost } from './workspace/editor'
import { WorkspaceRail } from './workspace/rail'
import { ApprovalBar } from './approval-bar'
import { ContextBar } from './context-bar'
import { WindowDrop } from './window-drop'
import { StreamAnnouncer } from './stream-announcer'
import { AgentMessage, ApprovalCard, ErrorCard, Offloaded, PolicyCard, Reasoning, SystemNote, ToolCard, UserMessage } from './stream'

/** Metadaten aus dem Log: Harness, Modell, Capabilities, Titel, Status. */
function useMeta(sessionId: string) {
  const events = useEvents((s) => s.logs[sessionId]?.events)
  const summary = useSessions((s) => s.byId[sessionId])
  return useMemo(() => {
    let harness = summary?.harness ?? ''
    let model: string | null | undefined
    let effort: string | null | undefined
    let permissionMode: PermissionMode | null | undefined
    let capabilities: Capabilities | undefined
    let sandbox: SessionSandbox | undefined
    let status = summary?.status ?? 'starting'
    for (const e of events ?? []) {
      const created = payloadOf(e, 'session.created')
      if (created) {
        harness = created.harness
        model = created.model
        effort = created.effort
        permissionMode = created.permission_mode as PermissionMode | undefined
      }
      const started = payloadOf(e, 'session.started')
      if (started) {
        capabilities = started.capabilities as unknown as Capabilities
        sandbox = started.sandbox
      }
      const st = payloadOf(e, 'session.status')
      if (st) status = st.status
      // Wirksame Einstellungen: der letzte Stand aus `session.settings_changed` (HAR-017, HAR-027).
      const settings = payloadOf(e, 'session.settings_changed')
      if (settings?.model) model = settings.model
      if (settings?.effort) effort = settings.effort
      if (settings?.permission_mode) permissionMode = settings.permission_mode as PermissionMode
    }
    // Titel des Users gewinnt gegen später erzeugte (UX-009 AC2).
    const { title, source } = titleFrom(summary, events)
    return { harness, model, effort, permissionMode, capabilities, sandbox, title, titleSource: source, status }
  }, [events, summary])
}

function StreamItem({ item, sessionId, harness }: { item: Item; sessionId: string; harness: string }) {
  switch (item.kind) {
    case 'user':
      return <UserMessage text={item.text} sessionId={sessionId} attachments={item.attachments} />
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
      return <SystemNote text={item.text} tone={item.tone} detail={item.detail} />
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
  const search = useSearch({ strict: false }) as SessionSearch
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
  const approvals = useMemo(() => items.filter((i): i is ApprovalItem => i.kind === 'approval'), [items])
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

  // Titel im Browser-Tab bzw. Fenster (UX-009 AC1).
  useEffect(() => {
    document.title = windowTitle(meta.title)
  }, [meta.title])
  const rename = (title: string) =>
    client
      .session(sessionId)
      .rename(title)
      .then((s) => useSessions.getState().upsert([s]))
  // Kontext und Compaction (USE-008, USE-009): letzter `context.usage`, laufende Compaction.
  const context = useMemo(() => contextOf(log?.events), [log?.events])
  const isCompacting = useMemo(() => compactingOf(log?.events), [log?.events])
  const compact = () => {
    setSendError(undefined)
    client
      .session(sessionId)
      .compact()
      .catch((e: unknown) => setSendError(e instanceof Error ? e.message : 'Kompaktieren fehlgeschlagen'))
  }
  // beton-eigene Slash-Befehle, die es schon gibt (WEB-006): `/compact` (SES-011) und `/fork`
  // (SES-006).
  const compaction = meta.capabilities?.compaction
  const commands: BetonCommand[] = []
  if (compaction && compaction !== 'none' && !running && !isCompacting) {
    commands.push({
      name: 'compact',
      title: 'Kontext komprimieren',
      description: 'Verlauf vom Harness zusammenfassen lassen und Kontext freigeben',
      run: compact,
    })
  }
  if (meta.harness) {
    commands.push({
      name: 'fork',
      title: 'Forken',
      description: 'Ab hier eine neue Session abzweigen, auch auf einem anderen Harness',
      run: () => continueWith(meta.harness),
    })
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

  // Zuletzt genutzt, je Gerät (UX-003 AC3).
  useEffect(() => useMru.getState().touch(sessionId), [sessionId])

  // Befehle dieser Session (UX-002): Palette und Kürzel aus der Registry. `Esc Esc` unterbricht
  // nur bei laufendem Turn (WEB-005, UX-004; `when`, UX-002 AC2).
  useCommand({
    id: 'session.interrupt',
    icon: Square,
    when: () => running,
    run: () => void client.session(sessionId).interrupt().catch(() => undefined),
  })
  useCommand({ id: 'session.fork', icon: GitFork, when: () => !!meta.harness, run: () => continueWith(meta.harness) })
  const contextPct = context ? contextLevel(context.used, context.window).pct : null
  useCommand({
    id: 'session.compact',
    title: tk('cmd.compact'),
    hint: contextPct !== null ? (t) => t('cmd.compactHint', { percent: contextPct }) : '/compact',
    icon: Minimize2,
    group: 'session',
    order: 4,
    keywords: ['compact', 'kompaktieren', 'komprimieren', 'kontext', 'context'],
    when: () => !!compaction && compaction !== 'none' && !running && !isCompacting,
    run: compact,
  })
  useCommands(
    targets.map((h, i) => ({
      id: `session.continue.${h.id}`,
      title: (t) => t('cmd.continueWith', { harness: harnessName(h.id) }),
      hint: tk('cmd.continueHint'),
      icon: voiceIcon(voiceOf(h.id)),
      group: 'session',
      order: 1 + i / 100,
      keywords: ['fork', 'harness', 'weiter', 'continue', 'wechseln', 'switch'],
      run: () => continueWith(h.id),
    })),
  )

  const scrollRef = useRef<HTMLDivElement>(null)
  const atBottom = useRef(true)
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 64,
    overscan: 8,
    getItemKey: (i) => items[i]?.key ?? i,
  })
  // Sprung zu Event `seq` aus Deep-Link oder Benachrichtigung (DESK-006 AC3, DESK-005 AC2):
  // einmal, sobald das Event geladen ist; danach folgt die Ansicht nicht mehr dem Ende.
  const target = search.seq !== undefined ? indexForSeq(items, search.seq) : undefined
  const targetKey = target !== undefined ? items[target]?.key : undefined
  const jumped = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (target === undefined || search.seq === undefined || jumped.current === search.seq) return
    if (lastSeq < search.seq) return
    jumped.current = search.seq
    atBottom.current = false
    virtualizer.scrollToIndex(target, { align: 'center' })
  }, [target, search.seq, lastSeq, virtualizer])
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
    <WindowDrop sessionId={sessionId} title={meta.title || 'Neue Session'}>
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <header className="@container flex h-12 shrink-0 items-center gap-3 border-b border-border px-3 sm:px-4">
        <button onClick={toggleList} className="flex size-8 items-center justify-center rounded-md hover:bg-accent lg:hidden" aria-label="Sessions anzeigen">
          <Menu className="size-4" />
        </button>
        <StatusMark status={waiting ? 'waiting' : listStatus(meta.status)} />
        <SessionTitle title={meta.title} source={meta.titleSource} onRename={rename} />
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
      {search.opened === 'link' && <OpenedViaLink sessionId={sessionId} search={search} />}
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
                <div className={cn(item.key === targetKey && 'rounded-md outline-2 outline-offset-4 outline-foreground/60')} data-target={item.key === targetKey ? 'seq' : undefined}>
                  <StreamItem item={item} sessionId={sessionId} harness={meta.harness} />
                </div>
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
      <StreamAnnouncer items={items} />
      {sendError && (
        <div className="border-t border-deny/40 bg-deny-soft px-4 py-1.5 text-[12px]" role="alert">
          {sendError}
        </div>
      )}
      <ApprovalBar sessionId={sessionId} approvals={approvals} />
      <AttachmentChips sessionId={sessionId} />
      <ContextBar
        harness={meta.harness}
        capabilities={meta.capabilities}
        context={context}
        compacting={isCompacting}
        running={running}
        onCompact={compact}
      />
      <Composer
        sessionId={sessionId}
        harness={meta.harness || 'claude'}
        model={meta.model}
        effort={meta.effort}
        permissionMode={meta.permissionMode}
        capabilities={meta.capabilities}
        sandbox={meta.sandbox}
        running={running}
        continueWith={targets}
        onContinue={meta.harness ? continueWith : undefined}
        commands={commands}
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
        onSend={(text, files) => {
          setSendError(undefined)
          // Der Server entscheidet: sofort starten oder einreihen (SES-004).
          const attachments = useWorkspace.getState().ws(sessionId).attachments
          clearAttachments(sessionId)
          client
            .session(sessionId)
            .send(withAttachments(text, attachments), undefined, files)
            .catch((e: unknown) => setSendError(e instanceof Error ? e.message : 'Senden fehlgeschlagen'))
        }}
        onInterrupt={() => void client.session(sessionId).interrupt().catch(() => undefined)}
        onSettings={(patch) => {
          setSendError(undefined)
          // Der Wechsel erscheint als `session.settings_changed` im Log (HAR-017, HAR-027).
          client
            .session(sessionId)
            .update(patch)
            .catch((e: unknown) => setSendError(e instanceof Error ? e.message : 'Wechsel fehlgeschlagen'))
        }}
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
    </WindowDrop>
  )
}

/** Hinweis „Geöffnet über beton://…“ (Entwurf `desktop-deeplink` Zustand „Session an Event geöffnet“). */
export function OpenedViaLink({ sessionId, search }: { sessionId: string; search: SessionSearch }) {
  return (
    <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-card px-4 text-[12px]" role="status">
      <Link2 className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="shrink-0 text-muted-foreground">Geöffnet über</span>
      <code className="truncate font-mono text-[11.5px]">{linkOf(sessionId, search)}</code>
      {search.seq !== undefined && <span className="shrink-0 text-muted-foreground">· Event {search.seq}</span>}
    </div>
  )
}

const voiceIcons = new Map<string, () => ReactElement>()

/** Stimmfarbe als Symbol in der Palette („Weiter mit …“). */
function voiceIcon(voice: ReturnType<typeof voiceOf>) {
  let icon = voiceIcons.get(voice)
  if (!icon) {
    icon = () => <VoiceDot voice={voice} />
    voiceIcons.set(voice, icon)
  }
  return icon
}
