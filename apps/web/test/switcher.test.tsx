import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { SessionSummary } from '@beton/sdk'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ list: vi.fn(), navigate: vi.fn() }))
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => api.navigate, useRouterState: () => '/s/ses_b' }))
vi.mock('@/lib/client', () => ({ client: { inbox: { list: api.list } } }))

import { SessionSwitcher, switcherRows, waitingOf } from '@/components/session-switcher'
import { useUi } from '@/components/ui-state'
import { mruOrder, reloadMru, useMru } from '@/lib/mru'
import { useSessions } from '@/store/sessions'

function summary(id: string, title: string, extra: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id: id as SessionSummary['id'],
    title,
    status: 'idle',
    kind: 'interactive',
    harness: 'claude',
    archived: false,
    head_seq: 1,
    cost_micro: 0,
    created_at: '2026-10-01T10:00:00Z',
    last_activity_at: '2026-10-01T10:00:00Z',
    pinned: false,
    read_seq: 1,
    unread: false,
    changed_at: '2026-10-01T10:00:00Z',
    ...extra,
  }
}

beforeEach(() => {
  localStorage.removeItem('beton.mru.v1')
  reloadMru()
  api.list.mockReset()
  api.navigate.mockReset()
  useSessions.setState({ byId: {} })
})
afterEach(cleanup)

describe('Session-Switcher (UX-003)', () => {
  it('UX-003 AC2: Sessions mit offener Freigabe bzw. Frage sind in der Liste markiert', async () => {
    expect(
      waitingOf([
        { session_id: 'ses_a', kind: 'question', status: 'open' },
        { session_id: 'ses_a', kind: 'approval', status: 'open' },
        { session_id: 'ses_c', kind: 'question', status: 'open' },
        { session_id: 'ses_d', kind: 'approval', status: 'done' },
      ]),
    ).toEqual(
      new Map([
        ['ses_a', 'approval'],
        ['ses_c', 'question'],
      ]),
    )
    api.list.mockResolvedValue({
      items: [
        { session_id: 'ses_a', kind: 'approval', status: 'open' },
        { session_id: 'ses_c', kind: 'question', status: 'open' },
      ],
      open_count: 2,
    })
    useSessions.setState({
      byId: {
        ses_a: summary('ses_a', 'Rate-Limiter'),
        ses_b: summary('ses_b', 'Review'),
        ses_c: summary('ses_c', 'Flaky Test'),
        ses_d: summary('ses_d', 'Ohne Inbox', { status: 'waiting_approval' }),
      },
    })
    act(() => useUi.setState({ switcher: 'open' }))
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <SessionSwitcher />
      </QueryClientProvider>,
    )
    const row = (id: string) => document.querySelector(`[data-session="${id}"]`)!
    await waitFor(() => expect(row('ses_a').getAttribute('data-waiting')).toBe('approval'))
    expect(row('ses_a').textContent).toContain('Freigabe offen')
    expect(row('ses_c').textContent).toContain('Frage offen')
    // Ohne Inbox-Eintrag zählt der Status der Session.
    expect(row('ses_d').textContent).toContain('Freigabe offen')
    expect(row('ses_b').getAttribute('data-waiting')).toBeNull()
  })

  it('UX-003 AC3: die MRU-Reihenfolge gilt je Gerät und überlebt Neustarts', () => {
    useMru.getState().touch('ses_a')
    useMru.getState().touch('ses_b')
    useMru.getState().touch('ses_a')
    expect(useMru.getState().ids).toEqual(['ses_a', 'ses_b'])
    // „Neustart“: Stand frisch aus dem localStorage.
    useMru.setState({ ids: [] })
    reloadMru()
    expect(useMru.getState().ids).toEqual(['ses_a', 'ses_b'])
    const list = [summary('ses_c', 'C', { last_activity_at: '2026-10-02T00:00:00Z' }), summary('ses_b', 'B'), summary('ses_a', 'A')]
    expect(mruOrder(list, useMru.getState().ids).map((s) => s.id)).toEqual(['ses_a', 'ses_b', 'ses_c'])
    expect(switcherRows(list, ['ses_a'], 'B').map((s) => s.id)).toEqual(['ses_b'])
  })

  it('UX-003: Tastatur – vorausgewählt ist die zuvor genutzte Session, Enter wechselt', async () => {
    api.list.mockResolvedValue({ items: [], open_count: 0 })
    useMru.getState().touch('ses_a')
    useMru.getState().touch('ses_b')
    useSessions.setState({ byId: { ses_a: summary('ses_a', 'Erste'), ses_b: summary('ses_b', 'Aktuelle') } })
    act(() => useUi.setState({ switcher: 'open' }))
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <SessionSwitcher />
      </QueryClientProvider>,
    )
    const input = screen.getByRole('combobox')
    const selected = () => document.querySelector('[role="option"][aria-selected="true"]')!.getAttribute('data-session')
    expect(selected()).toBe('ses_a')
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(selected()).toBe('ses_b')
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(api.navigate).toHaveBeenCalledWith({ to: '/s/$sessionId', params: { sessionId: 'ses_a' } })
    expect(useUi.getState().switcher).toBe(false)
  })
})
