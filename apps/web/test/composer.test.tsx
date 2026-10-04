import type { ReactElement } from 'react'
import { act, cleanup, fireEvent, render as rtlRender, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Attachment, Capabilities, PermissionMode } from '@beton/sdk'
import { Composer, pickerOptions, slashEntries } from '@/components/composer'
import { settingsOptions, YOLO_BLOCKED, type SettingsPatch } from '@/components/settings-picker'

// Server-Antworten für Anhänge, `@`-Suche und Skills (WEB-006).
const api = vi.hoisted(() => ({
  upload: vi.fn(),
  search: vi.fn(),
  skills: vi.fn(),
}))
vi.mock('@/lib/client', () => ({
  client: {
    info: () => Promise.resolve({ attachments: { max_file_bytes: 20 * 1024 * 1024, max_files: 10 } }),
    session: () => ({
      uploadAttachment: api.upload,
      skills: api.skills,
      workspace: { search: api.search },
    }),
  },
}))

function render(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return rtlRender(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

const base = {
  mode: 'native',
  transport: 'in_proc',
  auth_sources: ['none'],
  approval: 'native_request',
  tool_call_gate: 'full',
  model_switch: 'live',
  effort_switch: 'none',
  resume: 'warm',
  fork_history: 'preamble',
  interrupt: true,
  steering: true,
  subagents: 'none',
  usage_reporting: 'tokens_and_cost',
  compaction: 'native',
  instructions_delivery: 'first_message_prefix',
  mcp_injection: false,
  images: false,
  transcript_import: false,
} as unknown as Capabilities

function renderComposer(caps: Capabilities, sessionId = 'ses_a') {
  return render(
    <Composer
      sessionId={sessionId}
      harness="fake"
      capabilities={caps}
      running={false}
      onSend={() => undefined}
      onInterrupt={() => undefined}
      onSettings={() => undefined}
    />,
  )
}

beforeEach(() => {
  localStorage.clear()
  api.upload.mockReset()
  api.search.mockReset()
  api.skills.mockReset()
  api.search.mockResolvedValue({ items: [], next_cursor: null, total: 0 })
  vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => 'blob:vorschau', revokeObjectURL: () => undefined }))
})
afterEach(() => cleanup())

