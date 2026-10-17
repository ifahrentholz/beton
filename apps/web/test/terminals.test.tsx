import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { openTerminals, runningFor, sessionTerminals } from '@/lib/terminals'
import { useTheme } from '@/lib/theme'
import { ev, resetSeq } from './fixtures'

/** Ein Kanal wie aus dem SDK, gesteuert vom Test. */
const channel = {
  opts: undefined as undefined | Record<string, (...a: never[]) => void>,
  sent: [] as string[],
  consumed: 0,
  closed: false,
}
const resizes: Array<[string, number, number]> = []
const opened: unknown[] = []
/** Blobs der Session (`terminal.snapshot`) und ein Fehler für das nächste Öffnen. */
const blobs: Record<string, string> = {}
const fail: { open?: unknown } = {}

vi.mock('@/lib/client', () => ({
  client: {
    session: () => ({
      terminals: {
        attach: (_id: string, opts: Record<string, (...a: never[]) => void>) => {
          channel.opts = opts
          return {
            send: (d: string) => channel.sent.push(d),
            consumed: (n: number) => (channel.consumed += n),
            close: () => (channel.closed = true),
          }
        },
        resize: (id: string, cols: number, rows: number) => {
          resizes.push([id, cols, rows])
          return Promise.resolve({})
        },
        open: (req: unknown) => {
          opened.push(req)
          if (fail.open) return Promise.reject(fail.open)
          return Promise.resolve({ terminal_id: 'term_neu' })
        },
        close: () => Promise.resolve(),
        rename: () => Promise.resolve({}),
      },
      blobData: (ref: string) =>
        ref in blobs ? Promise.resolve({ arrayBuffer: () => Promise.resolve(new TextEncoder().encode(blobs[ref]).buffer) }) : Promise.reject(new Error('404')),
    }),
  },
}))

/** Fake-xterm: zeichnet Schreiben, Reset und Eingaben auf. */
function fakeXterm() {
  const state = {
    written: [] as string[],
    resets: 0,
    disableStdin: false,
    themes: [] as Record<string, string>[],
    created: [] as unknown[],
    onData: undefined as undefined | ((d: string) => void),
    onResize: undefined as undefined | ((s: { cols: number; rows: number }) => void),
  }
  const load = () =>
    Promise.resolve({
      createTerminal: (_el: HTMLElement, opts: unknown) => {
        state.created.push(opts)
        return {
          term: {
            options: {
              set disableStdin(v: boolean) {
                state.disableStdin = v
              },
              set theme(v: Record<string, string>) {
                state.themes.push(v)
              },
            },
            reset: () => (state.resets += 1),
            write: (b: Uint8Array | string, cb?: () => void) => {
              state.written.push(typeof b === 'string' ? b : new TextDecoder().decode(b))
              cb?.()
            },
            onData: (f: (d: string) => void) => (state.onData = f),
            onResize: (f: (s: { cols: number; rows: number }) => void) => (state.onResize = f),
          },
          fit: () => {},
          dispose: () => {},
        }
      },
    }) as unknown as Promise<typeof import('@/components/workspace/xterm-impl')>
  return { state, load }
}

class NoopObserver {
  observe() {}
  disconnect() {}
}
globalThis.ResizeObserver = NoopObserver as unknown as typeof ResizeObserver

afterEach(() => {
  cleanup()
  channel.opts = undefined
  channel.sent = []
  channel.consumed = 0
  resizes.length = 0
  opened.length = 0
  delete fail.open
  for (const k of Object.keys(blobs)) delete blobs[k]
  localStorage.removeItem('beton.terminals.hidden.ses_test')
})

const opened1 = (extra: Record<string, unknown> = {}) =>
  ev('terminal.opened', { terminal_id: 'term_1', channel_id: 1, kind: 'user', cmd: '/bin/zsh', cols: 120, rows: 32, name: 'zsh', ...extra })

