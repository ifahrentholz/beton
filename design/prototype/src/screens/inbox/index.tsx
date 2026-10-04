import { useState, type ReactNode } from 'react'
import { AtSign, Bot, Check, ChevronRight, CircleAlert, CornerDownLeft, FolderGit2, Gauge, GitFork, Inbox as InboxIcon, Keyboard, MessageCircleQuestion, Minimize2, Moon, Plus, Search, Settings, ShieldCheck, Square, X } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, StatusMark, VoiceDot } from '@/app/harness'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { AgentMessage, ToolCall, UserMessage } from '@/app/stream'
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from '@/components/ui/command'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { harnesses, inboxDone, inboxFyi, inboxOpen, sessions, type HarnessId, type InboxItem, type InboxKind } from '@/mock/data'

/* ───────────────────────── Inbox (UX-001, USE-004) ───────────────────────── */

const KIND: Record<InboxKind, { label: string; icon: typeof InboxIcon }> = {
  approval: { label: 'Freigabe', icon: ShieldCheck },
  question: { label: 'Frage', icon: MessageCircleQuestion },
  async_done: { label: 'Agent fertig', icon: Bot },
  async_failed: { label: 'Agent fehlgeschlagen', icon: Bot },
  mention: { label: 'Erwähnung', icon: AtSign },
  notice: { label: 'Hinweis', icon: Gauge },
}

/** Befehle (Freigaben) in Monospace, sonst Fließtext. */
function titleOf(item: InboxItem): ReactNode {
  return item.mono ? <code className="font-mono text-[12.5px]">{item.title}</code> : item.title
}

function InboxRow({ item, answering, selected }: { item: InboxItem; answering?: boolean; selected?: boolean }) {
  const waiting = item.kind === 'approval' || item.kind === 'question'
  const K = KIND[item.kind]
  const content = (
    <div
      className={cn(
        'relative border-l-4 px-4 py-3',
        waiting ? 'chamfer border-signal bg-signal-soft' : 'border-transparent',
        selected && !waiting && 'border-foreground bg-accent/60',
      )}
      aria-current={selected ? 'true' : undefined}
    >
      <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
        <K.icon className="size-3.5" />
        <span className={cn(waiting && 'font-medium text-foreground')}>{K.label}</span>
        <span>·</span>
        <span className="truncate">{item.session}</span>
        {item.project !== '—' && <span className="font-mono text-[11px]">{item.project}</span>}
        {item.later && <span className="rounded-sm border border-border px-1 text-[10px]">{item.later}</span>}
        <span className="ml-auto shrink-0">{item.when}</span>
      </div>
      <div className={cn('mt-1 text-[14px]', (waiting || item.kind !== 'notice') && 'font-medium')}>{titleOf(item)}</div>
      {item.body && <p className="mt-1 text-[13px] text-muted-foreground">{item.body}</p>}
      {item.paused && (
        <p className="mt-1 flex items-center gap-1.5 text-[12px]">
          <span className="chamfer-sm size-2 bg-signal" aria-hidden /> Session im Hintergrund pausiert, niemand schaut zu – sie läuft weiter, sobald du
          antwortest.
        </p>
      )}

      {item.kind === 'approval' && (
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Erlauben</button>
          <button className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">Ablehnen …</button>
          <button className="rounded-md px-2 py-1.5 text-[13px] text-muted-foreground hover:text-foreground">Session öffnen</button>
          {answering && (
            <span className="flex w-full items-center gap-2 pt-1">
              <span className="flex h-8 flex-1 items-center rounded-md border border-input bg-card px-2 text-[13px]">Bitte erst die Tests auf CI abwarten</span>
              <button className="rounded-md border border-deny/60 px-3 py-1.5 text-[13px] text-deny">Mit Grund ablehnen</button>
            </span>
          )}
        </div>
      )}
      {item.kind === 'question' && (
        <div className="mt-2.5 flex flex-col gap-2">
          <div className="flex flex-wrap gap-2">
            <button className="rounded-md border border-foreground/30 bg-card px-3 py-1.5 text-[13px]">Regeln ersetzen</button>
            <button className="rounded-md border border-foreground/30 bg-card px-3 py-1.5 text-[13px]">Bei ESLint 9 bleiben</button>
          </div>
          <div className="flex items-center gap-2">
            <span className={cn('flex h-8 flex-1 items-center rounded-md border border-input bg-card px-2 text-[13px]', !answering && 'text-muted-foreground', answering && 'outline-2 outline-ring')}>
              {answering ? 'Ersetzen, aber no-unused-vars als Warnung lassen' : 'Eigene Antwort …'}
              {answering && <span className="ml-0.5 inline-block h-4 w-px animate-pulse bg-foreground" />}
            </span>
            <button className={cn('chamfer-sm px-3 py-1.5 text-[13px] font-semibold', answering ? 'bg-foreground text-background' : 'bg-muted text-muted-foreground')}>Antworten</button>
          </div>
        </div>
      )}
      {item.kind === 'notice' && (
        <div className="mt-2 flex gap-2">
          <button className="rounded-md border border-border px-3 py-1 text-[12px] hover:bg-accent">Mit Codex weiterarbeiten</button>
          <button className="rounded-md px-2 py-1 text-[12px] text-muted-foreground hover:text-foreground">Solche Hinweise abschalten</button>
        </div>
      )}
    </div>
  )
  const marked = selected ? (
    <div className="relative">
      <span aria-hidden className="absolute top-2 bottom-2 -left-3 w-1 rounded-full bg-foreground" title="Ausgewählt (j/k)" />
      {content}
    </div>
  ) : (
    content
  )
  return item.feature ? <F id={item.feature}>{marked}</F> : marked
}