describe('Composer', () => {
  it('WEB-004 AC1: Picker zeigen nur Optionen, die der Harness unterstützt', () => {
    const caps = { ...base, models: ['fake-small', 'fake-large'], efforts: ['low', 'high'], permission_modes: ['plan', 'default'] } as Capabilities
    expect(pickerOptions(caps)).toEqual({ models: ['fake-small', 'fake-large'], efforts: [] })
    expect(settingsOptions(caps)).toEqual({ models: ['fake-small', 'fake-large'], efforts: [], modes: ['plan', 'default'], modelSwitch: true })
    renderComposer(caps)
    fireEvent.click(screen.getByLabelText('Modell'))
    const menu = screen.getByRole('menu', { name: 'Modell und Effort' })
    expect(within(menu).getAllByRole('menuitemradio').map((o) => o.textContent)).toEqual(['fake-small', 'fake-large'])
    expect(screen.queryByLabelText('Effort')).toBeNull()
    fireEvent.click(screen.getByLabelText('Permission-Mode'))
    const modes = screen.getByRole('menu', { name: 'Permission-Mode' })
    expect(within(modes).getAllByRole('menuitemradio').map((o) => o.querySelector('.font-medium')?.textContent)).toEqual([
      'Nur planen',
      'Fragen bei Schreibzugriff',
    ])
    cleanup()
    renderComposer({ ...caps, effort_switch: 'live' } as Capabilities)
    fireEvent.click(screen.getByLabelText('Effort'))
    const effort = screen.getByRole('group', { name: 'Effort' })
    expect(within(effort).getAllByRole('menuitemradio').map((o) => o.textContent)).toEqual(['niedrig', 'hoch'])
    cleanup()
    renderComposer({ ...base } as Capabilities)
    expect(screen.queryByLabelText('Modell')).toBeNull()
    expect(screen.queryByLabelText('Permission-Mode')).toBeNull()
  })

  it('WEB-004 AC2, HAR-017: Modell- und Effort-Wechsel gehen als Änderung an den Server', () => {
    const patches: SettingsPatch[] = []
    const caps = { ...base, effort_switch: 'live', models: ['fake-small', 'fake-large'], efforts: ['low', 'medium', 'high'] } as Capabilities
    render(
      <Composer
        sessionId="ses_m"
        harness="claude"
        model="fake-small"
        effort="medium"
        capabilities={caps}
        running
        onSend={() => undefined}
        onInterrupt={() => undefined}
        onSettings={(p) => patches.push(p)}
      />,
    )
    expect(screen.getByLabelText('Effort').textContent).toContain('Effort: mittel')
    fireEvent.click(screen.getByLabelText('Modell'))
    const menu = screen.getByRole('menu', { name: 'Modell und Effort' })
    expect(within(menu).getByText('Modell · Claude Code')).toBeTruthy()
    expect(within(menu).getByText('aktuell')).toBeTruthy()
    // Während eines Turns: Hinweis, dass der Wechsel ab dem nächsten Turn gilt (HAR-017 AC4).
    expect(within(menu).getByText(/Ein Wechsel gilt ab dem nächsten Turn/)).toBeTruthy()
    fireEvent.click(within(menu).getByText('fake-large'))
    expect(patches).toEqual([{ model: 'fake-large' }])
    fireEvent.click(screen.getByLabelText('Effort'))
    fireEvent.click(within(screen.getByRole('group', { name: 'Effort' })).getByText('hoch'))
    expect(patches).toEqual([{ model: 'fake-large' }, { effort: 'high' }])
  })

  it('HAR-027 AC1: YOLO bleibt ohne Sandbox gesperrt, andere Modi wechseln', () => {
    const patches: SettingsPatch[] = []
    const all: PermissionMode[] = ['plan', 'default', 'accept_edits', 'yolo']
    render(
      <Composer
        sessionId="ses_y"
        harness="claude"
        capabilities={{ ...base, permission_modes: all } as Capabilities}
        running={false}
        onSend={() => undefined}
        onInterrupt={() => undefined}
        onSettings={(p) => patches.push(p)}
      />,
    )
    expect(screen.getByLabelText('Permission-Mode').textContent).toContain('Fragen bei Schreibzugriff')
    fireEvent.click(screen.getByLabelText('Permission-Mode'))
    const menu = screen.getByRole('menu', { name: 'Permission-Mode' })
    expect(within(menu).getByText('Wie viel darf der Agent ohne Rückfrage?')).toBeTruthy()
    const yolo = within(menu).getByText('YOLO – keine Rückfragen des Agents').closest('button')!
    expect(yolo.disabled).toBe(true)
    expect(within(menu).getByText(YOLO_BLOCKED)).toBeTruthy()
    fireEvent.click(yolo)
    expect(patches).toEqual([])
    expect(within(menu).getByText('Claude Code übernimmt den Modus sofort.')).toBeTruthy()
    fireEvent.click(within(menu).getByText('Nur planen'))
    expect(patches).toEqual([{ permission_mode: 'plan' }])
  })

  it('WEB-004 AC3: Der Entwurf je Session überlebt einen Reload', () => {
    renderComposer(base)
    fireEvent.change(screen.getByLabelText('Nachricht'), { target: { value: 'halb fertig' } })
    cleanup()
    renderComposer(base)
    expect((screen.getByLabelText('Nachricht') as HTMLTextAreaElement).value).toBe('halb fertig')
    cleanup()
    renderComposer(base, 'ses_b')
    expect((screen.getByLabelText('Nachricht') as HTMLTextAreaElement).value).toBe('')
  })

  it('Senden mit Enter, Zeilenumbruch mit Shift+Enter', () => {
    const sent: string[] = []
    render(<Composer sessionId="ses_c" harness="fake" running={false} onSend={(t) => sent.push(t)} onInterrupt={() => undefined} />)
    const box = screen.getByLabelText('Nachricht')
    fireEvent.change(box, { target: { value: 'Zeile' } })
    act(() => {
      fireEvent.keyDown(box, { key: 'Enter', shiftKey: true })
    })
    expect(sent).toEqual([])
    act(() => {
      fireEvent.keyDown(box, { key: 'Enter' })
    })
    expect(sent).toEqual(['Zeile'])
  })
})

const shot: Attachment = { blob: `sha256:${'a'.repeat(64)}`, name: 'Bildschirmfoto.png', mime: 'image/png', size: 412 * 1024 }

