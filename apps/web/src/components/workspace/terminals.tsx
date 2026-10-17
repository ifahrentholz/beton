import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import type { ChannelAccess, Event } from '@beton/sdk'
import { CircleStop, Lock, LockOpen, Pencil, Plus, RotateCw, SquareTerminal, X } from 'lucide-react'
import { client } from '@/lib/client'
import { locale, useT } from '@/lib/i18n'
import { runningFor, sessionTerminals, toolSandbox, type TerminalInfo } from '@/lib/terminals'
import { terminalTheme, useTheme } from '@/lib/theme'
import { cn } from '@/lib/utils'
import { rich } from '../kbd'
import { Callout } from './ui'

/** Größe, mit der ein neues Terminal startet; danach passt Fit sie an den Container an. */
const START = { cols: 120, rows: 32 }

type Xterm = import('./xterm-impl').XTerm

/** Lädt xterm.js nach (eigener Chunk, WEB-010). */
export const loadXterm = (): Promise<typeof import('./xterm-impl')> => import('./xterm-impl')

/** Farben aus den Tokens am Container: Das Terminal bleibt dunkel (`.dark`, wie im Prototyp). */
const themeOf = (el: HTMLElement) => terminalTheme((name) => getComputedStyle(el).getPropertyValue(name))

/**
 * Theme des Terminals folgt dem UI (UX-005 AC1): Bei jedem Wechsel liest es die Tokens seines
 * Containers neu – den dunklen Satz der aktiven Familie (`theme-nord dark`, `theme-hc dark`) –,
 * ohne Neuladen. Liefert das Abmelden.
 */
function followTheme(x: Xterm, el: HTMLElement): () => void {
  return useTheme.subscribe((s, prev) => {
    if (s.family !== prev.family || s.mode !== prev.mode) x.term.options.theme = themeOf(el)
  })
}

/** Ausgeblendete beendete Terminals je Session (nur auf diesem Gerät). */
const hiddenKey = (sessionId: string) => `beton.terminals.hidden.${sessionId}`

function readHidden(sessionId: string): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(hiddenKey(sessionId)) ?? '[]')
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []
  } catch {
    return []
  }
}

function writeHidden(sessionId: string, ids: string[]) {
  try {
    localStorage.setItem(hiddenKey(sessionId), JSON.stringify(ids))
  } catch {
    // Ohne Speicher gilt das Ausblenden bis zum Neuladen.
  }
}

/**
 * Aktion im Tab (Umbenennen, Schließen, Ausblenden) für Maus und Screenreader. Nicht im
 * Tab-Fokus: Ein Tab darf keine fokussierbaren Elemente enthalten (WEB-016); per Tastatur gehen
 * dieselben Aktionen über Entf und F2 am Tab.
 */
function TabAction({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <span
      role="button"
      aria-label={label}
      className="cursor-pointer"
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
    >
      {children}
    </span>
  )
}