function InboxList({ state }: { state: string }) {
  const filters = [
    { id: 'open', label: 'Offen', n: state === 'empty' ? 0 : inboxOpen.length },
    { id: 'all', label: 'Alles Neue', n: state === 'empty' ? 0 : inboxOpen.length + inboxFyi.length },
    { id: 'approval', label: 'Freigaben' },
    { id: 'question', label: 'Fragen' },
    { id: 'agents', label: 'Hintergrund-Agents' },
    { id: 'mention', label: 'Erwähnungen' },
    { id: 'done', label: 'Erledigt' },
  ]
  const active = state === 'done' ? 'done' : 'all'
  return (
    <AppLayout nav="inbox" sessionList={false} inboxCount={state === 'empty' ? 0 : undefined}>
      <F id="UX-001" className="flex min-h-0 flex-1 flex-col">
        <div className="flex items-center gap-3 border-b border-border px-6 py-3">
          <h1 className="type-wide text-xl font-[700]">Inbox</h1>
          <div className="ml-4 flex gap-0.5" role="tablist">
            {filters.map((f) => (
              <button
                key={f.id}
                role="tab"
                aria-selected={f.id === active}
                className={cn('rounded-md px-2.5 py-1 text-[13px]', f.id === active ? 'bg-accent font-medium' : 'text-muted-foreground hover:text-foreground')}
              >
                {f.label}
                {f.n !== undefined && <span className="ml-1 tabular-nums text-muted-foreground">{f.n}</span>}
              </button>
            ))}
          </div>
          <button className="ml-auto inline-flex h-7 items-center gap-1 rounded-md border border-border px-2 text-[12px] text-muted-foreground">
            Alle Projekte <ChevronRight className="size-3 rotate-90" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {state === 'empty' ? (
            <div className="concrete-grain flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
              <Check className="size-6 text-ok" />
              <p className="type-wide text-lg font-[700]">Nichts wartet auf dich</p>
              <p className="max-w-sm text-sm text-muted-foreground">Neue Freigaben und Fragen deiner Agents erscheinen hier sofort – auch aus Sessions, die gerade niemand geöffnet hat.</p>
            </div>
          ) : state === 'done' ? (
            <div className="mx-auto max-w-3xl divide-y divide-border px-6 py-4">
              {inboxDone.map((d) => {
                const K = KIND[d.kind]
                return (
                  <div key={d.id} className="flex items-start gap-3 py-3">
                    <K.icon className="mt-0.5 size-4 text-muted-foreground" />
                    <div className="min-w-0 flex-1">
                      <div className="text-[13px]">{titleOf(d)}</div>
                      <div className="text-[12px] text-muted-foreground">
                        {d.session} · {d.when}
                      </div>
                    </div>
                    <span className="max-w-[260px] text-right text-[12px] text-muted-foreground">{d.resolution}</span>
                  </div>
                )
              })}
            </div>
          ) : (
            <div className="mx-auto flex max-w-3xl flex-col gap-2 px-6 py-4">
              <div className="text-[12px] font-medium text-muted-foreground">Du bist dran · {inboxOpen.length}</div>
              {inboxOpen.map((it, i) => (
                <InboxRow key={it.id} item={it} answering={state === 'answering' && it.kind === 'question'} selected={i === 0 && state !== 'answering'} />
              ))}
              <div className="mt-4 text-[12px] font-medium text-muted-foreground">Zur Info · {inboxFyi.length}</div>
              <div className="divide-y divide-border rounded-md border border-border">
                {inboxFyi.map((it) => (
                  <InboxRow key={it.id} item={it} />
                ))}
              </div>
            </div>
          )}
        </div>
        <div className="flex items-center gap-4 border-t border-border px-6 py-1.5 text-[11px] text-muted-foreground">
          <span><kbd className="font-mono">j</kbd>/<kbd className="font-mono">k</kbd> wechseln</span>
          <span><kbd className="font-mono">a</kbd> erlauben</span>
          <span><kbd className="font-mono">d</kbd> ablehnen</span>
          <span><kbd className="font-mono">↵</kbd> Session öffnen</span>
          <span><kbd className="font-mono">e</kbd> erledigt</span>
          <span className="ml-auto">Zähler auch im Dock und in der Menüleiste</span>
        </div>
      </F>
    </AppLayout>
  )
}

/* ───────────────────────── Hintergrund: Session (für Overlays) ───────────────────────── */

function SessionBackdrop({ running, children }: { running?: boolean; children: ReactNode }) {
  return (
    <div className="relative h-full">
      <AppLayout activeSession="ses_7f3k">
        <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status={running ? 'running' : 'idle'} />
        <div className="min-h-0 flex-1 overflow-hidden">
          <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
            <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
            <ToolCall kind="search" name="Suche" target="rg 'auth.post' src/" duration="0,2 s" />
            <ToolCall kind="edit" name="Bearbeiten" target="src/routes/auth.ts" duration="0,3 s" />
            <AgentMessage harness="claude" streaming={running}>
              <p>Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt.</p>
            </AgentMessage>
          </div>
        </div>
        <Composer harness="claude" running={running} />
      </AppLayout>
      <div className="absolute inset-0 z-10 flex items-start justify-center bg-foreground/25 pt-20">{children}</div>
    </div>
  )
}

/* ───────────────────────── Command-Palette (UX-002) ───────────────────────── */

