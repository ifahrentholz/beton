import type { Event } from '@beton/sdk'
import { payloadOf } from './events'

/** Kontextfüllstand aus dem letzten `context.usage` (SES-011, USE-008). */
export interface ContextState {
  used: number
  /** `null`: Fenster unbekannt (weder Harness noch Katalog). */
  window: number | null
  source: 'harness' | 'estimated'
}

/** Letzter gemeldeter Kontext der Session; `window_tokens: 0` heißt unbekannt. */
export function contextOf(events: readonly Event[] | undefined): ContextState | undefined {
  for (let i = (events?.length ?? 0) - 1; i >= 0; i -= 1) {
    const c = payloadOf(events![i]!, 'context.usage')
    if (c) return { used: c.used_tokens, window: c.window_tokens > 0 ? c.window_tokens : null, source: c.source }
  }
  return undefined
}

/** Läuft gerade eine Compaction (`compaction.started` ohne folgendes `completed`)? */
export function compacting(events: readonly Event[] | undefined): boolean {
  for (let i = (events?.length ?? 0) - 1; i >= 0; i -= 1) {
    const e = events![i]!
    if (e.type === 'compaction.completed') return false
    if (e.type === 'compaction.started') return true
  }
  return false
}

/** Stufe des Rings: Schwellen 70 / 85 / 95 % (USE-008); ohne Fenster nur absolute Tokens. */
export type ContextLevel = 'plain' | 'ok' | 'warn' | 'high' | 'full'

export function contextLevel(used: number, window: number | null): { pct: number | null; level: ContextLevel } {
  if (!window) return { pct: null, level: 'plain' }
  const pct = Math.round((used / window) * 100)
  const level: ContextLevel = pct >= 95 ? 'full' : pct >= 85 ? 'high' : pct >= 70 ? 'warn' : 'ok'
  return { pct, level }
}

/** Text neben dem Ring: „wird knapp“, „fast voll“, „voll“. */
export const LEVEL_LABEL: Record<ContextLevel, string> = { plain: '', ok: '', warn: 'wird knapp', high: 'fast voll', full: 'voll' }
