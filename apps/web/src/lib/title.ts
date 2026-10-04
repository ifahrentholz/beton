import type { Event } from '@beton/sdk'
import { payloadOf } from './events'

/** Platzhalter, solange eine Session keinen Titel hat (UX-009). */
export const PLACEHOLDER = 'Neue Session'

export type TitleSource = 'user' | 'generated' | 'harness'

export interface SessionTitle {
  title: string
  source?: TitleSource | undefined
}

function source(raw: string | undefined): TitleSource | undefined {
  return raw === 'user' || raw === 'generated' || raw === 'harness' ? raw : undefined
}

/**
 * Titel aus Listen-Stand und Log (UX-009): Ein Titel des Users bleibt; danach eintreffende
 * generierte Titel werden nicht angewendet (AC2).
 */
export function titleFrom(summary: { title?: string | undefined; title_source?: string | undefined } | undefined, events: readonly Event[] | undefined): SessionTitle {
  let current: SessionTitle = { title: summary?.title ?? '', source: source(summary?.title_source) }
  for (const e of events ?? []) {
    const t = payloadOf(e, 'session.title_changed')
    if (!t) continue
    const next = source(t.source)
    if (current.source === 'user' && next === 'generated') continue
    current = { title: t.title, source: next }
  }
  return current
}

/** Fenstertitel (Browser-Tab, Desktop-Fenster). */
export function windowTitle(title: string): string {
  return `${title.trim() || PLACEHOLDER} · beton`
}