function Palette({ initial, running }: { initial: string; running: boolean }) {
  const [q, setQ] = useState(initial)
  return (
    <Command className="w-[600px] rounded-lg! border border-border shadow-2xl" loop>
      <CommandInput value={q} onValueChange={setQ} placeholder="Befehl, Session oder Seite suchen …" />
      <CommandList className="max-h-[420px]">
        <CommandEmpty>
          <div className="text-[13px]">Nichts gefunden für „{q}“.</div>
          <div className="mt-1 text-[12px] text-muted-foreground">Gesucht wird in Befehlen, Sessions, Projekten und Seiten.</div>
        </CommandEmpty>
        <CommandGroup heading="Diese Session">
          <CommandItem value="Session forken fork">
            <GitFork /> Session forken <CommandShortcut>⌘⇧F</CommandShortcut>
          </CommandItem>
          <CommandItem value="Weiter mit Codex fork harness wechseln">
            <VoiceDot voice="codex" /> Weiter mit Codex <span className="text-muted-foreground">(Fork)</span>
          </CommandItem>
          <CommandItem value="Weiter mit Gemini CLI fork harness wechseln">
            <VoiceDot voice="acp" /> Weiter mit Gemini CLI <span className="text-muted-foreground">(Fork)</span>
          </CommandItem>
          {running && (
            <CommandItem value="Unterbrechen interrupt stop">
              <Square /> Unterbrechen <CommandShortcut>Esc Esc</CommandShortcut>
            </CommandItem>
          )}
          <F id="USE-009" as="span" className="block">
            <CommandItem value="Kontext kompaktieren compact">
              <Minimize2 /> Kontext kompaktieren <span className="text-muted-foreground">/compact · 36 % belegt</span>
            </CommandItem>
          </F>
          <CommandItem value="Modell wechseln model">
            <Bot /> Modell wechseln <CommandShortcut>⌘⇧M</CommandShortcut>
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading="Allgemein">
          <CommandItem value="Neue Session new">
            <Plus /> Neue Session <CommandShortcut>⌘N</CommandShortcut>
          </CommandItem>
          <CommandItem value="Agent starten agent">
            <Bot /> Agent starten …
          </CommandItem>
          <CommandItem value="Theme wechseln dunkel hell">
            <Moon /> Theme wechseln
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading="Gehe zu">
          <CommandItem value="Inbox">
            <InboxIcon /> Inbox <span className="text-muted-foreground">2 offen</span> <CommandShortcut>⌘⇧I</CommandShortcut>
          </CommandItem>
          <CommandItem value="Verbrauch usage">
            <Gauge /> Verbrauch
          </CommandItem>
          <CommandItem value="Einstellungen settings">
            <Settings /> Einstellungen <CommandShortcut>⌘,</CommandShortcut>
          </CommandItem>
          <CommandItem value="Projekt shop-frontend">
            <FolderGit2 /> Projekt shop-frontend
          </CommandItem>
          <CommandItem value="Session Review: Rate-Limiter">
            <StatusMark status="running" /> Review: Rate-Limiter
          </CommandItem>
        </CommandGroup>
      </CommandList>
      <div className="flex gap-4 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        <span>↑↓ wählen</span>
        <span>↵ ausführen</span>
        <span>Esc schließen</span>
        <span className="ml-auto">⌘K überall, auch im Eingabefeld</span>
      </div>
    </Command>
  )
}

function InboxPalette({ state }: { state: string }) {
  const initial = state === 'fork' ? 'fork' : state === 'none' ? 'deploy prod' : ''
  return (
    <SessionBackdrop running={state === 'running'}>
      <F id="UX-002">
        <Palette key={state} initial={initial} running={state === 'running'} />
      </F>
    </SessionBackdrop>
  )
}

/* ───────────────────────── Session-Switcher (UX-003) ───────────────────────── */

function InboxSwitcher({ state }: { state: string }) {
  const mru = ['ses_7f3k', 'ses_6n1c', 'ses_7f3m', 'ses_6q2a', 'ses_6p9z', 'ses_6k8e', 'ses_6m4d']
  const list = mru.map((id) => sessions.find((s) => s.id === id)!).filter((s) => state !== 'search' || /rate|review/i.test(s.title))
  const paused = new Set(['ses_6n1c'])
  return (
    <SessionBackdrop>
      <F id="UX-003" className="w-[560px] overflow-hidden rounded-lg border border-border bg-popover shadow-2xl">
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search className="size-4 text-muted-foreground" />
          <span className={cn('flex-1 text-[14px]', state !== 'search' && 'text-muted-foreground')}>{state === 'search' ? 'rate' : 'Zu Session wechseln …'}</span>
          <span className="text-[11px] text-muted-foreground">zuletzt genutzt</span>
        </div>
        <div className="py-1">
          {list.map((s, i) => {
            const waiting = s.status === 'waiting' || paused.has(s.id)
            return (
              <div key={s.id} className={cn('flex items-center gap-3 border-l-2 px-3 py-2', i === 1 || (state === 'search' && i === 0) ? 'border-signal bg-accent' : 'border-transparent')}>
                <StatusMark status={paused.has(s.id) ? 'waiting' : s.status} />
                <div className="min-w-0 flex-1">
                  <div className={cn('truncate text-[13px]', s.unread && 'font-semibold')}>{s.title}</div>
                  <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                    <HarnessBadge id={s.harness} className="text-[11px]" />
                    <span>· {s.project === 'shop' ? 'shop-frontend' : s.project}</span>
                    {s.async && <span>· im Hintergrund</span>}
                  </div>
                </div>
                {waiting && (
                  <span className="chamfer-sm bg-signal px-1.5 py-0.5 text-[11px] font-medium text-signal-foreground">{paused.has(s.id) ? 'Frage offen' : 'Freigabe offen'}</span>
                )}
                {!waiting && s.unread && <span className="text-[11px] text-muted-foreground">ungelesen</span>}
                <span className="w-16 text-right text-[11px] text-muted-foreground">{s.updated}</span>
              </div>
            )
          })}
        </div>
        <div className="flex gap-4 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
          <span>⌘⌥S öffnen</span>
          <span>↑↓ wählen</span>
          <span>↵ wechseln</span>
          <span>Ctrl+Tab durchschalten</span>
          <span className="ml-auto">Wechsel sofort aus dem lokalen Speicher</span>
        </div>
      </F>
    </SessionBackdrop>
  )
}

