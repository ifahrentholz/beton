import { useEffect } from 'react'
import { client } from '@/lib/client'
import { enqueue, useEvents } from '@/store/events'

/**
 * Hält den Event-Strom einer Session offen (Replay, dann live, mit Resume). Ein schon
 * geladenes Log bleibt beim Session-Wechsel erhalten; der Strom setzt an seiner letzten
 * `seq` an (schneller Wechsel, WEB-015).
 */
export function useSessionStream(sessionId: string): void {
  useEffect(() => {
    const controller = new AbortController()
    const { reset, setReconnecting } = useEvents.getState()
    const cached = useEvents.getState().logs[sessionId]
    const fromSeq = cached?.events.at(-1)?.seq ?? 0
    if (!cached) reset(sessionId)
    const stream = client.session(sessionId).events({
      fromSeq,
      signal: controller.signal,
      onReconnecting: () => setReconnecting(sessionId, true),
      onReset: () => reset(sessionId),
    })
    void (async () => {
      try {
        for await (const e of stream) {
          if (useEvents.getState().logs[sessionId]?.reconnecting) setReconnecting(sessionId, false)
          enqueue(sessionId, [e])
        }
      } catch {
        if (!controller.signal.aborted) setReconnecting(sessionId, true)
      }
    })()
    return () => controller.abort()
  }, [sessionId])
}
