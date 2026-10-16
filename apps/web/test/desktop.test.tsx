import type { ReactElement } from 'react'
import { act, cleanup, fireEvent, render as rtlRender, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { NotificationSettings } from '@beton/sdk'

// Server-Antworten für Anhänge und Einstellungen.
const api = vi.hoisted(() => ({ upload: vi.fn(), get: vi.fn(), put: vi.fn() }))
vi.mock('@/lib/client', () => ({
  client: {
    info: () => Promise.resolve({ attachments: { max_file_bytes: 20 * 1024 * 1024, max_files: 10 } }),
    session: () => ({ uploadAttachment: api.upload, skills: () => Promise.resolve({ items: [] }), workspace: { search: () => Promise.resolve({ items: [] }) } }),
    settings: { notifications: { get: api.get, put: api.put } },
  },
}))
vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to, ...rest }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...rest}>
      {children}
    </a>
  ),
}))

import { Composer } from '@/components/composer'
import { OpenedViaLink } from '@/components/session-view'
import { NotificationSettingsView } from '@/components/settings/notifications'
import { attachFiles, WindowDrop } from '@/components/window-drop'
import { indexForSeq, linkOf, validateSessionSearch } from '@/lib/deeplink'

function render(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return rtlRender(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

const ch = (desktop: boolean, sound: boolean, badge: boolean) => ({ desktop, sound, badge })
const defaults: NotificationSettings = {
  events: {
    approval_requested: ch(true, true, true),
    question: ch(true, false, true),
    turn_completed: ch(false, false, false),
    async_done: ch(true, false, true),
    mention: ch(true, false, true),
    runner_lost: ch(true, false, false),
    budget_threshold: ch(true, false, false),
    rate_limit_high: ch(true, false, false),
  },
  quiet_hours: { enabled: false, start: '22:00', end: '07:00', allow_approvals: true },
  preview: true,
}

beforeEach(() => {
  api.upload.mockReset()
  api.get.mockReset()
  api.put.mockReset()
  api.put.mockImplementation((s: NotificationSettings) => Promise.resolve(s))
  vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => 'blob:vorschau', revokeObjectURL: () => undefined }))
})
afterEach(() => cleanup())

describe('Benachrichtigungs-Einstellungen (UX-006)', () => {
  it('UX-006 AC3: zeigt den Stand vom Server, wie ihn ein anderes Gerät gespeichert hat', async () => {
    api.get.mockResolvedValue({ ...defaults, events: { ...defaults.events, turn_completed: ch(true, false, false) } })
    render(<NotificationSettingsView />)
    const box = (await screen.findByLabelText('Agent ist fertig: Desktop-Mitteilung')) as HTMLInputElement
    expect(box.checked).toBe(true)
    expect((screen.getByLabelText('Freigabe angefordert: Inbox (immer an)') as HTMLInputElement).disabled).toBe(true)
    expect((screen.getByLabelText('Freigabe angefordert: Web-Push') as HTMLInputElement).disabled).toBe(true)
  })

  it('UX-006 AC1: Desktop-Kanal für „Agent ist fertig“ wird je Klick gespeichert', async () => {
    api.get.mockResolvedValue(defaults)
    render(<NotificationSettingsView />)
    const box = (await screen.findByLabelText('Agent ist fertig: Desktop-Mitteilung')) as HTMLInputElement
    expect(box.checked).toBe(false)
    fireEvent.click(box)
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1))
    const saved = api.put.mock.calls[0]![0] as NotificationSettings
    expect(saved.events.turn_completed).toEqual(ch(true, false, false))
    expect(saved.events.approval_requested).toEqual(defaults.events.approval_requested)
  })

  it('UX-006 AC2: Ruhezeiten mit Zeitraum und „Freigaben trotzdem melden“', async () => {
    api.get.mockResolvedValue(defaults)
    render(<NotificationSettingsView />)
    expect(screen.queryByLabelText('Beginn der Ruhezeiten')).toBeNull()
    fireEvent.click(await screen.findByRole('switch', { name: 'Ruhezeiten' }))
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1))
    expect((api.put.mock.calls[0]![0] as NotificationSettings).quiet_hours.enabled).toBe(true)
    expect(((await screen.findByLabelText('Beginn der Ruhezeiten')) as HTMLInputElement).value).toBe('22:00')
    const approvals = screen.getByRole('switch', { name: 'Freigaben trotzdem melden' })
    expect(approvals.getAttribute('aria-checked')).toBe('true')
    fireEvent.click(approvals)
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(2))
    expect((api.put.mock.calls[1]![0] as NotificationSettings).quiet_hours).toEqual({ ...defaults.quiet_hours, enabled: true, allow_approvals: false })
  })
})