/* ───────────────────────── Tastenkürzel (UX-004) ───────────────────────── */

const SHORTCUTS: { group: string; items: [string, string][] }[] = [
  { group: 'Allgemein', items: [['Befehlspalette', '⌘K'], ['Neue Session', '⌘N'], ['Zu Session wechseln', '⌘⌥S'], ['Inbox', '⌘⇧I'], ['Einstellungen', '⌘,'], ['Tastenkürzel anzeigen', '⌘/']] },
  { group: 'Session', items: [['Senden', '↵'], ['Unterbrechen', 'Esc Esc'], ['Modell wechseln', '⌘⇧M'], ['Session forken', '⌘⇧F'], ['Umbenennen', 'F2']] },
  { group: 'Freigaben', items: [['Fokussierte Freigabe erlauben', '⌘↵'], ['Fokussierte Freigabe ablehnen', '⌘⌫']] },
  { group: 'Inbox', items: [['Nächster / vorheriger Eintrag', 'j / k'], ['Erlauben', 'a'], ['Ablehnen', 'd'], ['Session öffnen', '↵']] },
]

function Keys({ k }: { k: string }) {
  return (
    <span className="inline-flex gap-0.5">
      {k.split(' ').map((p, i) =>
        p === '/' ? (
          <span key={i} className="px-0.5 text-muted-foreground">/</span>
        ) : (
          <kbd key={i} className="min-w-5 rounded-sm border border-border bg-card px-1 text-center font-mono text-[11px]">
            {p}
          </kbd>
        ),
      )}
    </span>
  )
}

function InboxShortcuts({ state }: { state: string }) {
  return (
    <SessionBackdrop>
      <F id="UX-004" className="w-[720px] overflow-hidden rounded-lg border border-border bg-popover shadow-2xl">
        <div className="flex items-center gap-3 border-b border-border px-4 py-2.5">
          <Keyboard className="size-4" />
          <h2 className="text-[15px] font-semibold">Tastenkürzel</h2>
          <span className="ml-3 flex h-7 flex-1 items-center gap-2 rounded-md border border-input bg-card px-2 text-[12px] text-muted-foreground">
            <Search className="size-3.5" /> Kürzel suchen
          </span>
          <button aria-label="Schließen" className="text-muted-foreground hover:text-foreground">
            <X className="size-4" />
          </button>
        </div>
        <div className="grid grid-cols-2 gap-x-8 px-4 py-3">
          {SHORTCUTS.map((g) => (
            <div key={g.group} className="mb-3">
              <div className="mb-1 text-[12px] font-medium text-muted-foreground">{g.group}</div>
              {g.items.map(([label, k]) => {
                const editing = state === 'conflict' && label === 'Modell wechseln'
                return (
                  <div key={label} className={cn('group flex items-center gap-2 border-b border-border/60 py-1 text-[13px]', editing && 'bg-accent')}>
                    <span className="flex-1">{label}</span>
                    {editing ? (
                      <span className="rounded-sm border-2 border-ring px-1 font-mono text-[11px]">⌘K</span>
                    ) : (
                      <>
                        <button className="hidden text-[11px] text-muted-foreground group-hover:inline hover:text-foreground">Ändern</button>
                        <Keys k={k} />
                      </>
                    )}
                  </div>
                )
              })}
            </div>
          ))}
        </div>
        {state === 'conflict' && (
          <div className="mx-4 mb-3 flex items-center gap-3 rounded-md border border-border bg-card p-2.5 text-[13px]">
            <CircleAlert className="size-4 shrink-0" />
            <span className="flex-1">
              <Keys k="⌘K" /> gehört schon zu „Befehlspalette“. Tauschen? Dann bekommt die Befehlspalette <Keys k="⌘⇧M" />.
            </span>
            <button className="rounded-md bg-foreground px-3 py-1 text-[12px] font-semibold text-background">Tauschen</button>
            <button className="rounded-md border border-border px-3 py-1 text-[12px]">Abbrechen</button>
          </div>
        )}
        <div className="border-t border-border px-4 py-1.5 text-[11px] text-muted-foreground">
          Unter Windows und Linux: Ctrl statt ⌘. Kürzel wirken nicht, solange ein Eingabefeld Fokus hat – außer ⌘K.
        </div>
      </F>
    </SessionBackdrop>
  )
}

/* ───────────────────────── Einrichtung (UX-008, OBS-007, OBS-005) ───────────────────────── */

const STEPS = [
  { id: 'mode', label: 'Betriebsart' },
  { id: 'harnesses', label: 'Agent-CLIs' },
  { id: 'login', label: 'Anmeldung' },
  { id: 'keys', label: 'API-Keys', optional: true },
  { id: 'sandbox', label: 'Sandbox prüfen' },
  { id: 'privacy', label: 'Datenschutz' },
  { id: 'voice', label: 'Spracheingabe', optional: true },
  { id: 'first', label: 'Erste Session' },
]

