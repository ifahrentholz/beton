import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Capabilities } from '@beton/sdk'
import { ContextBar, ContextRing } from '@/components/context-bar'
import { compacting, contextLevel, contextOf } from '@/lib/context'
import { timeline } from '@/lib/timeline'
import { ev, resetSeq } from './fixtures'

afterEach(cleanup)

const caps = (compaction: 'native' | 'none') => ({ compaction }) as unknown as Capabilities
const log = (events: ReturnType<typeof ev>[]) => ({ events, streaming: {}, reasoning: {}, toolOutput: {} })

describe('Kontextanzeige (USE-008)', () => {
  it('USE-008 AC1: nach context.usage 150000/200000 zeigt der Ring 75 % im Warnstil', () => {
    resetSeq()
    const context = contextOf([ev('context.usage', { used_tokens: 150_000, window_tokens: 200_000, source: 'harness' })])
    expect(context).toEqual({ used: 150_000, window: 200_000, source: 'harness' })
    render(<ContextRing {...context!} />)
    const ring = screen.getByTestId('context-ring')
    expect(ring).toHaveProperty('dataset.level', 'warn')
    expect(ring.textContent).toContain('75 % Kontext')
    expect(ring.textContent).toContain('wird knapp')
    expect(ring.querySelector('.text-warn')).not.toBeNull()
    // Tooltip mit used/window und Quelle.
    expect(ring.getAttribute('title')).toBe('150.000 / 200.000 Tokens · Quelle: Harness')
  })

  it('USE-008 AC2: ohne bekanntes Fenster erscheint „150 k Tokens“ ohne Prozentangabe', () => {
    resetSeq()
    const context = contextOf([ev('context.usage', { used_tokens: 150_000, window_tokens: 0, source: 'estimated' })])
    expect(context?.window).toBeNull()
    render(<ContextRing {...context!} />)
    const ring = screen.getByTestId('context-ring')
    expect(ring.textContent).toBe('150 k Tokens')
    expect(ring.textContent).not.toContain('%')
    expect(ring.querySelector('svg')).toBeNull()
  })

  it('USE-008: Farbschwellen 70 / 85 / 95 %', () => {
    expect(contextLevel(138_000, 200_000).level).toBe('ok')
    expect(contextLevel(140_000, 200_000).level).toBe('warn')
    expect(contextLevel(170_000, 200_000).level).toBe('high')
    expect(contextLevel(190_000, 200_000).level).toBe('full')
    expect(contextLevel(190_000, null)).toEqual({ pct: null, level: 'plain' })
  })
})

describe('Compaction (USE-009)', () => {
  it('USE-009 AC3: bei compaction: none ist der Button deaktiviert und zeigt einen Tooltip mit Grund', () => {
    const onCompact = vi.fn()
    render(
      <ContextBar
        harness="direct:ollama"
        capabilities={caps('none')}
        context={{ used: 61_000, window: 131_072, source: 'harness' }}
        compacting={false}
        running={false}
        onCompact={onCompact}
      />,
    )
    const button = screen.getByRole('button', { name: /Kompaktieren/ })
    expect(button).toHaveProperty('disabled', true)
    expect(button.getAttribute('title')).toBe('direct:ollama kann nicht kompaktieren. Starte bei Bedarf eine neue Session.')
    expect(screen.getByText('nicht verfügbar für diesen Harness')).toBeTruthy()
    button.click()
    expect(onCompact).not.toHaveBeenCalled()
  })

  it('USE-009 AC1: Klick auf „Kompaktieren“ stößt die Compaction an; Verlauf und Anzeige folgen den Events', () => {
    resetSeq()
    const onCompact = vi.fn()
    const before = [
      ev('session.created', { harness: 'claude', owner: { kind: 'user', id: 'usr_1' }, kind: 'interactive', cwd: '/w', trigger: 'user' }),
      ev('context.usage', { used_tokens: 186_400, window_tokens: 200_000, source: 'harness' }),
    ]
    const { rerender } = render(
      <ContextBar harness="claude" capabilities={caps('native')} context={contextOf(before)} compacting={false} running={false} onCompact={onCompact} />,
    )
    screen.getByRole('button', { name: /Kompaktieren/ }).click()
    expect(onCompact).toHaveBeenCalledTimes(1)
    expect(screen.getByTestId('context-ring').textContent).toContain('93 % Kontext')

    const running = [...before, ev('compaction.started', { before_tokens: 186_400, trigger: 'user' })]
    expect(compacting(running)).toBe(true)
    rerender(<ContextBar harness="claude" capabilities={caps('native')} context={contextOf(running)} compacting running={false} onCompact={onCompact} />)
    expect(screen.getByRole('button', { name: /Kompaktiert …/ })).toHaveProperty('disabled', true)
    expect(timeline(log(running)).map((i) => (i.kind === 'note' ? i.text : i.kind))).toEqual(['Kontext wird kompaktiert … (von dir ausgelöst)'])

    const done = [
      ...running,
      ev('compaction.completed', { before_tokens: 186_400, after_tokens: 38_200, trigger: 'user' }),
      ev('context.usage', { used_tokens: 38_200, window_tokens: 200_000, source: 'harness' }),
    ]
    expect(compacting(done)).toBe(false)
    rerender(<ContextBar harness="claude" capabilities={caps('native')} context={contextOf(done)} compacting={false} running={false} onCompact={onCompact} />)
    expect(screen.getByTestId('context-ring').textContent).toBe('19 % Kontext')
    const notes = timeline(log(done)).filter((i) => i.kind === 'note')
    expect(notes).toHaveLength(1)
    expect(notes[0]).toMatchObject({
      text: 'Kontext kompaktiert · 186.400 → 38.200 Tokens · von dir ausgelöst',
      detail: 'Claude Code hat den bisherigen Verlauf zusammengefasst. Der volle Verlauf bleibt in beton erhalten und sichtbar.',
    })
  })

  it('USE-009: während eines Turns und ohne Capabilities ist der Button deaktiviert', () => {
    const props = { harness: 'claude', context: { used: 1000, window: 200_000, source: 'harness' as const }, compacting: false, onCompact: vi.fn() }
    const { rerender } = render(<ContextBar {...props} capabilities={caps('native')} running />)
    expect(screen.getByRole('button', { name: /Kompaktieren/ })).toHaveProperty('disabled', true)
    rerender(<ContextBar {...props} capabilities={undefined} running={false} />)
    expect(screen.getByRole('button', { name: /Kompaktieren/ })).toHaveProperty('disabled', true)
  })
})
