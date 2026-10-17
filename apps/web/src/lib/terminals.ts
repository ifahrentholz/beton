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
}

/**
 * Laufende Terminals aus dem Log: `terminal.opened` ohne `terminal.closed`, mit dem letzten
 * Namen aus `terminal.renamed`. Endet der Runner, sind sie mit ihm beendet (`runnerGone`).
 */
export function openTerminals(events: readonly Event[] | undefined, runnerGone = false): TerminalInfo[] {
  if (runnerGone) return []
  const open = new Map<string, TerminalInfo>()
  for (const e of events ?? []) {
    const o = payloadOf(e, 'terminal.opened')
    if (o) {
      open.set(o.terminal_id, {
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
      const t = open.get(r.terminal_id)
      if (t) t.name = r.name
      continue
    }
    const c = payloadOf(e, 'terminal.closed')
    if (c) open.delete(c.terminal_id)
    // Ein neuer Runner (Neustart, Resume) beginnt ohne Terminals.
    if (e.type === 'session.started') open.clear()
  }
  return [...open.values()]
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
