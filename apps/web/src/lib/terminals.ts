import type { Event, TerminalKind } from '@beton/sdk'
import { payloadOf } from './events'

/** Ein Terminal der Session laut Log (SES-019, WEB-010). */
export interface TerminalInfo {
  id: string
  name: string
  kind: TerminalKind
  cmd: string
  cols: number
  rows: number
  /** Zeitpunkt des Starts (`terminal.opened`). */
  since: string
  /** Gestartet vom Agent (`kind: tool`) bzw. vom Benutzer. */
  byAgent: boolean
  /** Beendet: mit `terminal.closed` (Zeit, Exit-Code) oder mit dem Runner. */
  ended?: TerminalEnd
  /** Letzter Bildschirmzustand (`terminal.snapshot`, DATA-011). */
  snapshot?: { ref: string; cols: number; rows: number }
}

export type TerminalEnd = { at: string; exitCode?: number } | { withRunner: true }

/**
 * Alle Terminals der Session aus dem Log, in der Reihenfolge des Starts: laufende und beendete
 * (DATA-011 AC2: Ein beendetes Terminal bleibt mit seinem letzten Snapshot sichtbar). Der Name
 * kommt aus dem letzten `terminal.renamed`. Ein neuer Runner (`session.started`) bzw. das Ende
 * des Runners (`runnerGone`) beendet die laufenden mit.
 */
export function sessionTerminals(events: readonly Event[] | undefined, runnerGone = false): TerminalInfo[] {
  const all = new Map<string, TerminalInfo>()
  const endRunning = () => {
    for (const t of all.values()) if (!t.ended) t.ended = { withRunner: true }
  }
  for (const e of events ?? []) {
    const o = payloadOf(e, 'terminal.opened')
    if (o) {
      all.delete(o.terminal_id)
      all.set(o.terminal_id, {
        id: o.terminal_id,
        name: o.name ?? 'Terminal',
        kind: o.kind,
        cmd: o.cmd,
        cols: o.cols,
        rows: o.rows,
        since: e.ts,
        byAgent: o.kind === 'tool',
      })
      continue
    }
    const r = payloadOf(e, 'terminal.renamed')
    if (r) {
      const t = all.get(r.terminal_id)
      if (t) t.name = r.name
      continue
    }
    const snap = payloadOf(e, 'terminal.snapshot')
    if (snap) {
      const t = all.get(snap.terminal_id)
      if (t) t.snapshot = { ref: snap.blob_ref, cols: snap.cols, rows: snap.rows }
      continue
    }
    const c = payloadOf(e, 'terminal.closed')
    if (c) {
      const t = all.get(c.terminal_id)
      if (t && !t.ended) t.ended = c.exit_code === undefined ? { at: e.ts } : { at: e.ts, exitCode: c.exit_code }
      continue
    }
    if (e.type === 'session.started') endRunning()
  }
  if (runnerGone) endRunning()
  return [...all.values()]
}

/** Laufende Terminals: `terminal.opened` ohne `terminal.closed` im aktuellen Runner. */
export function openTerminals(events: readonly Event[] | undefined, runnerGone = false): TerminalInfo[] {
  return sessionTerminals(events, runnerGone).filter((t) => !t.ended)
}

/** „läuft seit 14 Min.“ */
export function runningFor(since: string, now = Date.now()): string {
  const min = Math.max(0, Math.floor((now - Date.parse(since)) / 60_000))
  if (min < 1) return 'läuft seit weniger als einer Minute'
  if (min < 60) return `läuft seit ${min} Min.`
  return `läuft seit ${Math.floor(min / 60)} Std. ${min % 60} Min.`
}

/** Laufen Tools (und damit Terminals) in der Sandbox? Laut letztem `session.started`. */
export function toolSandbox(events: readonly Event[] | undefined): boolean | undefined {
  for (let i = (events?.length ?? 0) - 1; i >= 0; i--) {
    const s = payloadOf(events![i]!, 'session.started')
    if (s) return s.sandbox?.tool_sandbox ?? false
  }
  return undefined
}
