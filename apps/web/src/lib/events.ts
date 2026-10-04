import type { Event, EventPayload, OffloadedPayload } from '@beton/sdk'

/** Event mit eingebetteter Nutzlast. */
export type InlineEvent = Event & EventPayload
/** Event, dessen Nutzlast im Blob-Store liegt (PROTO-001 AC4). */
export type OffloadedEvent = Event & OffloadedPayload

export function isInline(e: Event): e is InlineEvent {
  return 'payload' in e
}

export function isOffloaded(e: Event): e is OffloadedEvent {
  return 'payload_ref' in e
}

/** Nutzlast eines Event-Typs. */
export type PayloadOf<T extends EventPayload['type']> = Extract<EventPayload, { type: T }>['payload']

/** Nutzlast eines bestimmten Typs, sonst `undefined`. */
export function payloadOf<T extends EventPayload['type']>(e: Event, type: T): PayloadOf<T> | undefined {
  if (e.type !== type || !isInline(e)) return undefined
  return (e as unknown as { payload: never }).payload
}

/** Text einer Nachricht aus ihren Inhaltsblöcken. */
export function textOf(content: unknown[]): string {
  return content
    .map((b) => {
      if (typeof b === 'string') return b
      if (b && typeof b === 'object' && (b as { type?: unknown }).type === 'text') {
        return String((b as { text?: unknown }).text ?? '')
      }
      return ''
    })
    .join('')
}