describe('Deep-Links (DESK-006)', () => {
  it('DESK-006 AC3: `?seq=N` zielt auf das Element, das Event N zeigt', () => {
    expect(validateSessionSearch({ seq: 214, opened: 'link' })).toMatchObject({ seq: 214, opened: 'link' })
    expect(validateSessionSearch({ seq: '7' }).seq).toBe(7)
    expect(validateSessionSearch({ seq: -1, opened: 'evil', comment: '<x>' })).toMatchObject({ seq: undefined, opened: undefined, comment: undefined })
    const items = [{ key: 'e3' }, { key: 'e5' }, { key: 'e5-1' }, { key: 'e9' }]
    expect(indexForSeq(items, 9)).toBe(3)
    // Ein Tool-Call fasst requested (5) bis completed (8) zusammen.
    expect(indexForSeq(items, 7)).toBe(2)
    expect(indexForSeq(items, 2)).toBeUndefined()
  })

  it('DESK-006 AC1: Hinweis „Geöffnet über beton://…“ nennt Link und Event', () => {
    rtlRender(<OpenedViaLink sessionId="ses_7f3k" search={{ seq: 214, opened: 'link' }} />)
    expect(screen.getByText('beton://local/s/ses_7f3k?seq=214')).toBeTruthy()
    expect(screen.getByText('· Event 214')).toBeTruthy()
    expect(linkOf('ses_x', {})).toBe('beton://local/s/ses_x')
  })
})

describe('Datei aufs Fenster ziehen (DESK-001)', () => {
  it('DESK-001 AC3: Eine auf das Fenster gezogene Datei landet als Anhang im Composer der aktiven Session', async () => {
    api.upload.mockResolvedValue({ blob: 'sha256:' + 'a'.repeat(64), name: 'login-fehler.png', mime: 'image/png', size: 3 })
    render(
      <WindowDrop sessionId="ses_7f3k" title="Rate-Limiter für die Login-API">
        <div data-testid="stream" />
        <Composer sessionId="ses_7f3k" harness="claude" running={false} onSend={() => undefined} onInterrupt={() => undefined} />
      </WindowDrop>,
    )
    const file = new File([new Uint8Array(3)], 'login-fehler.png', { type: 'image/png' })
    const stream = screen.getByTestId('stream')
    fireEvent.dragEnter(stream, { dataTransfer: { types: ['Files'], items: [{ kind: 'file' }], files: [] } })
    expect(screen.getByText('Loslassen zum Anhängen')).toBeTruthy()
    expect(screen.getByText(/landet als Anhang im Composer von „Rate-Limiter für die Login-API“/)).toBeTruthy()
    fireEvent.drop(screen.getByTestId('window-drop'), { dataTransfer: { types: ['Files'], items: [{ kind: 'file' }], files: [file] } })
    expect(screen.queryByTestId('window-drop')).toBeNull()
    await waitFor(() => expect(screen.getByTestId('attachment').dataset.state).toBe('ready'))
    expect(api.upload).toHaveBeenCalledWith(file, 'login-fehler.png', 'image/png')
  })

  it('DESK-001 AC3: der Drop geht nur an den Composer seiner Session', () => {
    render(
      <WindowDrop sessionId="ses_a" title="A">
        <Composer sessionId="ses_b" harness="claude" running={false} onSend={() => undefined} onInterrupt={() => undefined} />
      </WindowDrop>,
    )
    const file = new File(['x'], 'notiz.txt', { type: 'text/plain' })
    act(() => attachFiles('ses_a', [file]))
    expect(api.upload).not.toHaveBeenCalled()
    expect(screen.queryByTestId('attachment')).toBeNull()
  })
})
