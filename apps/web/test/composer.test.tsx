import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { Capabilities, PermissionMode } from '@beton/sdk'
import { Composer } from '@/components/composer'
import { settingsOptions, YOLO_BLOCKED, type SettingsPatch } from '@/components/settings-picker'

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

beforeEach(() => localStorage.clear())
afterEach(() => cleanup())

describe('Composer', () => {
  it('WEB-004 AC1: Picker zeigen nur Optionen, die der Harness unterstützt', () => {
    const caps = { ...base, models: ['fake-small', 'fake-large'], efforts: ['low', 'high'], permission_modes: ['plan', 'default'] } as Capabilities
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
