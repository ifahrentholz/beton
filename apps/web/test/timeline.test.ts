import { beforeEach, describe, expect, it } from 'vitest'
import type { Event } from '@beton/sdk'
import { openApprovals, settingsNotes, timeline } from '@/lib/timeline'
import { ev, resetSeq } from './fixtures'

const empty = { streaming: {}, reasoning: {}, toolOutput: {} }

beforeEach(() => resetSeq())

function tool(callId: string, ...after: string[]) {
  const events = [ev('tool.call.requested', { call_id: callId, tool: 'Bash', args: { command: 'ls -la' }, source: 'harness' })]
  for (const step of after) {
    if (step === 'started') events.push(ev('tool.call.started', { call_id: callId }))
    else events.push(ev('tool.call.completed', { call_id: callId, status: step, duration_ms: 1200 }))
  }
  return events
}

describe('Timeline', () => {
  it('WEB-002 AC2: Tool-Cards mit Name, Kurzform und Status requested|running|completed|failed|denied', () => {
    const events = [...tool('a'), ...tool('b', 'started'), ...tool('c', 'started', 'ok'), ...tool('d', 'error'), ...tool('e', 'denied')]
    const items = timeline({ ...empty, events })
    const cards = items.filter((i) => i.kind === 'tool')
    expect(cards.map((c) => c.kind === 'tool' && c.status)).toEqual(['requested', 'running', 'completed', 'failed', 'denied'])
    const c = cards[2]!
    expect(c.kind === 'tool' && [c.tool, c.durationMs]).toEqual(['Bash', 1200])
  })

  it('WEB-002 AC3: policy.decision mit deny erscheint als Karte mit Regelname und Begründung', () => {
    const events = [
      ev('policy.decision', {
        decision_id: 'd1',
        phase: 'tool',
        outcome: 'deny',
        matched: [{ rule: 'project/no-push', reason: 'Pushes auf main sind gesperrt' }],
        defaults_applied: [],
        conflicts: [],
        errors: [],
        grants_used: [],
        enforcement: 'enforce',
        policy_set_hash: 'h',
        eval_us: 3,
      }),
      ev('policy.decision', { decision_id: 'd2', phase: 'tool', outcome: 'allow', matched: [], defaults_applied: [], conflicts: [], errors: [], grants_used: [], enforcement: 'enforce', policy_set_hash: 'h', eval_us: 1 }),
    ]
    const items = timeline({ ...empty, events })
    expect(items).toEqual([{ kind: 'policy', key: 'e1', rule: 'project/no-push', reason: 'Pushes auf main sind gesperrt' }])
  })

  it('WEB-018: Freigabe offen, dann entschieden', () => {
    const req = ev('approval.requested', {
      approval_id: 'apr_1',
      kind: 'tool',
      subject: { tool: 'Bash', args: { command: 'git push' } },
      options: ['allow', 'deny'],
      expires_at: '2026-10-04T00:00:00Z',
      on_timeout: 'deny',
    })
    let items = timeline({ ...empty, events: [req] })
    expect(openApprovals(items)).toHaveLength(1)
    const res = ev('approval.resolved', { approval_id: 'apr_1', decision: 'deny', actor: { kind: 'user', id: 'usr_local' }, via: 'user', comment: 'Erst testen' })
    items = timeline({ ...empty, events: [req, res] })
    expect(openApprovals(items)).toHaveLength(0)
    const a = items[0]!
    expect(a.kind === 'approval' && [a.decision, a.comment]).toEqual(['deny', 'Erst testen'])
  })

  it('Eingaben, Antworten und laufende Deltas in Reihenfolge', () => {
    const events = [
      ev('message.completed', { message_id: 'u', role: 'user', content: [{ type: 'text', text: 'Hallo' }], author: 'usr_local' }),
      ev('message.completed', { message_id: 'a', role: 'assistant', content: [{ type: 'text', text: 'Hi' }] }),
    ]
    const items = timeline({ ...empty, events, streaming: { b: 'Wei' } })
    expect(items.map((i) => i.kind)).toEqual(['user', 'assistant', 'assistant'])
    expect(items[2]).toMatchObject({ text: 'Wei', streaming: true })
  })

  it('PROTO-001 AC4: ausgelagerte Nutzlast wird zum Platzhalter', () => {
    const offloaded = { ...ev('message.completed', {}), payload_ref: 'sha256:abc' } as Record<string, unknown>
    delete offloaded.payload
    const items = timeline({ ...empty, events: [offloaded as unknown as Event] })
    expect(items).toEqual([{ kind: 'offloaded', key: 'e1', eventType: 'message.completed', ref: 'sha256:abc', seq: 1 }])
  })

  it('HAR-017 AC1, AC4: Modellwechsel als Hinweis mit dem Turn, ab dem er gilt', () => {
    const events = [
      ev('session.created', { harness: 'claude', model: 'claude-opus-5-5', cwd: '/', owner: 'usr_local', kind: 'main', trigger: 'api' }),
      ev('turn.started', { turn_id: 'trn_1', author: 'usr_local' }),
      ev('turn.completed', { turn_id: 'trn_1', stop_reason: 'end_turn', usage_summary: {} }),
      ev('session.settings_changed', { model: 'claude-sonnet-5-5', mechanism: 'live', effective_from_turn: 'trn_2' }),
      ev('turn.started', { turn_id: 'trn_2', author: 'usr_local' }),
    ]
    const notes = timeline({ ...empty, events }).filter((i) => i.kind === 'note')
    expect(notes.map((n) => n.kind === 'note' && n.text)).toEqual([
      'Modell gewechselt: claude-opus-5-5 → claude-sonnet-5-5 · sofort wirksam, ab Turn 2 · Verlauf bleibt erhalten',
    ])
  })

  it('HAR-017 AC2: Wechsel per Neustart nennt den Neustart', () => {
    expect(settingsNotes({ model: 'gemini-3-flash', mechanism: 'restart' }, { harness: 'claude', turn: 3 })).toEqual([
      'Modell gewechselt auf gemini-3-flash · Claude Code wurde neu gestartet und hat die Session fortgesetzt',
    ])
  })

  it('HAR-017 AC3: gemappter Effort zeigt angefragte und aktive Stufe', () => {
    expect(settingsNotes({ effort: 'high', requested_effort: 'xhigh', mechanism: 'live' }, { harness: 'codex', previousModel: 'gpt-5.3', turn: 1 })).toEqual([
      'Effort: „sehr hoch“ angefragt, „hoch“ aktiv – gpt-5.3 unterstützt keine höhere Stufe',
    ])
    // Reiner Effort- oder Moduswechsel: kein Hinweis (der Picker zeigt den Stand).
    expect(settingsNotes({ effort: 'low', permission_mode: 'plan' } as never, { harness: 'codex', turn: 1 })).toEqual([])
  })
})
