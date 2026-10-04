import { beforeEach, describe, expect, it } from 'vitest'
import { applyEvents, enqueue, flush, setScheduler, useEvents } from '@/store/events'
import { ev, resetSeq, transient } from './fixtures'

beforeEach(() => {
  resetSeq()
  useEvents.setState({ logs: {} })
})

describe('Event-Store', () => {
  it('WEB-015 AC2: 100 Deltas in einem Frame führen zu genau einem Render', () => {
    let frames: Array<() => void> = []
    setScheduler((f) => frames.push(f))
    let notifications = 0
    const unsub = useEvents.subscribe(() => (notifications += 1))
    for (let i = 0; i < 100; i += 1) {
      enqueue('ses_test', [transient('message.delta', { message_id: 'm1', text: 'x' })])
    }
    expect(frames).toHaveLength(1)
    expect(notifications).toBe(0)
    frames[0]!()
    frames = []
    unsub()
    expect(notifications).toBe(1)
    expect(useEvents.getState().logs['ses_test']?.streaming['m1']).toBe('x'.repeat(100))
  })

  it('Deltas weichen der abgeschlossenen Nachricht; Duplikate werden verworfen', () => {
    const base = { events: [], streaming: {}, reasoning: {}, toolOutput: {}, live: false, reconnecting: false }
    const delta = transient('message.delta', { message_id: 'm1', text: 'Hal' })
    const done = ev('message.completed', { message_id: 'm1', role: 'assistant', content: [{ type: 'text', text: 'Hallo' }] })
    let log = applyEvents(base, [delta])
    expect(log.streaming).toEqual({ m1: 'Hal' })
    log = applyEvents(log, [done, done])
    expect(log.streaming).toEqual({})
    expect(log.events).toHaveLength(1)
    // Snapshot ersetzt statt anzuhängen.
    log = applyEvents(log, [transient('message.delta', { message_id: 'm2', text: 'neu', snapshot: true })])
    expect(log.streaming['m2']).toBe('neu')
  })

  it('flush ohne Vorgemerktes ändert nichts', () => {
    let notifications = 0
    const unsub = useEvents.subscribe(() => (notifications += 1))
    flush()
    unsub()
    expect(notifications).toBe(0)
  })

  it('Turn-Ende räumt verwaiste Stream-Einträge ab', () => {
    const base = { events: [], streaming: {}, reasoning: {}, toolOutput: {}, live: false, reconnecting: false }
    let log = applyEvents(base, [transient('message.delta', { message_id: '', text: 'ohne ID' })])
    log = applyEvents(log, [
      ev('message.completed', { message_id: 'msg_1', role: 'assistant', content: [{ type: 'text', text: 'ohne ID' }] }),
      ev('turn.completed', { turn_id: 'trn_1', stop_reason: 'end_turn', usage_summary: {} }),
    ])
    expect(log.streaming).toEqual({})
  })
})
