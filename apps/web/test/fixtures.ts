import type { Event } from '@beton/sdk'

let seq = 0
export function resetSeq(): void {
  seq = 0
}

/** Dauerhaftes Event mit fortlaufender `seq`. */
export function ev(type: string, payload: unknown, extra: Partial<Event> = {}): Event {
  seq += 1
  return {
    v: 1,
    id: `evt_${String(seq).padStart(26, '0')}`,
    session_id: 'ses_test',
    seq,
    ts: '2026-10-04T00:00:00Z',
    actor: { kind: 'system', component: 'runner' },
    type,
    payload,
    ...extra,
  } as unknown as Event
}

/** Transientes Event (ohne eigene `seq`). */
export function transient(type: string, payload: unknown): Event {
  return {
    v: 1,
    id: 'evt_t',
    session_id: 'ses_test',
    seq: 0,
    ts: '2026-10-04T00:00:00Z',
    actor: { kind: 'system', component: 'runner' },
    type,
    payload,
    transient: true,
  } as unknown as Event
}
