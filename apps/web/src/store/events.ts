import { create } from 'zustand'
import type { Event } from '@beton/sdk'
import { isInline } from '@/lib/events'

/** Live-Zustand einer Session: dauerhafte Events plus laufende Deltas. */
export interface SessionLog {
  /** Dauerhafte Events in `seq`-Reihenfolge. */
  events: Event[]
  /** Text der gerade gestreamten Nachrichten (`message.delta`), je `message_id`. */
  streaming: Record<string, string>
  /** Gestreamtes Reasoning (`reasoning.delta`), je `message_id`. */
  reasoning: Record<string, string>
  /** Ausgabe laufender Tool-Calls (`tool.call.output.delta`), je `call_id`. */
  toolOutput: Record<string, string>
  /** Replay abgeschlossen, Strom live. */
  live: boolean
  /** Verbindung unterbrochen, Neuaufbau läuft. */
  reconnecting: boolean
}

const empty = (): SessionLog => ({
  events: [],
  streaming: {},
  reasoning: {},
  toolOutput: {},
  live: false,
  reconnecting: false,
})

interface EventsState {
  logs: Record<string, SessionLog>
  setLive: (session: string, live: boolean) => void
  setReconnecting: (session: string, reconnecting: boolean) => void
  reset: (session: string) => void
}

export const useEvents = create<EventsState>()((set) => ({
  logs: {},
  setLive: (session, live) =>
    set((s) => ({ logs: { ...s.logs, [session]: { ...(s.logs[session] ?? empty()), live } } })),
  setReconnecting: (session, reconnecting) =>
    set((s) => ({ logs: { ...s.logs, [session]: { ...(s.logs[session] ?? empty()), reconnecting } } })),
  reset: (session) => set((s) => ({ logs: { ...s.logs, [session]: empty() } })),
}))

/** Wendet einen Stapel Events auf das Log einer Session an (rein, für Tests exportiert). */
export function applyEvents(log: SessionLog, batch: Event[]): SessionLog {
  let events = log.events
  let streaming = log.streaming
  let reasoning = log.reasoning
  let toolOutput = log.toolOutput
  const lastSeq = events.at(-1)?.seq ?? 0
  const fresh: Event[] = []
  for (const e of batch) {
    if (e.transient) {
      if (!isInline(e)) continue
      if (e.type === 'message.delta' || e.type === 'reasoning.delta') {
        const target = e.type === 'message.delta' ? 'streaming' : 'reasoning'
        const map = target === 'streaming' ? (streaming = { ...streaming }) : (reasoning = { ...reasoning })
        const id = e.payload.message_id
        map[id] = e.payload.snapshot ? e.payload.text : (map[id] ?? '') + e.payload.text
      } else if (e.type === 'tool.call.output.delta') {
        toolOutput = { ...toolOutput }
        const id = e.payload.call_id
        toolOutput[id] = e.payload.snapshot ? e.payload.text : (toolOutput[id] ?? '') + e.payload.text
      }
      continue
    }
    if (e.seq <= (fresh.at(-1)?.seq ?? lastSeq)) continue
    fresh.push(e)
    if (isInline(e)) {
      if (e.type === 'message.completed' && e.payload.message_id in streaming) {
        streaming = { ...streaming }
        delete streaming[e.payload.message_id]
      }
      if (e.type === 'reasoning.completed' && e.payload.message_id in reasoning) {
        reasoning = { ...reasoning }
        delete reasoning[e.payload.message_id]
      }
      // Turn-Ende: was noch als Stream offen ist, ist verwaist (z. B. Delta ohne passende
      // Nachrichten-ID) und würde sonst neben der fertigen Nachricht stehen bleiben.
      if (e.type === 'turn.completed' || e.type === 'turn.failed' || e.type === 'turn.interrupted') {
        if (Object.keys(streaming).length) streaming = {}
        if (Object.keys(reasoning).length) reasoning = {}
      }
    }
  }
  if (fresh.length) events = events.concat(fresh)
  return { ...log, events, streaming, reasoning, toolOutput }
}

// --- Bündelung pro Animation-Frame (WEB-015 AC2) ---------------------------------------

type Scheduler = (flush: () => void) => void
let scheduler: Scheduler = (flush) => requestAnimationFrame(() => flush())
const pending = new Map<string, Event[]>()
let scheduled = false

/** Für Tests: eigenen Taktgeber setzen. */
export function setScheduler(s: Scheduler): void {
  scheduler = s
}

/** Events vormerken; angewendet wird einmal pro Frame in einem einzigen Store-Update. */
export function enqueue(session: string, events: Event[]): void {
  const list = pending.get(session)
  if (list) list.push(...events)
  else pending.set(session, [...events])
  if (!scheduled) {
    scheduled = true
    scheduler(flush)
  }
}

export function flush(): void {
  scheduled = false
  if (pending.size === 0) return
  const batches = [...pending.entries()]
  pending.clear()
  useEvents.setState((s) => {
    const logs = { ...s.logs }
    for (const [session, batch] of batches) {
      logs[session] = applyEvents(logs[session] ?? empty(), batch)
    }
    return { logs }
  })
}