/** Tab-Leiste der Terminals wie im Prototyp (`workspace-terminals`). */
export function TermTabs({
  terminals,
  active,
  canDrive,
  onSelect,
  onNew,
  onClose,
  onHide,
  onRename,
}: {
  terminals: TerminalInfo[]
  active: string | undefined
  canDrive: boolean
  onSelect: (id: string) => void
  onNew: () => void
  onClose: (id: string) => void
  onHide: (id: string) => void
  onRename: (id: string, name: string) => void
}) {
  const t = useT()
  const [editing, setEditing] = useState<string | undefined>()
  const [draft, setDraft] = useState('')
  const startRename = (term: TerminalInfo) => {
    setDraft(term.name)
    setEditing(term.id)
  }
  return (
    <div className="flex h-9 shrink-0 items-end gap-0.5 overflow-x-auto border-b border-border bg-sidebar px-1">
      {/* Nur Tabs in der Tab-Liste; „+“ steht daneben (ohne Tabs keine leere Liste, WEB-016). */}
      {terminals.length > 0 && (
        <div role="tablist" aria-label={t('term.tabs')} className="flex items-end gap-0.5">
          {terminals.map((term) => (
            <span
              key={term.id}
              role="tab"
              tabIndex={0}
              aria-selected={term.id === active}
              title={term.byAgent ? 'vom Agent gestartet' : 'von dir'}
              onClick={() => onSelect(term.id)}
              aria-keyshortcuts={term.ended ? 'Delete' : canDrive ? 'Delete F2' : undefined}
              onKeyDown={(e) => {
                // Tabs nach APG: Enter wählt, Entf schließt bzw. blendet aus, F2 benennt um.
                if (e.key === 'Enter') onSelect(term.id)
                else if (e.key === 'Delete' && term.ended) onHide(term.id)
                else if (e.key === 'Delete' && canDrive) onClose(term.id)
                else if (e.key === 'F2' && canDrive && !term.ended) startRename(term)
              }}
              className={cn(
                '-mb-px flex h-8 shrink-0 cursor-pointer items-center gap-1.5 rounded-t-md border border-b-0 px-2.5 text-[12px]',
                term.id === active ? 'border-border bg-card font-medium' : 'border-transparent text-muted-foreground',
                term.ended && 'italic',
              )}
            >
              {term.ended && <CircleStop className="size-3 text-muted-foreground" role="img" aria-label={t('term.ended')} />}
              {!term.ended && term.byAgent && <span className="size-1.5 animate-pulse rounded-full bg-ok" aria-label="läuft" />}
              {editing === term.id ? (
                <input
                  autoFocus
                  aria-label="Name des Terminals"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onClick={(e) => e.stopPropagation()}
                  onBlur={() => setEditing(undefined)}
                  onKeyDown={(e) => {
                    e.stopPropagation()
                    if (e.key === 'Enter' && draft.trim()) {
                      onRename(term.id, draft.trim())
                      setEditing(undefined)
                    }
                    if (e.key === 'Escape') setEditing(undefined)
                  }}
                  className="w-28 rounded-sm border border-border bg-background px-1 text-[12px]"
                />
              ) : (
                term.name
              )}
              {canDrive && !term.ended && term.id === active && editing !== term.id && (
                <TabAction label={t('cmd.rename')} onClick={() => startRename(term)}>
                  <Pencil className="size-3 text-muted-foreground" />
                </TabAction>
              )}
              {term.ended ? (
                // Ausblenden ist eine Ansicht dieses Geräts, auch ohne Recht `drive`.
                <TabAction label={t('term.hide', { name: term.name })} onClick={() => onHide(term.id)}>
                  <X className="size-3 text-muted-foreground" />
                </TabAction>
              ) : (
                canDrive && (
                  <TabAction label={t('term.close', { name: term.name })} onClick={() => onClose(term.id)}>
                    <X className="size-3 text-muted-foreground" />
                  </TabAction>
                )
              )}
            </span>
          ))}
        </div>
      )}
      {canDrive && (
        <button onClick={onNew} aria-label={t('term.new')} className="mb-1 ml-1 flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent">
          <Plus className="size-3.5" />
        </button>
      )}
    </div>
  )
}

/**
 * Ein Terminal (WEB-010): xterm.js am Binärkanal des PTY im Runner. Ausgabe kommt mit Kredit
 * erst nach dem Rendern zurück (PROTO-007), Eingaben nur mit Recht `drive` (AC3), Größe folgt
 * dem Fenster (AC2).
 */
