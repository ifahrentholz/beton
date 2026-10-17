import { useEffect, useMemo, useRef, useState } from 'react'
import type { ChannelAccess, Event } from '@beton/sdk'
import { Lock, Pencil, Plus, RotateCw, X } from 'lucide-react'
import { client } from '@/lib/client'
import { openTerminals, runningFor, toolSandbox, type TerminalInfo } from '@/lib/terminals'
import { cn } from '@/lib/utils'
import { Callout } from './ui'

/** Größe, mit der ein neues Terminal startet; danach passt Fit sie an den Container an. */
const START = { cols: 120, rows: 32 }

type Xterm = import('./xterm-impl').XTerm

/** Lädt xterm.js nach (eigener Chunk, WEB-010). */
export const loadXterm = (): Promise<typeof import('./xterm-impl')> => import('./xterm-impl')

/** Tab-Leiste der Terminals wie im Prototyp (`workspace-terminals`). */
export function TermTabs({
  terminals,
  active,
  canDrive,
  onSelect,
  onNew,
  onClose,
  onRename,
}: {
  terminals: TerminalInfo[]
  active: string | undefined
  canDrive: boolean
  onSelect: (id: string) => void
  onNew: () => void
  onClose: (id: string) => void
  onRename: (id: string, name: string) => void
}) {
  const [editing, setEditing] = useState<string | undefined>()
  const [draft, setDraft] = useState('')
  return (
    <div role="tablist" aria-label="Terminals" className="flex h-9 shrink-0 items-end gap-0.5 overflow-x-auto border-b border-border bg-sidebar px-1">
      {terminals.map((t) => (
        <span
          key={t.id}
          role="tab"
          tabIndex={0}
          aria-selected={t.id === active}
          title={t.byAgent ? 'vom Agent gestartet' : 'von dir'}
          onClick={() => onSelect(t.id)}
          onKeyDown={(e) => e.key === 'Enter' && onSelect(t.id)}
          className={cn(
            '-mb-px flex h-8 shrink-0 cursor-pointer items-center gap-1.5 rounded-t-md border border-b-0 px-2.5 text-[12px]',
            t.id === active ? 'border-border bg-card font-medium' : 'border-transparent text-muted-foreground',
          )}
        >
          {t.byAgent && <span className="size-1.5 animate-pulse rounded-full bg-ok" aria-label="läuft" />}
          {editing === t.id ? (
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
                  onRename(t.id, draft.trim())
                  setEditing(undefined)
                }
                if (e.key === 'Escape') setEditing(undefined)
              }}
              className="w-28 rounded-sm border border-border bg-background px-1 text-[12px]"
            />
          ) : (
            t.name
          )}
          {canDrive && t.id === active && editing !== t.id && (
            <button
              aria-label="Umbenennen"
              onClick={(e) => {
                e.stopPropagation()
                setDraft(t.name)
                setEditing(t.id)
              }}
            >
              <Pencil className="size-3 text-muted-foreground" />
            </button>
          )}
          {canDrive && (
            <button
              aria-label={`${t.name} schließen`}
              onClick={(e) => {
                e.stopPropagation()
                onClose(t.id)
              }}
            >
              <X className="size-3 text-muted-foreground" />
            </button>
          )}
        </span>
      ))}
      {canDrive && (
        <button onClick={onNew} aria-label="Neues Terminal" className="mb-1 ml-1 flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent">
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
  useEffect(() => {
    let disposed = false
    let x: Xterm | undefined
    let channel: ReturnType<ReturnType<typeof client.session>['terminals']['attach']> | undefined
    let observer: ResizeObserver | undefined
    let resizeTimer: ReturnType<typeof setTimeout> | undefined
    let opens = 0
    let readOnly = true
    const session = client.session(sessionId)
    void load().then((mod) => {
      if (disposed || !box.current) return
      x = mod.createTerminal(box.current, false)
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
      channel?.close()
      x?.dispose()
    }
    // Der Kanal gilt für genau dieses Terminal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, terminal.id])
  return (
    <div data-testid="terminal-screen" className="dark min-h-0 flex-1 bg-sunken p-2">
      <div ref={box} className="h-full w-full" />
    </div>
  )
}

/**
 * Terminals einer Session in der Workspace-Rail (SES-019, WEB-010): benannte Tabs, neue
 * Shell, Umbenennen, Schließen; vom Agent gestartete Langläufer erscheinen mit.
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
  const terminals = useMemo(() => openTerminals(events, runnerGone), [events, runnerGone])
  const sandboxed = useMemo(() => toolSandbox(events), [events])
  const [active, setActive] = useState<string | undefined>()
  const [access, setAccess] = useState<ChannelAccess | undefined>()
  const [reattached, setReattached] = useState(false)
  const [error, setError] = useState<string | undefined>()
  const [size, setSize] = useState<{ cols: number; rows: number } | undefined>()
  const current = terminals.find((t) => t.id === active) ?? terminals.at(-1)
  // Lokal steuert der Owner; mit Rollen (COL-001, M4) entscheidet das Recht des Kanals.
  const canDrive = access !== 'read'
  const session = client.session(sessionId)

  const run = (p: Promise<unknown>) =>
    p.catch((e: unknown) => setError((e as { problem?: { detail?: string } }).problem?.detail ?? String(e)))

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
        onNew={() => {
          setError(undefined)
          void run(session.terminals.open(START).then((t) => setActive(t.terminal_id)))
        }}
        onClose={(id) => void run(session.terminals.close(id))}
        onRename={(id, name) => void run(session.terminals.rename(id, name))}
      />
      {error && (
        <Callout tone="deny" className="m-2">
          {error}
        </Callout>
      )}
      {current && (
        <>
          <div className="flex h-8 shrink-0 items-center gap-3 border-b border-border px-3 text-[11px] text-muted-foreground">
            {current.byAgent ? (
              <>
                <span>Vom Agent gestartet · {runningFor(current.since)}</span>
                <span>Läuft weiter, auch wenn alle Fenster geschlossen sind</span>
              </>
            ) : sandboxed === false ? (
              // Ausdrücklich bestätigtes `backend: none` (SBX-006): nie als Sandbox ausgeben.
              <span className="inline-flex items-center gap-1 text-deny">Ohne Sandbox (backend: none)</span>
            ) : (
              <span className="inline-flex items-center gap-1">
                <Lock className="size-3" /> Sandbox: Profil „Tools“
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
