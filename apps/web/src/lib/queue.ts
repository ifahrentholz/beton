import type { Event, QueueItem } from '@beton/sdk'
import { payloadOf } from './events'

/** Serverseitige Queue einer Session (SES-004) laut letztem `queue.updated`. */
export interface QueueState {
  items: QueueItem[]
  paused: boolean
}

const EMPTY: QueueState = { items: [], paused: false }

/** Letzter Queue-Stand aus dem Log; leer, solange es keinen gibt. */
export function queueOf(events: readonly Event[] | undefined): QueueState {
  for (let i = (events?.length ?? 0) - 1; i >= 0; i--) {
    const q = payloadOf(events![i]!, 'queue.updated')
    if (q) return { items: q.items, paused: q.paused }
  }
  return EMPTY
}

/** Der lokale User (M0/M1: genau einer). */
export const ME = 'usr_local'

/** Anzeige des Autors: „Du“ oder die ID der Person. */
export function authorLabel(author: string): string {
  return author === ME ? 'Du' : author
}

/** Reihenfolge nach Drag & Drop: `id` an `to` (für die sofortige Anzeige vor dem Server-Stand). */
export function moved<T extends { id: string }>(items: readonly T[], id: string, to: number): T[] {
  const from = items.findIndex((i) => i.id === id)
  if (from < 0) return [...items]
  const next = [...items]
  const [it] = next.splice(from, 1)
  next.splice(Math.min(to, next.length), 0, it!)
  return next
}
