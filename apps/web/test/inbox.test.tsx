import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { InboxItemView } from '@beton/sdk'

const api = vi.hoisted(() => ({ list: vi.fn(), resolve: vi.fn() }))
vi.mock('@/lib/client', () => ({ client: { inbox: { list: api.list, resolve: api.resolve } } }))
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => vi.fn() }))

import { InboxView } from '@/components/inbox'

function item(id: string, title: string, created: string): InboxItemView {
  return {
    id,
    session_id: 'ses_1',
    session_title: title,
    kind: 'approval',
    status: 'open',
    approval_id: `apr_${id}`,
    title: 'Shell-Kommando bestätigen',
    summary: `git push ${id}`,
    detail: { approval_kind: 'policy', subject: { summary: `git push ${id}`, tool: { name: 'Bash', kind: 'shell', args: {} } }, options: ['approve', 'deny'] },
    created_seq: 1,
    created_at: created,
  }
}

beforeEach(() => {
  api.list.mockReset()
  api.resolve.mockReset()
  api.resolve.mockResolvedValue(undefined)
})
afterEach(cleanup)

const press = (k: string) =>
  act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
  })

describe('Inbox (UX-001)', () => {
  it('UX-001: die Auswahl folgt dem Eintrag, nicht der Position (Security-Review M2)', async () => {
    const a = item('a', 'Gesehen', '2026-10-14T10:00:00Z')
    const b = item('b', 'Neu', '2026-10-14T10:05:00Z')
    api.list.mockResolvedValue({ items: [a], next_cursor: null, open_count: 1 })
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <InboxView />
      </QueryClientProvider>,
    )
    await screen.findByText('git push a')
    // Ein neuer Eintrag erscheint oben; ausgewählt bleibt der gesehene.
    api.list.mockResolvedValue({ items: [b, a], next_cursor: null, open_count: 2 })
    await act(() => qc.invalidateQueries({ queryKey: ['inbox'] }))
    await screen.findByText('git push b')
    press('a')
    await waitFor(() => expect(api.resolve).toHaveBeenCalledTimes(1))
    expect(api.resolve).toHaveBeenCalledWith('a', { decision: 'allow' })
  })

  it('UX-001: verschwindet der ausgewählte Eintrag, entscheidet a nichts', async () => {
    const a = item('a', 'Gesehen', '2026-10-14T10:00:00Z')
    const b = item('b', 'Neu', '2026-10-14T10:05:00Z')
    api.list.mockResolvedValue({ items: [a], next_cursor: null, open_count: 1 })
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <InboxView />
      </QueryClientProvider>,
    )
    await screen.findByText('git push a')
    api.list.mockResolvedValue({ items: [b], next_cursor: null, open_count: 1 })
    await act(() => qc.invalidateQueries({ queryKey: ['inbox'] }))
    await screen.findByText('git push b')
    press('a')
    expect(api.resolve).not.toHaveBeenCalled()
  })
})
