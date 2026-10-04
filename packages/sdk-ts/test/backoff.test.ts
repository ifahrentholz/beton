import { describe, expect, it } from 'vitest'
import { Backoff, BACKOFF_MAX_MS, shutdownDelay } from '../src/index.js'

describe('Backoff', () => {
  it('PROTO-009: 0,5 s → 30 s exponentiell mit ±20 % Jitter (wie beton_proto::ws::Backoff)', () => {
    const b = new Backoff()
    expect(b.nextWith(0)).toBe(500)
    expect(b.nextWith(0)).toBe(1000)
    expect(b.nextWith(1)).toBe(2400)
    expect(b.nextWith(-1)).toBe(3200)
    for (let i = 0; i < 20; i += 1) b.nextWith(0)
    expect(b.nextWith(0)).toBe(BACKOFF_MAX_MS)
    expect(b.nextWith(1)).toBe(BACKOFF_MAX_MS * 1.2)
    b.reset()
    expect(b.nextWith(0)).toBe(500)
  })

  it('PROTO-009: nach 4503 höchstens 2 s Jitter', () => {
    expect(shutdownDelay(0)).toBe(0)
    expect(shutdownDelay(1)).toBe(2000)
    expect(shutdownDelay(0.5)).toBe(1000)
  })
})