function Tick({ s }: { s: 'ok' | 'warn' | 'fail' | 'missing' }) {
  if (s === 'ok') return <span className="inline-flex items-center gap-1 text-[12px] text-ok"><Check className="size-3.5" />ok</span>
  if (s === 'warn') return <span className="inline-flex items-center gap-1 text-[12px]"><span className="size-2 rotate-45 border border-foreground" />Hinweis</span>
  if (s === 'fail') return <span className="inline-flex items-center gap-1 text-[12px] text-deny"><span className="size-2 rotate-45 bg-deny" />Fehler</span>
  return <span className="inline-flex items-center gap-1 text-[12px] text-muted-foreground"><span className="size-2 rounded-full border border-muted-foreground" />nicht gefunden</span>
}

function Choice({ title, children, selected, recommended }: { title: string; children: ReactNode; selected?: boolean; recommended?: boolean }) {
  return (
    <button role="radio" aria-checked={!!selected} className={cn('w-full rounded-md border p-3 text-left', selected ? 'border-foreground ring-1 ring-foreground' : 'border-border hover:bg-accent/60')}>
      <div className="flex items-center gap-2">
        <span className={cn('size-3.5 rounded-full border border-foreground', selected && 'border-[5px]')} />
        <span className="text-[14px] font-semibold">{title}</span>
        {recommended && <span className="text-[11px] text-muted-foreground">empfohlen</span>}
      </div>
      <div className="mt-1 pl-5.5 text-[13px] text-muted-foreground">{children}</div>
    </button>
  )
}