describe('Composer: Anhänge, @ und / (WEB-006)', () => {
  it('WEB-006 AC1: Ein eingefügter Screenshot erscheint als Vorschau und geht mit der Eingabe', async () => {
    api.upload.mockResolvedValue(shot)
    const sent: Array<[string, Attachment[]]> = []
    render(<Composer sessionId="ses_p" harness="claude" running={false} onSend={(t, a) => sent.push([t, a])} onInterrupt={() => undefined} />)
    const box = screen.getByLabelText('Nachricht')
    fireEvent.change(box, { target: { value: 'Was zeigt das?' } })
    const file = new File([new Uint8Array(412 * 1024)], 'image.png', { type: 'image/png' })
    fireEvent.paste(box, { clipboardData: { files: [file], getData: () => '' } })
    // Vorschau sofort, Upload in den Blob-Store der Session.
    expect(await screen.findByAltText(/^Vorschau Bildschirmfoto/)).toBeTruthy()
    expect(screen.getByText(/Bildschirmfoto · 412 KB/)).toBeTruthy()
    await waitFor(() => expect(screen.getByTestId('attachment').dataset.state).toBe('ready'))
    expect(api.upload).toHaveBeenCalledWith(file, expect.stringMatching(/^Bildschirmfoto .*\.png$/), 'image/png')
    fireEvent.click(screen.getByRole('button', { name: 'Senden' }))
    expect(sent).toEqual([['Was zeigt das?', [shot]]])
    expect(screen.queryByTestId('attachment')).toBeNull()
  })

  it('WEB-006: Zu große und fremde Dateien werden mit Grund abgelehnt', () => {
    render(<Composer sessionId="ses_q" harness="claude" running={false} onSend={() => undefined} onInterrupt={() => undefined} />)
    const input = screen.getByTestId('attach-input')
    const big = new File(['x'], 'login-trace.har', { type: 'text/plain' })
    Object.defineProperty(big, 'size', { value: 48 * 1024 * 1024 })
    const zip = new File(['PK'], 'a.zip', { type: 'application/zip' })
    fireEvent.change(input, { target: { files: [big, zip] } })
    const alerts = screen.getAllByRole('alert').map((a) => a.textContent)
    expect(alerts).toEqual([
      'login-trace.har nicht angehängt: 48 MB, erlaubt sind 20 MB pro Datei. Kürze die Datei oder hänge einen Ausschnitt an.',
      'a.zip nicht angehängt: Erlaubt sind Bilder (PNG, JPEG, GIF, WebP), PDF und Textdateien.',
    ])
    expect(api.upload).not.toHaveBeenCalled()
  })

  it('WEB-006 AC2: @mid listet Workspace-Dateien und fügt die Referenz ein', async () => {
    api.search.mockImplementation((q: string) =>
      Promise.resolve({ items: q ? [{ path: 'src/middleware.rs' }, { path: 'docs/mid.md' }] : [], next_cursor: null, total: 20000 }),
    )
    render(<Composer sessionId="ses_m" harness="claude" running={false} onSend={() => undefined} onInterrupt={() => undefined} />)
    const box = screen.getByLabelText('Nachricht') as HTMLTextAreaElement
    fireEvent.focus(box)
    const text = 'Wende das Muster aus @mid'
    fireEvent.change(box, { target: { value: text, selectionStart: text.length } })
    const menu = await screen.findByRole('listbox', { name: 'Dateien im Workspace' })
    await waitFor(() => expect(menu.textContent).toContain('src/middleware.rs'))
    expect(menu.textContent).toContain('2 von 20.000')
    expect(api.search).toHaveBeenLastCalledWith('mid', 'fuzzy', { limit: 8 })
    // Treffer hervorgehoben, erster aktiv.
    expect(screen.getAllByRole('option')[0]!.getAttribute('aria-selected')).toBe('true')
    expect(menu.querySelector('mark')?.textContent).toBe('mid')
    fireEvent.keyDown(box, { key: 'Enter' })
    expect(box.value).toBe('Wende das Muster aus @src/middleware.rs ')
    expect(screen.queryByRole('listbox')).toBeNull()
  })

  it('WEB-006 AC3: Das Slash-Menü zeigt Skills des aktiven Agents mit Beschreibung', async () => {
    api.skills.mockResolvedValue({
      agent: 'implementer',
      items: [
        { name: 'review', description: 'Prüft die Änderungen wie ein Reviewer', origin: 'agent' },
        { name: 'changelog', description: 'Schreibt einen CHANGELOG-Eintrag', origin: 'project' },
      ],
      next_cursor: null,
    })
    const forked: string[] = []
    render(
      <Composer
        sessionId="ses_s"
        harness="claude"
        running={false}
        onSend={() => undefined}
        onInterrupt={() => undefined}
        commands={[{ name: 'fork', title: 'Forken', description: 'Ab hier abzweigen', run: () => forked.push('fork') }]}
      />,
    )
    const box = screen.getByLabelText('Nachricht') as HTMLTextAreaElement
    fireEvent.focus(box)
    fireEvent.change(box, { target: { value: '/', selectionStart: 1 } })
    const menu = await screen.findByRole('listbox', { name: 'Befehle und Skills' })
    await waitFor(() => expect(menu.textContent).toContain('Skills des Agents „implementer“'))
    expect(menu.textContent).toContain('review')
    expect(menu.textContent).toContain('Prüft die Änderungen wie ein Reviewer')
    expect(menu.textContent).toContain('/fork')
    fireEvent.change(box, { target: { value: '/rev', selectionStart: 4 } })
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(1))
    fireEvent.keyDown(box, { key: 'Enter' })
    expect(box.value).toBe('/review ')
    // beton-Befehle führen direkt aus.
    fireEvent.change(box, { target: { value: '/fo', selectionStart: 3 } })
    fireEvent.keyDown(box, { key: 'Enter' })
    expect(forked).toEqual(['fork'])
    expect(box.value).toBe('')
  })

  it('WEB-006: Slash-Einträge nach Präfix, beton vor Skills', () => {
    const e = slashEntries('c', [{ name: 'compact', title: 'Kontext komprimieren', description: '', run: () => undefined }], [
      { name: 'review-code', description: '' },
      { name: 'changelog', description: '' },
    ])
    expect(e.map((x) => `${x.group}:${x.name}`)).toEqual(['beton:compact', 'skill:changelog', 'skill:review-code'])
  })
})