describe('Terminals', () => {
  it('SES-019: laufende Terminals aus dem Log, mit Umbenennen, Ende und neuem Runner', () => {
    resetSeq()
    const events = [
      opened1(),
      ev('terminal.opened', { terminal_id: 'term_2', channel_id: 2, kind: 'tool', cmd: 'npm run dev', cols: 80, rows: 24, name: 'Dev-Server' }),
      ev('terminal.renamed', { terminal_id: 'term_1', name: 'Build' }),
    ]
    expect(openTerminals(events).map((t) => [t.id, t.name, t.byAgent])).toEqual([
      ['term_1', 'Build', false],
      ['term_2', 'Dev-Server', true],
    ])
    expect(openTerminals([...events, ev('terminal.closed', { terminal_id: 'term_1', exit_code: 0 })]).map((t) => t.id)).toEqual(['term_2'])
    expect(openTerminals([...events, ev('session.started', {})])).toEqual([])
    expect(openTerminals(events, true)).toEqual([])
    expect(runningFor('2026-10-04T00:00:00Z', Date.parse('2026-10-04T00:14:30Z'))).toBe('läuft seit 14 Min.')
  })

  it('WEB-010: Keyframe setzt den Bildschirm neu, Kredit erst nach dem Rendern, Eingaben gehen zum PTY', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    const started = ev('session.started', { sandbox: { tool_sandbox: true, egress_proxy: true } })
    const { rerender } = render(<TerminalsPanel sessionId="ses_test" events={[started, opened1()]} load={x.load} />)
    expect(screen.getByRole('tab', { name: /zsh/ }).getAttribute('aria-selected')).toBe('true')
    expect(screen.getByText(/Sandbox: Profil „Tools“/)).toBeTruthy()
    // `backend: none` steht nie als Sandbox da.
    const none = ev('session.started', { sandbox: { tool_sandbox: false, egress_proxy: false } })
    rerender(<TerminalsPanel sessionId="ses_test" events={[none, opened1()]} load={x.load} />)
    expect(screen.getByText(/Ohne Sandbox/)).toBeTruthy()
    rerender(<TerminalsPanel sessionId="ses_test" events={[started, opened1()]} load={x.load} />)
    await vi.waitFor(() => expect(channel.opts).toBeDefined())
    act(() => channel.opts!.onOpen!('drive' as never))
    act(() => channel.opts!.onData!(new TextEncoder().encode('\x1bcBILD') as never, true as never))
    act(() => channel.opts!.onData!(new TextEncoder().encode('$ ') as never, false as never))
    expect(x.state.resets).toBe(1)
    expect(x.state.written).toEqual(['\x1bcBILD', '$ '])
    expect(channel.consumed).toBe(8)
    x.state.onData!('ls\r')
    expect(channel.sent).toEqual(['ls\r'])
    expect(screen.queryByText(/nur lesend/)).toBeNull()
  })

  it('WEB-010 AC2: eine neue Fenstergröße geht an das PTY', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    render(<TerminalsPanel sessionId="ses_test" events={[opened1()]} load={x.load} />)
    await vi.waitFor(() => expect(channel.opts).toBeDefined())
    act(() => channel.opts!.onOpen!('drive' as never))
    act(() => {
      x.state.onResize!({ cols: 90, rows: 40 })
      x.state.onResize!({ cols: 100, rows: 30 })
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200)
    })
    // Entprellt: nur die letzte Größe.
    expect(resizes).toEqual([['term_1', 100, 30]])
    expect(screen.getByTestId('terminal-size').textContent).toBe('100 × 30')
    vi.useRealTimers()
  })

  it('WEB-010 AC3: Nur-Lese-Rollen sehen den Output, Eingaben sind deaktiviert', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    render(<TerminalsPanel sessionId="ses_test" events={[opened1()]} load={x.load} />)
    await vi.waitFor(() => expect(channel.opts).toBeDefined())
    act(() => channel.opts!.onOpen!('read' as never))
    act(() => channel.opts!.onData!(new TextEncoder().encode('Ausgabe') as never, true as never))
    expect(x.state.written).toEqual(['Ausgabe'])
    expect(x.state.disableStdin).toBe(true)
    x.state.onData!('rm -rf ~\r')
    expect(channel.sent).toEqual([])
    expect(screen.getByText(/nur lesend/)).toBeTruthy()
    // Keine Steuerung: kein neues Terminal, kein Schließen, kein Umbenennen, keine Größe.
    expect(screen.queryByLabelText('Neues Terminal')).toBeNull()
    expect(screen.queryByLabelText('zsh schließen')).toBeNull()
    expect(screen.queryByLabelText('Umbenennen')).toBeNull()
    act(() => x.state.onResize!({ cols: 50, rows: 20 }))
    await new Promise((r) => setTimeout(r, 200))
    expect(resizes).toEqual([])
  })

  it('SES-019: neues Terminal, Wieder verbunden nach Reconnect, vom Agent gestartet', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    const agent = ev('terminal.opened', { terminal_id: 'term_2', channel_id: 2, kind: 'tool', cmd: 'npm run dev', cols: 80, rows: 24, name: 'Dev-Server' })
    render(<TerminalsPanel sessionId="ses_test" events={[agent]} load={x.load} />)
    expect(screen.getByText(/Vom Agent gestartet/)).toBeTruthy()
    expect(screen.getByLabelText('läuft')).toBeTruthy()
    await vi.waitFor(() => expect(channel.opts).toBeDefined())
    act(() => channel.opts!.onOpen!('drive' as never))
    expect(screen.queryByRole('status')).toBeNull()
    act(() => channel.opts!.onOpen!('drive' as never))
    expect(screen.getByRole('status').textContent).toMatch(/Wieder verbunden/)
    fireEvent.click(screen.getByLabelText('Neues Terminal'))
    await vi.waitFor(() => expect(opened).toEqual([{ cols: 120, rows: 32 }]))
  })
  it('SES-019, DATA-011: beendete Terminals bleiben mit Ende, Exit-Code und letztem Snapshot', () => {
    resetSeq()
    const events = [
      opened1(),
      ev('terminal.snapshot', { terminal_id: 'term_1', blob_ref: 'blob_alt', cols: 120, rows: 32, cursor: {} }),
      ev('terminal.snapshot', { terminal_id: 'term_1', blob_ref: 'blob_neu', cols: 100, rows: 30, cursor: {} }),
      { ...ev('terminal.closed', { terminal_id: 'term_1', exit_code: 0 }), ts: '2026-10-04T12:21:00Z' },
      ev('terminal.opened', { terminal_id: 'term_2', channel_id: 2, kind: 'user', cmd: '/bin/zsh', cols: 80, rows: 24, name: 'Tests' }),
    ]
    const list = sessionTerminals(events)
    expect(list.map((t) => [t.id, !!t.ended])).toEqual([
      ['term_1', true],
      ['term_2', false],
    ])
    expect(list[0]!.ended).toEqual({ at: '2026-10-04T12:21:00Z', exitCode: 0 })
    expect(list[0]!.snapshot).toEqual({ ref: 'blob_neu', cols: 100, rows: 30 })
    // Ein neuer Runner bzw. ein beendeter Runner beendet die laufenden Terminals mit.
    expect(sessionTerminals([...events, ev('session.started', {})]).map((t) => t.ended)).toEqual([
      { at: '2026-10-04T12:21:00Z', exitCode: 0 },
      { withRunner: true },
    ])
    expect(sessionTerminals(events, true)[1]!.ended).toEqual({ withRunner: true })
    // `openTerminals` kennt weiter nur die laufenden.
    expect(openTerminals(events).map((t) => t.id)).toEqual(['term_2'])
  })

  it('SES-019: leere Rail zeigt „Noch kein Terminal“ mit Erklärung und Knopf', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    render(<TerminalsPanel sessionId="ses_test" events={[]} load={x.load} />)
    expect(screen.getByText('Noch kein Terminal in dieser Session')).toBeTruthy()
    expect(screen.getByText(/in derselben Sandbox wie die Tools des Agents/)).toBeTruthy()
    fireEvent.click(screen.getAllByRole('button', { name: 'Neues Terminal' }).at(-1)!)
    await vi.waitFor(() => expect(opened).toEqual([{ cols: 120, rows: 32 }]))
  })

  it('SBX-006: ohne Sandbox (backend: none) mit offenem Schloss und Hinweis, nie als Sandbox', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    const none = ev('session.started', { sandbox: { tool_sandbox: false, egress_proxy: false } })
    render(<TerminalsPanel sessionId="ses_test" events={[none, opened1()]} load={x.load} />)
    const warn = screen.getByTestId('terminal-no-sandbox')
    expect(warn.textContent).toContain('Ohne Sandbox (backend: none)')
    expect(warn.querySelector('svg')).not.toBeNull()
    expect(warn.className).toContain('text-deny')
    expect(screen.getByText('Die Shell sieht alles, was dein Benutzer sieht.')).toBeTruthy()
    expect(screen.queryByText(/Sandbox: Profil/)).toBeNull()
  })

  it('DATA-011 AC2: ein beendetes Terminal zeigt den letzten Bildschirm aus dem Snapshot; „×“ blendet es aus', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    blobs.blob_1 = '❯ pnpm vitest run\r\n Test Files  2 passed (2)\r\n'
    const closedAt = new Date(2026, 9, 4, 14, 21).toISOString()
    const events = [
      opened1({ name: 'Tests' }),
      ev('terminal.snapshot', { terminal_id: 'term_1', blob_ref: 'blob_1', cols: 100, rows: 30, cursor: {} }),
      { ...ev('terminal.closed', { terminal_id: 'term_1', exit_code: 0 }), ts: closedAt },
    ]
    render(<TerminalsPanel sessionId="ses_test" events={events} load={x.load} />)
    const tab = screen.getByRole('tab', { name: /Tests/ })
    expect(tab.className).toContain('italic')
    expect(within(tab).getByLabelText('beendet')).toBeTruthy()
    expect(within(tab).queryByLabelText('Umbenennen')).toBeNull()
    expect(screen.getByText('Beendet um 14:21 (Exit 0) · letzter Bildschirm, keine Eingabe mehr möglich')).toBeTruthy()
    // Der Emulator zeigt den Snapshot in seiner Größe, ohne Eingabe und ohne Kanal.
    await vi.waitFor(() => expect(x.state.written.join('')).toContain('Test Files  2 passed (2)'))
    expect(x.state.created[0]).toMatchObject({ readOnly: true, cols: 100, rows: 30 })
    expect(x.state.written.at(-1)).toBe('\x1b[?25l')
    expect(channel.opts).toBeUndefined()
    expect(screen.getByTestId('terminal-screen').className).toContain('opacity-70')
    // Ausblenden: lokal und dauerhaft für diese Session.
    fireEvent.click(screen.getByLabelText('Tests ausblenden'))
    expect(screen.queryByRole('tab')).toBeNull()
    expect(screen.getByText('Noch kein Terminal in dieser Session')).toBeTruthy()
    cleanup()
    render(<TerminalsPanel sessionId="ses_test" events={events} load={x.load} />)
    expect(screen.queryByRole('tab')).toBeNull()
  })

  it('DATA-011: ohne Snapshot steht da, dass nichts gespeichert ist', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    render(<TerminalsPanel sessionId="ses_test" events={[opened1(), ev('terminal.closed', { terminal_id: 'term_1' })]} load={x.load} />)
    expect(screen.getByText('Vom letzten Bildschirm ist nichts gespeichert.')).toBeTruthy()
    expect(x.state.created).toEqual([])
  })

  it('SBX-017: „Start fehlgeschlagen“ bei sandbox_unavailable nennt Grund und Weg zur Diagnose', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    fail.open = { problem: { code: 'sandbox_unavailable', detail: 'Profil „Tools“ ließ sich nicht anwenden' } }
    render(<TerminalsPanel sessionId="ses_test" events={[]} load={x.load} />)
    fireEvent.click(screen.getAllByRole('button', { name: 'Neues Terminal' }).at(-1)!)
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toContain('Terminal nicht gestartet')
    expect(alert.textContent).toContain('Die Sandbox ist nicht verfügbar (sandbox_unavailable: Profil „Tools“ ließ sich nicht anwenden).')
    expect(alert.textContent).toContain('Ohne Sandbox startet beton keine Shell. Prüfe die Sandbox mit beton doctor und versuche es erneut.')
    // Der Weg zurück: erneut versuchen über den Knopf darunter.
    expect(screen.getByText('Noch kein Terminal in dieser Session')).toBeTruthy()
    // Andere Fehler bleiben die Meldung des Servers.
    fail.open = { problem: { code: 'conflict', detail: 'Runner nicht verbunden' } }
    fireEvent.click(screen.getAllByRole('button', { name: 'Neues Terminal' }).at(-1)!)
    await vi.waitFor(() => expect(screen.getByRole('alert').textContent).toBe('Runner nicht verbunden'))
  })

  it('UX-005 AC1: ein Theme-Wechsel setzt die Farben des Terminals ohne Neuladen', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    render(<TerminalsPanel sessionId="ses_test" events={[opened1()]} load={x.load} />)
    await vi.waitFor(() => expect(x.state.created).toHaveLength(1))
    // Das Terminal bleibt dunkel (wie im Prototyp): Farben aus den Tokens seines `.dark`-Containers.
    const box = screen.getByTestId('terminal-screen')
    expect(box.classList.contains('dark')).toBe(true)
    box.style.setProperty('--sunken', '#2e3440')
    box.style.setProperty('--foreground', '#eceff4')
    expect(x.state.themes).toEqual([])
    act(() => useTheme.getState().update({ theme: 'nord' }))
    expect(x.state.themes).toHaveLength(1)
    expect(x.state.themes[0]).toMatchObject({ background: '#2e3440', foreground: '#eceff4' })
    act(() => useTheme.getState().update({ theme: 'system' }))
  })
  it('WEB-016: Terminal-Tabs ohne verschachtelte Fokusziele; F2 benennt um, Entf schließt bzw. blendet aus', async () => {
    resetSeq()
    const { TerminalsPanel } = await import('@/components/workspace/terminals')
    const x = fakeXterm()
    const events = [opened1({ name: 'Alt' }), ev('terminal.closed', { terminal_id: 'term_1', exit_code: 1 }), ev('terminal.opened', { terminal_id: 'term_2', channel_id: 2, kind: 'user', cmd: '/bin/zsh', cols: 80, rows: 24, name: 'zsh' })]
    render(<TerminalsPanel sessionId="ses_test" events={events} load={x.load} />)
    for (const tab of screen.getAllByRole('tab')) expect(tab.querySelector('button, input, [tabindex]')).toBeNull()
    const live = screen.getByRole('tab', { name: /zsh/ })
    expect(live.getAttribute('aria-keyshortcuts')).toBe('Delete F2')
    fireEvent.keyDown(live, { key: 'F2' })
    expect(screen.getByLabelText('Name des Terminals')).toBeTruthy()
    fireEvent.keyDown(screen.getByLabelText('Name des Terminals'), { key: 'Escape' })
    const ended = screen.getByRole('tab', { name: /Alt/ })
    expect(ended.getAttribute('aria-keyshortcuts')).toBe('Delete')
    fireEvent.keyDown(ended, { key: 'Delete' })
    expect(screen.queryByRole('tab', { name: /Alt/ })).toBeNull()
  })
})