function InboxOnboarding({ state }: { state: string }) {
  const idx = STEPS.findIndex((s) => s.id === state)
  const step = STEPS[idx] ?? STEPS[0]
  let body: ReactNode = null
  let primary = 'Weiter'
  let secondary: string | null = 'Überspringen'

  if (step.id === 'mode') {
    secondary = null
    body = (
      <>
        <h2 className="type-wide text-2xl font-[700]">Willkommen bei beton</h2>
        <p className="mt-2 max-w-xl text-[14px]">
          beton steuert deine Coding-Agents – Claude Code, Codex und andere – an einem Ort. Alles läuft auf diesem Rechner.
        </p>
        <div className="mt-5 grid max-w-xl grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-[13px]">
          <span className="mt-1.5 size-2 rounded-full bg-ok" aria-hidden />
          <span>Sessions, Verlauf und Einstellungen liegen in <code className="font-mono">~/.beton</code>. Es gibt keinen beton-Server und kein Konto.</span>
          <span className="mt-1.5 size-2 rounded-full bg-ok" aria-hidden />
          <span>Du nutzt deine bestehenden Abos über die offiziellen CLIs. Einen API-Key brauchst du nicht.</span>
          <span className="mt-1.5 size-2 rotate-45 border border-foreground" aria-hidden />
          <span>Ins Netz gehen nur die Anfragen der CLIs an ihre Modell-Anbieter. Alles andere fragt dich vorher.</span>
        </div>
        <div className="mt-6 flex max-w-xl flex-col gap-2" role="radiogroup" aria-label="Betriebsart">
          <Choice title="Lokal auf diesem Rechner" selected recommended>
            Für dich allein. Später kannst du dich trotzdem mit einem Team-Server verbinden.
          </Choice>
          <Choice title="Mit einem Team-Server verbinden">Wenn dein Team einen eigenen beton-Server betreibt. Du brauchst dessen Adresse.</Choice>
        </div>
      </>
    )
  }
  if (step.id === 'harnesses') {
    body = (
      <>
        <h2 className="type-wide text-2xl font-[700]">Gefundene Agent-CLIs</h2>
        <p className="mt-2 max-w-xl text-[14px] text-muted-foreground">beton nutzt die CLIs, die du schon installiert hast. Ohne deine Bestätigung installiert es nichts.</p>
        <div className="mt-5 max-w-2xl divide-y divide-border rounded-md border border-border">
          {[
            { h: 'claude' as HarnessId, cmd: 'claude 2.4.1', path: '/opt/homebrew/bin/claude', s: 'ok' as const },
            { h: 'codex' as HarnessId, cmd: 'codex 0.61.0', path: '~/.local/bin/codex', s: 'ok' as const },
            { h: 'gemini' as HarnessId, cmd: 'gemini 0.19.2 · ACP', path: '/opt/homebrew/bin/gemini', s: 'ok' as const },
          ].map((x) => (
            <div key={x.h} className="flex items-center gap-3 px-3 py-2.5">
              <HarnessBadge id={x.h} className="w-36" />
              <span className="font-mono text-[12px]">{x.cmd}</span>
              <span className="font-mono text-[11px] text-muted-foreground">{x.path}</span>
              <span className="ml-auto">
                <Tick s={x.s} />
              </span>
            </div>
          ))}
          <div className="flex items-center gap-3 px-3 py-2.5">
            <span className="w-36 text-[12px] font-medium">OpenCode</span>
            <span className="text-[12px] text-muted-foreground">nicht installiert</span>
            <span className="ml-auto flex items-center gap-3">
              <Tick s="missing" />
              <button className="rounded-md border border-border px-2 py-1 text-[12px] hover:bg-accent">Installieren …</button>
            </span>
          </div>
        </div>
        <p className="mt-3 text-[12px] text-muted-foreground">
          „Installieren …“ zeigt dir zuerst den genauen Befehl (<code className="font-mono">npm i -g opencode-ai</code>) und startet erst nach deiner
          Bestätigung.
        </p>
      </>
    )
  }
  if (step.id === 'login') {
    body = (
      <>
        <h2 className="type-wide text-2xl font-[700]">Bei den CLIs anmelden</h2>
        <p className="mt-2 max-w-xl text-[14px] text-muted-foreground">
          Die Anmeldung macht jede CLI selbst, mit deinem Abo. beton sieht dein Passwort nie und hat keinen eigenen Login-Dialog.
        </p>
        <div className="mt-5 max-w-2xl divide-y divide-border rounded-md border border-border">
          <div className="flex items-center gap-3 px-3 py-2.5">
            <HarnessBadge id="claude" className="w-36" />
            <span className="text-[13px]">Claude Max · angemeldet</span>
            <span className="ml-auto"><Tick s="ok" /></span>
          </div>
          <div className="px-3 py-2.5">
            <div className="flex items-center gap-3">
              <HarnessBadge id="codex" className="w-36" />
              <span className="text-[13px]">nicht angemeldet</span>
              <span className="ml-auto text-[12px] text-muted-foreground">Anmeldung läuft …</span>
            </div>
            <pre className="mt-2 rounded-md bg-sunken p-2.5 font-mono text-[12px] leading-relaxed">
              {`$ codex login
Opening https://auth.openai.com/… in your browser.
Waiting for sign-in to complete …`}
              <span className="ml-0.5 inline-block h-3.5 w-1.5 animate-pulse bg-foreground align-middle" />
            </pre>
            <p className="mt-1 text-[11px] text-muted-foreground">Terminal der Codex-CLI. Wird nicht aufgezeichnet und landet in keinem Diagnose-Bundle.</p>
          </div>
          <div className="flex items-center gap-3 px-3 py-2.5">
            <HarnessBadge id="gemini" className="w-36" />
            <span className="text-[13px]">Google-Konto · angemeldet</span>
            <span className="ml-auto"><Tick s="ok" /></span>
          </div>
        </div>
      </>
    )
  }
  if (step.id === 'keys') {
    primary = 'Weiter ohne API-Key'
    secondary = null
    body = (
      <>
        <h2 className="type-wide text-2xl font-[700]">API-Keys und Gateways</h2>
        <p className="mt-2 max-w-xl text-[14px]">
          <strong>Brauchst du nicht</strong>, wenn du über deine Abos arbeitest. Nur für Abrechnung per API, ein Firmen-Gateway oder lokale Modelle über eine
          eigene URL.
        </p>
        <div className="mt-5 max-w-xl space-y-2">
          {['Anthropic API-Key', 'OpenAI API-Key', 'Gateway (z. B. LiteLLM, OpenRouter)'].map((l) => (
            <div key={l} className="grid grid-cols-[200px_1fr] items-center gap-3">
              <span className="text-[13px] text-muted-foreground">{l}</span>
              <span className="flex h-8 items-center rounded-md border border-input bg-card px-2 text-[13px] text-muted-foreground">nicht gesetzt</span>
            </div>
          ))}
          <p className="pt-1 text-[12px] text-muted-foreground">Werte landen im Schlüsselbund des Systems, nie in einer Datei.</p>
        </div>
      </>
    )
  }
  if (step.id === 'sandbox') {
    body = (
      <F id="OBS-005">
        <h2 className="type-wide text-2xl font-[700]">Sandbox und Umgebung</h2>
        <p className="mt-2 max-w-xl text-[14px] text-muted-foreground">Agents führen Befehle in einer Sandbox aus. Ein Ausschnitt aus „Umgebung prüfen“:</p>
        <div className="mt-5 max-w-2xl divide-y divide-border rounded-md border border-border text-[13px]">
          {[
            ['Sandbox (macOS sandbox-exec)', 'ok', 'Datei- und Netzwerkregeln werden durchgesetzt'],
            ['Egress-Proxy-Zertifikat', 'ok', 'Lokal erzeugt, nur für Agent-Prozesse'],
            ['Schlüsselbund', 'ok', 'Zugriff erlaubt'],
            ['Rechte von ~/.beton', 'ok', '0700'],
            ['Docker oder Podman', 'warn', 'Nicht gefunden – nur für Container-Runner nötig'],
          ].map(([l, s, h]) => (
            <div key={l} className="grid grid-cols-[240px_80px_1fr] items-center gap-3 px-3 py-2">
              <span>{l}</span>
              <Tick s={s as 'ok' | 'warn'} />
              <span className="text-[12px] text-muted-foreground">{h}</span>
            </div>
          ))}
        </div>
      </F>
    )
  }
  if (step.id === 'privacy') {
    secondary = 'Überspringen (bleibt aus)'
    body = (
      <F id="OBS-007">
        <h2 className="type-wide text-2xl font-[700]">Darf beton etwas senden?</h2>
        <p className="mt-2 max-w-xl text-[14px] text-muted-foreground">Beides ist freiwillig und standardmäßig aus. Nichts ist vorausgewählt.</p>
        <div className="mt-5 max-w-2xl space-y-4">
          <div>
            <div className="text-[13px] font-semibold">Anonyme Nutzungsstatistik</div>
            <p className="text-[12px] text-muted-foreground">
              Einmal am Tag: Version, Betriebssystem, Zahl der Sessions je Harness-Art, genutzte Funktionen ja/nein. Nie Prompts, Code, Pfade oder
              Namen. <a className="underline underline-offset-2">Vollständige Liste</a>
            </p>
            <div className="mt-2 flex gap-2" role="radiogroup" aria-label="Nutzungsstatistik">
              <button role="radio" aria-checked={false} className="rounded-md border border-border px-3 py-1.5 text-[13px] hover:bg-accent">Nicht senden</button>
              <button role="radio" aria-checked={false} className="rounded-md border border-border px-3 py-1.5 text-[13px] hover:bg-accent">Anonym senden</button>
            </div>
          </div>
          <F id="OBS-008">
            <div className="text-[13px] font-semibold">Crash-Reports</div>
            <p className="text-[12px] text-muted-foreground">
              Berichte werden immer nur lokal gespeichert. Hier entscheidest du, ob beton sie automatisch sendet – sonst fragt es bei jedem Bericht.
            </p>
            <div className="mt-2 flex gap-2" role="radiogroup" aria-label="Crash-Reports">
              <button role="radio" aria-checked={false} className="rounded-md border border-border px-3 py-1.5 text-[13px] hover:bg-accent">Jedes Mal fragen</button>
              <button role="radio" aria-checked={false} className="rounded-md border border-border px-3 py-1.5 text-[13px] hover:bg-accent">Automatisch senden</button>
            </div>
          </F>
        </div>
      </F>
    )
  }
  if (step.id === 'first') {
    primary = 'Erste Session starten'
    secondary = null
    body = (
      <>
        <h2 className="type-wide text-2xl font-[700]">Bereit</h2>
        <p className="mt-2 max-w-xl text-[14px] text-muted-foreground">Wähle ein Projekt. beton legt für jede Session einen eigenen Git-Worktree an, dein Arbeitsstand bleibt unberührt.</p>
        <div className="mt-5 max-w-xl space-y-2">
          <div className="flex items-center gap-3 rounded-md border border-foreground p-3 ring-1 ring-foreground">
            <FolderGit2 className="size-4" />
            <div className="flex-1">
              <div className="text-[13px] font-medium">shop-frontend</div>
              <div className="font-mono text-[11px] text-muted-foreground">~/code/shop-frontend · Git, Branch main</div>
            </div>
            <span className="text-[12px] text-muted-foreground">zuletzt geöffnet</span>
          </div>
          <button className="w-full rounded-md border border-dashed border-input p-2.5 text-left text-[13px] text-muted-foreground">Anderen Ordner wählen …</button>
          <div className="flex items-center gap-3 pt-2 text-[13px]">
            <span className="text-muted-foreground">Mit</span>
            <HarnessBadge id="claude" model="claude-opus-5-5" />
            <span className="text-muted-foreground">über dein Claude-Max-Abo</span>
          </div>
        </div>
      </>
    )
  }

  return (
    <F id="UX-008" className="flex h-full bg-background">
      <div className="concrete-grain flex w-60 shrink-0 flex-col border-r border-border bg-sidebar p-5">
        <div className="type-wide text-lg font-[750]">beton</div>
        <div className="text-[12px] text-muted-foreground">Einrichtung</div>
        <ol className="mt-6 space-y-1">
          {STEPS.map((s, i) => (
            <li key={s.id} className={cn('flex items-center gap-2 text-[13px]', i === idx ? 'font-semibold' : i < idx ? 'text-foreground' : 'text-muted-foreground')}>
              <span className={cn('flex size-5 items-center justify-center rounded-full border text-[11px]', i < idx && 'border-foreground bg-foreground text-background', i === idx && 'border-foreground', i > idx && 'border-border')}>
                {i < idx ? <Check className="size-3" /> : i + 1}
              </span>
              {s.label}
              {s.optional && <span className="text-[11px] font-normal text-muted-foreground">optional</span>}
            </li>
          ))}
        </ol>
        <p className="mt-auto text-[11px] text-muted-foreground">Jeder Schritt ist überspringbar. Später erneut unter Einstellungen. Dasselbe im Terminal: beton setup.</p>
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto px-10 py-10">{body}</div>
        <div className="flex items-center gap-2 border-t border-border px-10 py-3">
          {idx > 0 && <button className="rounded-md px-3 py-1.5 text-[13px] text-muted-foreground hover:text-foreground">Zurück</button>}
          <span className="ml-auto text-[12px] text-muted-foreground">
            Schritt {idx + 1} von {STEPS.length}
          </span>
          {secondary && <button className="rounded-md border border-border px-3 py-1.5 text-[13px] hover:bg-accent">{secondary}</button>}
          <button className="inline-flex items-center gap-1.5 rounded-md bg-foreground px-4 py-1.5 text-[13px] font-semibold text-background">
            {primary} <CornerDownLeft className="size-3.5 opacity-60" />
          </button>
        </div>
      </div>
    </F>
  )
}

