import { describe, expect, it } from 'vitest'
import type { SessionSummary } from '@beton/sdk'
import { listRows } from '@/components/session-list'
import { harnessVisible } from '@/lib/features'
import { deltaSince } from '@/store/sessions'

const s = (id: string, activity: string, extra: Partial<SessionSummary> = {}): SessionSummary => ({
  id: id as SessionSummary['id'],
  title: id,
  status: 'idle',
  kind: 'main',
  harness: 'fake',
  archived: false,
  head_seq: 5,
  cost_micro: 0,
  created_at: activity,
  last_activity_at: activity,
  pinned: false,
  read_seq: 5,
  unread: false,
  changed_at: activity,
  ...extra,
})

describe('Session-Liste', () => {
  it('SES-012 AC3: Angepinnte stehen unabhängig von der Sortierung oben', () => {
    const rows = listRows(
      [
        s('ses_neu', '2026-10-04T10:00:00.000Z'),
        s('ses_alt', '2026-10-01T10:00:00.000Z', { pinned: true }),
        s('ses_mitte', '2026-10-03T10:00:00.000Z'),
      ],
      false,
    )
    expect(rows.map((r) => (r.kind === 'heading' ? `# ${r.label}` : r.session.id))).toEqual([
      '# Angepinnt',
      'ses_alt',
      'ses_neu',
      'ses_mitte',
    ])
    // Ohne angepinnte Sessions keine Gruppe; Suche zeigt nur Treffer.
    const plain = listRows([s('ses_a', '2026-10-04T10:00:00.000Z'), s('ses_b', '2026-10-03T10:00:00.000Z')], false, new Set(['ses_b']))
    expect(plain).toEqual([{ kind: 'session', session: expect.objectContaining({ id: 'ses_b' }) }])
  })

  it('SES-012 AC2: Deltas fragen mit Überlappung ab, damit kein Gelesen-Stand verloren geht', () => {
    expect(deltaSince('2026-10-04T10:00:02.000Z')).toBe('2026-10-04T10:00:00.000Z')
    expect(deltaSince('1970-01-01T00:00:00.000Z')).toBe('1970-01-01T00:00:00.000Z')
  })

  it('UX-007 AC1: Der Fake-Harness ist ohne Flag in der Auswahl ausgeblendet', () => {
    expect(harnessVisible('fake', new Set())).toBe(false)
    expect(harnessVisible('fake', new Set(['fake_harness']))).toBe(true)
    expect(harnessVisible('claude', new Set())).toBe(true)
  })
})
