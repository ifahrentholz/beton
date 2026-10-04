import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Event, HarnessInfo } from '@beton/sdk'
import { continueTargets, forkOrigin, HarnessMenu } from '@/components/fork'

afterEach(() => cleanup())

function ev(seq: number, type: string, payload: unknown): Event {
  return { v: 1, id: `evt_${seq}`, session_id: 'ses_b', seq, ts: '2026-10-04T10:00:00Z', actor: { kind: 'system', component: 'runner' }, type, payload } as unknown as Event
}

function harness(id: string, extra: Partial<{ installed: boolean; fork: string; incompatible: boolean }> = {}): HarnessInfo {
  return {
    id,
    modes: ['native'],
    capabilities: [{ fork_history: extra.fork ?? 'preamble' }],
    probe: { installed: extra.installed ?? true, auth_status: 'logged_in' },
    incompatible: extra.incompatible ?? false,
  } as unknown as HarnessInfo
}

describe('Fork', () => {
  it('SES-007 AC4: Herkunft des Forks aus session.forked (Quelle, Harness, übernommene Turns)', () => {
    const events = [
      ev(1, 'session.created', { harness: 'codex' }),
      ev(2, 'turn.started', {}),
      ev(3, 'turn.completed', {}),
      ev(4, 'turn.started', {}),
      ev(5, 'turn.completed', {}),
      ev(6, 'session.forked', { from_session: 'ses_a', at_seq: 41, reason: 'user', harness: 'codex', from_harness: 'claude', history_mode: 'preamble' }),
      ev(7, 'turn.started', {}),
    ]
    expect(forkOrigin(events)).toEqual({ from: 'ses_a', harness: 'codex', fromHarness: 'claude', atSeq: 41, turns: 2 })
    expect(forkOrigin(events.slice(0, 5))).toBeUndefined()
  })

  it('SES-007 AC5: „Weiter mit …“ bietet nur andere Harnesses mit übernehmbarem Verlauf an', () => {
    const catalog = [
      harness('claude', { fork: 'rebuild' }),
      harness('codex'),
      harness('acp:gemini', { installed: false }),
      harness('acp:goose', { fork: 'none' }),
      harness('fake'),
    ]
    const targets = continueTargets(catalog, 'claude', (id) => id !== 'fake')
    expect(targets.map((h) => h.id)).toEqual(['codex'])

    const onContinue = vi.fn()
    render(<HarnessMenu harness="claude" targets={targets} onContinue={onContinue} />)
    expect(screen.queryByRole('menu')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Harness' }))
    expect(screen.getByRole('menu').textContent).toContain('diese bleibt bei Claude Code')
    fireEvent.click(screen.getByRole('menuitem', { name: /Weiter mit Codex/ }))
    expect(onContinue).toHaveBeenCalledWith('codex')
    expect(screen.queryByRole('menu')).toBeNull()
  })
})