/* ───────────────────────── Automatische Session-Titel (UX-009) ───────────────────────── */

function InboxTitles({ state }: { state: string }) {
  const title =
    state === 'placeholder' ? 'Neue Session' : state === 'user' || state === 'renaming' ? 'Login-Schutz gegen Brute-Force' : 'Rate-Limiter für die Login-API'
  const listTitle = state === 'renaming' ? 'Rate-Limiter für die Login-API' : title
  const others = sessions.filter((s) => s.project === 'shop' && s.id !== 'ses_7f3k').slice(0, 3)
  return (
    <AppLayout sessionList={false}>
      <div className="flex min-h-0 flex-1">
        <div className="flex w-64 shrink-0 flex-col border-r border-border bg-sidebar pt-2">
          <div className="flex items-baseline justify-between px-3 pb-1">
            <span className="text-xs font-semibold">shop-frontend</span>
            <span className="font-mono text-[10px] text-muted-foreground">~/code/shop-frontend</span>
          </div>
          <F id="UX-009" className="flex items-center gap-2 border-l-2 border-signal bg-accent px-3 py-1.5" badge="top-right">
            <StatusMark status={state === 'placeholder' ? 'running' : 'idle'} />
            <div className="min-w-0 flex-1">
              <div className={cn('truncate text-[13px]', state === 'placeholder' && 'text-muted-foreground italic', state === 'generated' && 'animate-in fade-in')}>{listTitle}</div>
              <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                <VoiceDot voice="claude" className="size-1.5" /> Claude Code <span className="ml-auto">jetzt</span>
              </div>
            </div>
          </F>
          {others.map((s) => (
            <div key={s.id} className="flex items-center gap-2 border-l-2 border-transparent px-3 py-1.5">
              <StatusMark status={s.status} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px]">{s.title}</div>
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <VoiceDot voice={harnesses[s.harness].voice} className="size-1.5" /> {harnesses[s.harness].name}
                  <span className="ml-auto">{s.updated}</span>
                </div>
              </div>
            </div>
          ))}
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
            <StatusMark status={state === 'placeholder' ? 'running' : 'idle'} />
            {state === 'renaming' ? (
              <span className="flex h-8 w-[360px] items-center rounded-md border-2 border-ring bg-card px-2 text-[15px] font-semibold">
                {title}
                <span className="ml-0.5 inline-block h-4 w-px animate-pulse bg-foreground" />
              </span>
            ) : (
              <h2 className={cn('text-[15px] font-semibold', state === 'placeholder' && 'font-normal text-muted-foreground italic')}>{title}</h2>
            )}
            {state === 'generated' && <span className="text-[11px] text-muted-foreground" title="Aus der ersten Nachricht erzeugt. Doppelklick oder F2 zum Umbenennen.">automatisch</span>}
            {state === 'renaming' && <span className="text-[11px] text-muted-foreground">↵ übernehmen · Esc abbrechen · danach kein automatischer Titel mehr</span>}
            {state === 'user' && <span className="text-[11px] text-muted-foreground">von dir benannt</span>}
            <HarnessBadge id="claude" model="claude-opus-5-5" className="ml-auto" />
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
              <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
              <ToolCall kind="search" name="Suche" target="rg 'auth.post' src/" duration="0,2 s" status={state === 'placeholder' ? 'running' : 'ok'} />
            </div>
          </div>
          {state === 'user' && (
            <div className="mx-4 mb-2 flex items-center gap-3 rounded-md border border-border bg-card px-3 py-2 text-[12px]">
              <Switch defaultChecked size="sm" aria-label="Titel automatisch erzeugen" />
              <span>Titel für neue Sessions automatisch erzeugen</span>
              <span className="text-muted-foreground">– aus der ersten Nachricht, mit einem kleinen Modell deines Harness</span>
            </div>
          )}
          <Composer harness="claude" running={state === 'placeholder'} />
        </div>
      </div>
    </AppLayout>
  )
}

