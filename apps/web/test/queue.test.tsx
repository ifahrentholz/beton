import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { QueueItem } from '@beton/sdk'
import { QueueList } from '@/components/queue'
import { client } from '@/lib/client'
import { moved, queueOf } from '@/lib/queue'
import { ev, resetSeq } from './fixtures'

const item = (id: string, text: string, author = 'usr_local'): QueueItem => ({
  id,
  author: author as QueueItem['author'],
  text,
  attachments: [],
  created_at: '2026-10-04T00:00:00Z',
})

const items = [item('inp_a', 'erst das'), item('inp_b', 'dann das'), item('inp_c', 'zuletzt')]

let calls: Array<{ method: string; url: string; body: unknown }> = []

beforeEach(() => {
  calls = []
  vi.spyOn(client, 'request').mockImplementation(async (method: string, url: string, body?: unknown) => {
    calls.push({ method, url, body })
    return undefined as never
  })
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function renderQueue(props: Partial<Parameters<typeof QueueList>[0]> = {}) {
  return render(
    <QueueList sessionId="ses_q" items={items} harness="claude" steering running onError={() => undefined} {...props} />,
  )
}

describe('Queue-UI', () => {
  it('zeigt die serverseitige Queue aus dem letzten queue.updated', () => {
    resetSeq()
    const log = [
      ev('queue.updated', { items: [items[0]], paused: false }),
      ev('turn.started', {}),
      ev('queue.updated', { items, paused: true }),
    ]
    expect(queueOf(log)).toEqual({ items, paused: true })
    expect(queueOf([])).toEqual({ items: [], paused: false })
  })

  it('WEB-005 AC1: Drag & Drop schickt die neue Position an den Server', async () => {
    renderQueue()
    const rows = screen.getAllByTestId('queue-row')
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining('Eingereiht 1erst das'),
      expect.stringContaining('Eingereiht 2dann das'),
      expect.stringContaining('Eingereiht 3zuletzt'),
    ])
    fireEvent.dragStart(rows[2]!, { dataTransfer: { setData: () => undefined, effectAllowed: '' } })
    fireEvent.dragOver(rows[0]!, { dataTransfer: { dropEffect: '' } })
    fireEvent.drop(rows[0]!, { dataTransfer: {} })
    // Sofort in der neuen Reihenfolge, bis der Server-Stand (queue.updated) eintrifft.
    expect(screen.getAllByTestId('queue-row')[0]!.textContent).toContain('zuletzt')
    await vi.waitFor(() => expect(calls).toHaveLength(1))
    expect(calls[0]).toEqual({
      method: 'POST',
      url: expect.stringContaining('/v1/sessions/ses_q/queue/inp_c/move'),
      body: { position: 0 },
    })
    expect(moved(items, 'inp_a', 5).map((i) => i.id)).toEqual(['inp_b', 'inp_c', 'inp_a'])
  })

  it('WEB-005 AC2: „Als Steer senden“ ist ohne Steering deaktiviert und erklärt warum', () => {
    renderQueue({ steering: false })
    const steer = screen.getAllByRole('button', { name: 'Jetzt lenken' })[0]!
    expect(steer).toHaveProperty('disabled', true)
    const hint = steer.parentElement!.getAttribute('title')
    expect(hint).toContain('Claude Code kann nicht in einen laufenden Turn eingreifen')
    expect(steer.getAttribute('aria-describedby')).toBeTruthy()
    cleanup()
    renderQueue({ steering: true })
    const enabled = screen.getAllByRole('button', { name: 'Jetzt lenken' })[0]!
    expect(enabled).toHaveProperty('disabled', false)
    expect(enabled.parentElement!.getAttribute('title')).toBeNull()
    fireEvent.click(enabled)
    expect(calls[0]?.url).toContain('/queue/inp_a/steer')
  })

  it('bearbeitet und entfernt Einträge; Autoren erst bei mehreren Personen', async () => {
    renderQueue({ items: [item('inp_a', 'meins'), item('inp_x', 'von Anna', 'usr_01ANNA')] })
    expect(screen.getByText('Du:')).toBeTruthy()
    fireEvent.click(screen.getAllByRole('button', { name: 'Bearbeiten' })[0]!)
    const box = screen.getByLabelText('Eingereihte Nachricht bearbeiten')
    fireEvent.change(box, { target: { value: 'geändert' } })
    fireEvent.keyDown(box, { key: 'Enter' })
    fireEvent.click(screen.getAllByRole('button', { name: 'Entfernen' })[1]!)
    await vi.waitFor(() => expect(calls).toHaveLength(2))
    expect(calls[0]).toMatchObject({ method: 'PATCH', body: { text: 'geändert' } })
    expect(calls[1]).toMatchObject({ method: 'DELETE', url: expect.stringContaining('/queue/inp_x') })
    cleanup()
    renderQueue({ items: [item('inp_a', 'meins')] })
    expect(screen.queryByText('Du:')).toBeNull()
  })
})