export function TerminalScreen({
  sessionId,
  terminal,
  onAccess,
  onReattached,
  onEnded,
  onSize,
  load = loadXterm,
}: {
  sessionId: string
  terminal: TerminalInfo
  onAccess: (a: ChannelAccess) => void
  onReattached: () => void
  onEnded: () => void
  onSize: (cols: number, rows: number) => void
  load?: typeof loadXterm
}) {
  const box = useRef<HTMLDivElement>(null)
  const wrap = useRef<HTMLDivElement>(null)
  useEffect(() => {
    let disposed = false
    let x: Xterm | undefined
    let unfollow: (() => void) | undefined
    let channel: ReturnType<ReturnType<typeof client.session>['terminals']['attach']> | undefined
    let observer: ResizeObserver | undefined
    let resizeTimer: ReturnType<typeof setTimeout> | undefined
    let opens = 0
    let readOnly = true
    const session = client.session(sessionId)
    void load().then((mod) => {
      if (disposed || !box.current || !wrap.current) return
      x = mod.createTerminal(box.current, { readOnly: false, theme: themeOf(wrap.current) })
      unfollow = followTheme(x, wrap.current)
      const term = x.term
      let sentSize = `${terminal.cols}x${terminal.rows}`
      const sendSize = (cols: number, rows: number) => {
        if (readOnly || `${cols}x${rows}` === sentSize) return
        if (resizeTimer) clearTimeout(resizeTimer)
        resizeTimer = setTimeout(() => {
          sentSize = `${cols}x${rows}`
          void session.terminals.resize(terminal.id, cols, rows).catch(() => {})
        }, 120)
      }
      channel = session.terminals.attach(terminal.id, {
        manualCredit: true,
        onOpen: (access) => {
          opens += 1
          readOnly = access !== 'drive'
          term.options.disableStdin = readOnly
          onAccess(access)
          // Die Größe kann sich vor dem Öffnen des Kanals schon eingestellt haben.
          if (term.cols && term.rows) sendSize(term.cols, term.rows)
          if (opens > 1) onReattached()
        },
        onData: (bytes, keyframe) => {
          if (keyframe) term.reset()
          term.write(bytes, () => channel?.consumed(bytes.length))
        },
        onClosed: () => onEnded(),
      })
      term.onData((d) => {
        if (!readOnly) channel?.send(d)
      })
      term.onResize(({ cols, rows }) => {
        onSize(cols, rows)
        sendSize(cols, rows)
      })
      x.fit()
      observer = new ResizeObserver(() => x?.fit())
      observer.observe(box.current)
    })
    return () => {
      disposed = true
      if (resizeTimer) clearTimeout(resizeTimer)
      observer?.disconnect()
      unfollow?.()
      channel?.close()
      x?.dispose()
    }
    // Der Kanal gilt für genau dieses Terminal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, terminal.id])
  return (
    <div ref={wrap} data-testid="terminal-screen" className="dark min-h-0 flex-1 bg-sunken p-2">
      <div ref={box} className="h-full w-full" />
    </div>
  )
}

/**
 * Letzter Bildschirm eines beendeten Terminals (DATA-011 AC2, Entwurf `workspace-terminals`,
 * Zustand `closed`): der Snapshot-Blob im Emulator, in seiner Größe, abgeblendet, ohne Cursor
 * und ohne Eingabe. Kein Kanal: Der Prozess läuft nicht mehr.
 */
export function SnapshotScreen({
  sessionId,
  snapshot,
  load = loadXterm,
}: {
  sessionId: string
  snapshot: NonNullable<TerminalInfo['snapshot']>
  load?: typeof loadXterm
}) {
  const box = useRef<HTMLDivElement>(null)
  const wrap = useRef<HTMLDivElement>(null)
  const [failed, setFailed] = useState<string | undefined>()
  useEffect(() => {
    let disposed = false
    let x: Xterm | undefined
    let unfollow: (() => void) | undefined
    setFailed(undefined)
    void Promise.all([load(), client.session(sessionId).blobData(snapshot.ref).then((b) => b.arrayBuffer())])
      .then(([mod, data]) => {
        if (disposed || !box.current || !wrap.current) return
        x = mod.createTerminal(box.current, { readOnly: true, theme: themeOf(wrap.current), cols: snapshot.cols, rows: snapshot.rows, webgl: false })
        unfollow = followTheme(x, wrap.current)
        x.term.write(new Uint8Array(data))
        // Der Snapshot bringt den Cursor mit; ein beendetes Terminal zeigt keinen.
        x.term.write('\x1b[?25l')
      })
      .catch((e: unknown) => !disposed && setFailed(e instanceof Error ? e.message : String(e)))
    return () => {
      disposed = true
      unfollow?.()
      x?.dispose()
    }
  }, [sessionId, snapshot.ref, snapshot.cols, snapshot.rows, load])
  return (
    <div ref={wrap} data-testid="terminal-screen" className="dark min-h-0 flex-1 overflow-auto bg-sunken p-2 opacity-70">
      {failed ? <div className="text-[12px] text-deny">{failed}</div> : <div ref={box} data-testid="terminal-snapshot" />}
    </div>
  )
}

/** Leere Rail (Entwurf `workspace-terminals`, Zustand `empty`). */
function EmptyTerms({ canDrive, onNew }: { canDrive: boolean; onNew: () => void }) {
  const t = useT()
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      <SquareTerminal aria-hidden className="size-6 text-muted-foreground" />
      <div className="text-[13px] font-medium">{t('term.empty')}</div>
      <p className="max-w-[46ch] text-[12px] text-muted-foreground">{t('term.emptyText')}</p>
      {canDrive && (
        <button
          type="button"
          onClick={onNew}
          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-card px-3 text-[12px] font-medium hover:bg-accent"
        >
          <Plus aria-hidden className="size-3.5" /> {t('term.new')}
        </button>
      )}
    </div>
  )
}

/** Fehler beim Öffnen: `sandbox_unavailable` mit Grund und Weg zur Diagnose (fail closed, SBX-017). */
type OpenError = { code?: string; detail: string }

function OpenFailed({ error }: { error: OpenError }) {
  const t = useT()
  if (error.code !== 'sandbox_unavailable') {
    return (
      <div role="alert" className="m-2">
        <Callout tone="deny">{error.detail}</Callout>
      </div>
    )
  }
  return (
    <div role="alert" className="p-2">
      <Callout tone="deny">
        <div className="font-medium">{t('term.failed')}</div>
        <div className="text-[12px]">
          {rich(t('term.failedSandbox'), {
            reason: (
              <>
                <span className="font-mono">sandbox_unavailable</span>
                {error.detail ? `: ${error.detail.replace(/\.$/, '')}` : ''}
              </>
            ),
            cmd: <code className="font-mono">beton doctor</code>,
          })}
        </div>
      </Callout>
    </div>
  )
}

/** „14:21“ in der Sprache der Oberfläche. */
const clock = (ts: string) => new Date(ts).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' })

/**
 * Terminals einer Session in der Workspace-Rail (SES-019, WEB-010): benannte Tabs, neue
 * Shell, Umbenennen, Schließen; vom Agent gestartete Langläufer erscheinen mit. Beendete
 * Terminals bleiben mit ihrem letzten Bildschirm, bis man sie ausblendet (DATA-011 AC2).
 */
export function TerminalsPanel({
  sessionId,
  events,
  runnerGone,
  load,
}: {
  sessionId: string
  events: readonly Event[] | undefined
  runnerGone?: boolean
  load?: typeof loadXterm
}) {
  const t = useT()
  const [hidden, setHidden] = useState(() => readHidden(sessionId))
  useEffect(() => setHidden(readHidden(sessionId)), [sessionId])
  const all = useMemo(() => sessionTerminals(events, runnerGone), [events, runnerGone])
  const terminals = useMemo(() => all.filter((term) => !term.ended || !hidden.includes(term.id)), [all, hidden])
  const sandboxed = useMemo(() => toolSandbox(events), [events])
  const [active, setActive] = useState<string | undefined>()
  const [access, setAccess] = useState<ChannelAccess | undefined>()
  const [reattached, setReattached] = useState(false)
  const [error, setError] = useState<OpenError | undefined>()
  const [size, setSize] = useState<{ cols: number; rows: number } | undefined>()
  const current = terminals.find((term) => term.id === active) ?? terminals.at(-1)
  // Lokal steuert der Owner; mit Rollen (COL-001, M4) entscheidet das Recht des Kanals.
  const canDrive = access !== 'read'
  const session = client.session(sessionId)

  const run = (p: Promise<unknown>) =>
    p.catch((e: unknown) => {
      const problem = (e as { problem?: { code?: string; detail?: string } }).problem
      setError({ ...(problem?.code ? { code: problem.code } : {}), detail: problem?.detail ?? (e instanceof Error ? e.message : String(e)) })
    })
  const openNew = () => {
    setError(undefined)
    void run(session.terminals.open(START).then((term) => setActive(term.terminal_id)))
  }
  const hide = (id: string) => {
    // Nur Ausgeblendete, die es im Log noch gibt, merken.
    const next = [...hidden.filter((h) => all.some((term) => term.id === h)), id]
    setHidden(next)
    writeHidden(sessionId, next)
    if (active === id) setActive(undefined)
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="terminals-panel">
      <TermTabs
        terminals={terminals}
        active={current?.id}
        canDrive={canDrive}
        onSelect={(id) => {
          setActive(id)
          setReattached(false)
          setSize(undefined)
        }}
        onNew={openNew}
        onClose={(id) => void run(session.terminals.close(id))}
        onHide={hide}
        onRename={(id, name) => void run(session.terminals.rename(id, name))}
      />
      {error && <OpenFailed error={error} />}
      {!current && <EmptyTerms canDrive={canDrive} onNew={openNew} />}
      {current?.ended && (
        <>
          <div className="flex h-8 shrink-0 items-center gap-3 border-b border-border px-3 text-[11px] text-muted-foreground">
            <span>
              {'withRunner' in current.ended
                ? t('term.endedWithRunner')
                : current.ended.exitCode === undefined
                  ? t('term.endedAtNoCode', { time: clock(current.ended.at) })
                  : t('term.endedAt', { time: clock(current.ended.at), code: current.ended.exitCode })}
              {' · '}
              {t('term.lastScreen')}
            </span>
          </div>
          {current.snapshot ? (
            <SnapshotScreen key={current.id} sessionId={sessionId} snapshot={current.snapshot} {...(load ? { load } : {})} />
          ) : (
            <div data-testid="terminal-screen" className="dark min-h-0 flex-1 bg-sunken p-3 text-[12px] text-muted-foreground opacity-70">
              {t('term.noSnapshot')}
            </div>
          )}
        </>
      )}
      {current && !current.ended && (
        <>
          <div className="flex h-8 shrink-0 items-center gap-3 border-b border-border px-3 text-[11px] text-muted-foreground">
            {current.byAgent ? (
              <>
                <span>Vom Agent gestartet · {runningFor(current.since)}</span>
                <span>Läuft weiter, auch wenn alle Fenster geschlossen sind</span>
              </>
            ) : sandboxed === false ? (
              // Ausdrücklich bestätigtes `backend: none` (SBX-006): nie als Sandbox ausgeben.
              <>
                <span className="inline-flex items-center gap-1 font-medium text-deny" data-testid="terminal-no-sandbox">
                  <LockOpen aria-hidden className="size-3" /> {t('term.noSandbox')}
                </span>
                <span>{t('term.noSandboxHint')}</span>
              </>
            ) : (
              <span className="inline-flex items-center gap-1">
                <Lock aria-hidden className="size-3" /> {t('term.sandbox')}
              </span>
            )}
            {!current.byAgent && (
              <span data-testid="terminal-size">
                {size?.cols ?? current.cols} × {size?.rows ?? current.rows}
              </span>
            )}
          </div>
          {reattached && (
            <div className="flex items-center gap-2 border-b border-border bg-ok-soft px-3 py-1.5 text-[12px]" role="status">
              <RotateCw className="size-3.5 text-ok" />
              Wieder verbunden. Das Terminal lief weiter, während du weg warst; der Verlauf ist vollständig.
            </div>
          )}
          <TerminalScreen
            key={current.id}
            sessionId={sessionId}
            terminal={current}
            onAccess={setAccess}
            onReattached={() => setReattached(true)}
            onEnded={() => setReattached(false)}
            onSize={(cols, rows) => setSize({ cols, rows })}
            {...(load ? { load } : {})}
          />
          {access === 'read' && (
            <div className="border-t border-border p-2">
              <Callout>
                Du siehst dieses Terminal nur lesend. Eingaben sind deaktiviert; dafür braucht es die Rolle{' '}
                <span className="font-medium">Mitsteuern</span>.
              </Callout>
            </div>
          )}
        </>
      )}
    </div>
  )
}