export const group: ScreenGroup = {
  id: 'inbox',
  title: 'Inbox & Navigation',
  order: 80,
  screens: [
    {
      id: 'inbox-list',
      title: 'Inbox',
      description:
        'Alles, was auf dich wartet: Freigaben und Fragen (gelb, mit Fase) direkt beantwortbar – auch für pausierte Hintergrund-Sessions –, darunter fertige Agents, Erwähnungen und Hinweise.',
      features: ['UX-001', 'USE-004'],
      states: [
        { id: 'open', title: 'Offene Einträge' },
        { id: 'answering', title: 'Frage beantworten' },
        { id: 'done', title: 'Erledigt' },
        { id: 'empty', title: 'Leer' },
      ],
      component: InboxList,
    },
    {
      id: 'inbox-palette',
      title: 'Befehlspalette (⌘K)',
      description: 'Befehle, Navigation und Aktionen der aktuellen Session, fuzzy durchsuchbar und vollständig per Tastatur. Tippen funktioniert.',
      features: ['UX-002', 'USE-009'],
      states: [
        { id: 'default', title: 'Geöffnet' },
        { id: 'fork', title: 'Suche „fork“' },
        { id: 'running', title: 'Agent läuft (mit Unterbrechen)' },
        { id: 'none', title: 'Keine Treffer' },
      ],
      component: InboxPalette,
    },
    {
      id: 'inbox-switcher',
      title: 'Session-Switcher',
      description: 'Zuletzt genutzte Sessions mit Status; offene Freigaben und Fragen sind markiert. Wechsel lädt sofort aus dem lokalen Speicher.',
      features: ['UX-003'],
      states: [
        { id: 'default', title: 'Zuletzt genutzt' },
        { id: 'search', title: 'Suche' },
      ],
      component: InboxSwitcher,
    },
    {
      id: 'inbox-shortcuts',
      title: 'Tastenkürzel',
      description: 'Übersicht aller Kürzel (⌘/), gruppiert und durchsuchbar; Umbelegen mit Konfliktprüfung.',
      features: ['UX-004'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'conflict', title: 'Konflikt beim Umbelegen' },
      ],
      component: InboxShortcuts,
    },
    {
      id: 'inbox-onboarding',
      title: 'Einrichtung',
      description:
        'Erster Start: Betriebsart mit „alles lokal“-Erklärung, gefundene CLIs, Anmeldung durch die CLIs selbst, optionale API-Keys, Sandbox-Check, Datenschutz ohne Vorauswahl, erste Session.',
      features: ['UX-008', 'OBS-005', 'OBS-007', 'OBS-008'],
      frame: 'desktop',
      states: STEPS.filter((s) => s.id !== 'voice').map((s) => ({ id: s.id, title: `${STEPS.indexOf(s) + 1}. ${s.label}` })),
      component: InboxOnboarding,
    },
    {
      id: 'inbox-titles',
      title: 'Automatische Session-Titel',
      description: 'Neue Sessions heißen erst „Neue Session“; der erzeugte Titel erscheint ohne Layout-Sprung. Umbenennen per Doppelklick oder F2 beendet die Automatik.',
      features: ['UX-009'],
      states: [
        { id: 'placeholder', title: 'Platzhalter' },
        { id: 'generated', title: 'Titel erzeugt' },
        { id: 'renaming', title: 'Umbenennen' },
        { id: 'user', title: 'Eigener Titel' },
      ],
      component: InboxTitles,
    },
  ],
}
