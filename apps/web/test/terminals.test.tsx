import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { openTerminals, runningFor } from '@/lib/terminals'
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
          return Promise.resolve({ terminal_id: 'term_neu' })
        },
        close: () => Promise.resolve(),
        rename: () => Promise.resolve({}),
      },
    }),
  },
}))

/** Fake-xterm: zeichnet Schreiben, Reset und Eingaben auf. */
function fakeXterm() {
  const state = {
    written: [] as string[],
    resets: 0,
    disableStdin: false,
    onData: undefined as undefined | ((d: string) => void),
    onResize: undefined as undefined | ((s: { cols: number; rows: number }) => void),
  }
  const load = () =>
    Promise.resolve({
      createTerminal: () => ({
        term: {
          options: {
            set disableStdin(v: boolean) {
              state.disableStdin = v
            },
          },
          reset: () => (state.resets += 1),
          write: (b: Uint8Array, cb?: () => void) => {
            state.written.push(new TextDecoder().decode(b))
            cb?.()
          },
          onData: (f: (d: string) => void) => (state.onData = f),
          onResize: (f: (s: { cols: number; rows: number }) => void) => (state.onResize = f),
        },
        fit: () => {},
        dispose: () => {},
      }),
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
})
