import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const nav = vi.hoisted(() => ({ navigate: vi.fn() }))
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => nav.navigate, useRouterState: () => '/s/ses_a' }))
vi.mock('@/lib/client', () => ({ client: { inbox: { list: () => Promise.resolve({ items: [], open_count: 0 }) } } }))

import { CommandPalette, paletteItems } from '@/components/command-palette'
import { ShortcutsOverlay, shortcutRows } from '@/components/shortcuts-overlay'
import { useUi } from '@/components/ui-state'
import { commands, ensureDispatcher, rebind, registerCommand, resetSequence, resetShortcuts, shortcutOf, swapShortcut, useKeymap } from '@/lib/commands'
import { fuzzyScore } from '@/lib/fuzzy'
import { t } from '@/lib/i18n'
import { assignable, chordText, formatChord, normalize, strokeOf } from '@/lib/keys'

function wrap(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

const offs: (() => void)[] = []
const reg = (def: Parameters<typeof registerCommand>[0]) => offs.push(registerCommand(def))

beforeEach(() => {
  ensureDispatcher()
  resetShortcuts()
  resetSequence()
  nav.navigate.mockReset()
  useUi.setState({ palette: false, switcher: false, shortcuts: false, newSession: false })
})
afterEach(() => {
  cleanup()
  while (offs.length) offs.pop()!()
})

const key = (init: KeyboardEventInit, target: EventTarget = window) =>
  act(() => {
    target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))
  })

describe('Tastenkürzel als Text (UX-004)', () => {
  it('normalisiert, formatiert plattformabhängig und erkennt belegbare Kürzel', () => {
    expect(normalize('Shift+Mod+m')).toBe('Mod+Shift+M')
    expect(formatChord('Mod+Shift+M', true)).toEqual(['⌘⇧M'])
    expect(formatChord('Mod+Shift+M', false)).toEqual(['Ctrl+Shift+M'])
    expect(formatChord('Escape Escape', true)).toEqual(['Esc', 'Esc'])
    expect(formatChord('J / K', true)).toEqual(['j', '/', 'k'])
    expect(chordText('Mod+Enter', true)).toBe('⌘↵')
    expect(assignable('Mod+K')).toBe(true)
    expect(assignable('F2')).toBe(true)
    expect(assignable('K')).toBe(false)
    expect(assignable('Shift+K')).toBe(false)
    // ⌥S liefert auf macOS „ß“: maßgeblich ist die Taste. `/` braucht auf deutschen Tastaturen ⇧.
    expect(strokeOf({ key: 'ß', code: 'KeyS', metaKey: true, ctrlKey: false, altKey: true, shiftKey: false }, true)).toBe('Mod+Alt+S')
    expect(strokeOf({ key: '/', code: 'Digit7', metaKey: true, ctrlKey: false, altKey: false, shiftKey: true }, true)).toBe('Mod+/')
    expect(strokeOf({ key: 'k', code: 'KeyK', metaKey: false, ctrlKey: true, altKey: false, shiftKey: false }, false)).toBe('Mod+K')
  })
})

describe('Befehlspalette (UX-002)', () => {
  it('UX-002 AC1: „fork“ in einer Session zeigt „Session forken“ als ersten Treffer; Enter führt den Fork aus', () => {
    const fork = vi.fn()
    reg({ id: 'session.fork', run: fork, when: () => true })
    reg({ id: 'session.continue.codex', title: 'Weiter mit Codex', hint: '(Fork)', group: 'session', keywords: ['fork', 'harness'], run: vi.fn() })
    reg({ id: 'session.new', run: vi.fn() })
    act(() => useUi.setState({ palette: true }))
    wrap(<CommandPalette />)
    const input = screen.getByRole('combobox')
    expect(document.activeElement).toBe(input)
    fireEvent.change(input, { target: { value: 'fork' } })
    const options = screen.getAllByRole('option')
    expect(options[0]!.textContent).toContain('Session forken')
    expect(options[0]!.getAttribute('aria-selected')).toBe('true')
    expect(options.map((o) => o.textContent)).toContain('Weiter mit Codex(Fork)')
    vi.useFakeTimers()
    try {
      fireEvent.keyDown(input, { key: 'Enter' })
      vi.runAllTimers()
    } finally {
      vi.useRealTimers()
    }
    expect(fork).toHaveBeenCalledTimes(1)
    expect(useUi.getState().palette).toBe(false)
  })

  it('UX-002 AC2: Befehle mit `when` erscheinen nur, wenn die Bedingung erfüllt ist', () => {
    let running = false
    const interrupt = vi.fn()
    reg({ id: 'session.interrupt', run: interrupt, when: () => running })
    reg({ id: 'session.fork', run: vi.fn() })
    act(() => useUi.setState({ palette: true }))
    const { unmount } = wrap(<CommandPalette />)
    expect(screen.queryByText('Unterbrechen')).toBeNull()
    expect(screen.getByText('Session forken')).toBeTruthy()
    unmount()
    running = true
    act(() => useUi.setState({ palette: true }))
    wrap(<CommandPalette />)
    expect(screen.getByText('Unterbrechen')).toBeTruthy()
    // Auch das Kürzel greift nur bei erfüllter Bedingung: `Esc Esc` ohne laufenden Turn tut nichts.
    cleanup()
    running = false
    key({ key: 'Escape' })
    key({ key: 'Escape' })
    expect(interrupt).not.toHaveBeenCalled()
    running = true
    key({ key: 'Escape' })
    key({ key: 'Escape' })
    expect(interrupt).toHaveBeenCalledTimes(1)
  })

  it('UX-002 AC3: vollständig per Tastatur: Pfeile wählen, Enter führt aus, Esc schließt', () => {
    const a = vi.fn()
    const b = vi.fn()
    reg({ id: 'x.a', title: 'Alpha', group: 'general', order: 1, run: a })
    reg({ id: 'x.b', title: 'Beta', group: 'general', order: 2, run: b })
    act(() => useUi.setState({ palette: true }))
    wrap(<CommandPalette />)
    const input = screen.getByRole('combobox')
    const dialog = screen.getByRole('dialog', { name: 'Befehlspalette' })
    expect(dialog.getAttribute('aria-modal')).toBe('true')
    const listbox = screen.getByRole('listbox')
    expect(input.getAttribute('aria-controls')).toBe(listbox.id)
    const first = within(listbox).getAllByRole('option')[0]!
    expect(input.getAttribute('aria-activedescendant')).toBe(first.id)
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    const selected = within(listbox).getAllByRole('option').find((o) => o.getAttribute('aria-selected') === 'true')!
    expect(input.getAttribute('aria-activedescendant')).toBe(selected.id)
    // Esc schließt die Palette.
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(useUi.getState().palette).toBe(false)
  })

  it('UX-002: keine Treffer zeigt den Leerzustand aus dem Entwurf', () => {
    act(() => useUi.setState({ palette: true }))
    wrap(<CommandPalette />)
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'deploy prod' } })
    expect(screen.getByText('Nichts gefunden für „deploy prod“.')).toBeTruthy()
    expect(screen.queryAllByRole('option')).toHaveLength(0)
  })

  it('UX-002: unscharfe Suche gewichtet Wortanfänge im Titel vor Suchwörtern', () => {
    expect(fuzzyScore('fork', 'Session forken')).toBeGreaterThan(fuzzyScore('fork', 'Weiter mit Codex', ['fork']))
    expect(fuzzyScore('nsess', 'Neue Session')).toBeGreaterThan(0)
    expect(fuzzyScore('xyz', 'Neue Session')).toBe(0)
    const groups = paletteItems('modell', [
      { id: 'a', group: 'general', title: 'Neue Session', order: 0, run: () => undefined },
      { id: 'b', group: 'session', title: 'Modell wechseln', order: 0, run: () => undefined },
    ])
    expect(groups.map((g) => g.group)).toEqual(['session'])
  })
})

describe('Tastenkürzel (UX-004)', () => {
  it('UX-004 AC1: ⌘/ öffnet das Overlay; alle Kürzel der Registry sind gelistet', () => {
    reg({ id: 'shortcuts.open', run: () => useUi.getState().set({ shortcuts: true }) })
    wrap(<ShortcutsOverlay />)
    key({ key: '/', code: 'Slash', ctrlKey: true })
    expect(useUi.getState().shortcuts).toBe(true)
    cleanup()
    wrap(<ShortcutsOverlay />)
    const listed = [...document.querySelectorAll('[data-shortcut]')].map((el) => el.getAttribute('data-shortcut'))
    const registry = commands()
      .filter((c) => c.shortcut && c.section)
      .map((c) => c.id)
    expect(new Set(listed)).toEqual(new Set(registry))
    expect(listed).toHaveLength(registry.length)
    // Die Freigabe-Kürzel der Leiste (WEB-007) und die Inbox-Tasten (UX-001) sind dabei.
    expect(listed).toEqual(expect.arrayContaining(['approval.allow', 'approval.deny', 'approval.focus', 'inbox.step', 'session.interrupt']))
    expect(shortcutRows(commands(), {}, t).find((r) => r.id === 'session.interrupt')?.chord).toBe('Escape Escape')
  })

  it('UX-004 AC2: kollidierende Umbelegung wird mit Hinweis abgelehnt oder nach Bestätigung getauscht', () => {
    // Ohne UI: abgelehnt mit dem Besitzer des Kürzels.
    const res = rebind('session.model', 'Mod+K')
    expect(res).toMatchObject({ ok: false, reason: 'conflict' })
    expect(res.ok === false && res.reason === 'conflict' && res.with.id).toBe('palette.open')
    expect(shortcutOf(commands().find((c) => c.id === 'session.model')!)).toBe('Mod+Shift+M')
    // Freie Kombination wird übernommen und gespeichert.
    expect(rebind('session.model', 'Mod+Alt+M')).toEqual({ ok: true })
    expect(JSON.parse(localStorage.getItem('beton.shortcuts.v1')!)).toEqual({ 'session.model': 'Mod+Alt+M' })
    resetShortcuts()

    // Im Overlay: „Ändern“, neue Tasten, Konflikt mit Hinweis, „Tauschen“.
    wrap(<ShortcutsOverlay />)
    act(() => useUi.setState({ shortcuts: true }))
    fireEvent.click(screen.getByRole('button', { name: '„Modell wechseln“ ändern' }))
    expect(screen.getByTestId('shortcut-recording')).toBeTruthy()
    key({ key: 'k', code: 'KeyK', ctrlKey: true })
    const alert = screen.getByRole('alert')
    expect(alert.textContent).toContain('gehört schon zu „Befehlspalette“')
    // Abbrechen lässt alles, wie es war.
    fireEvent.click(within(alert).getByRole('button', { name: 'Abbrechen' }))
    expect(useKeymap.getState().overrides).toEqual({})
    fireEvent.click(screen.getByRole('button', { name: '„Modell wechseln“ ändern' }))
    key({ key: 'k', code: 'KeyK', ctrlKey: true })
    fireEvent.click(within(screen.getByRole('alert')).getByRole('button', { name: 'Tauschen' }))
    const cmd = (id: string) => commands().find((c) => c.id === id)!
    expect(shortcutOf(cmd('session.model'))).toBe('Mod+K')
    expect(shortcutOf(cmd('palette.open'))).toBe('Mod+Shift+M')
    expect(swapShortcut('session.model', 'Mod+Shift+M')).toBe(true)
    expect(useKeymap.getState().overrides).toEqual({})
  })

  it('UX-004: „Ändern“ nimmt neue Tasten auf – Hinweis, was belegbar ist, „Abbrechen“ und Esc brechen ab', () => {
    wrap(<ShortcutsOverlay />)
    act(() => useUi.setState({ shortcuts: true }))
    fireEvent.click(screen.getByRole('button', { name: '„Modell wechseln“ ändern' }))
    // Im Rahmen steht „Tasten drücken …“ statt der bisherigen Belegung.
    expect(screen.getByTestId('shortcut-recording').textContent).toBe('Tasten drücken …')
    const hint = screen.getByTestId('shortcut-hint')
    expect(hint.textContent).toContain('Neue Tasten für „Modell wechseln“ drücken. Belegbar sind Kombinationen mit ⌘, ⌥ oder Ctrl und Funktionstasten;')
    expect(hint.textContent).toContain('bricht ab.')
    expect(hint.querySelector('kbd')?.textContent).toBe('Esc')
    // Tasten ohne Modifier zählen nicht; die Aufnahme läuft weiter.
    key({ key: 'm', code: 'KeyM' })
    expect(screen.getByTestId('shortcut-recording')).toBeTruthy()
    // „Abbrechen“ beendet die Aufnahme, nichts ändert sich.
    fireEvent.click(within(hint).getByRole('button', { name: 'Abbrechen' }))
    expect(screen.queryByTestId('shortcut-recording')).toBeNull()
    expect(screen.queryByTestId('shortcut-hint')).toBeNull()
    expect(useKeymap.getState().overrides).toEqual({})
    // Esc ebenso, ohne das Overlay zu schließen.
    fireEvent.click(screen.getByRole('button', { name: '„Modell wechseln“ ändern' }))
    key({ key: 'Escape', code: 'Escape' })
    expect(screen.queryByTestId('shortcut-hint')).toBeNull()
    expect(useUi.getState().shortcuts).toBe(true)
    expect(useKeymap.getState().overrides).toEqual({})
  })

  it('UX-004: Suche ohne Treffer zeigt „Kein Kürzel passt zu …“ statt eines leeren Rasters', () => {
    wrap(<ShortcutsOverlay />)
    act(() => useUi.setState({ shortcuts: true }))
    expect(screen.queryByText(/Kein Kürzel passt/)).toBeNull()
    fireEvent.change(screen.getByRole('textbox', { name: 'Kürzel suchen' }), { target: { value: 'deploy' } })
    expect(screen.getByText('Kein Kürzel passt zu „deploy“.')).toBeTruthy()
    expect(screen.getByText('Gesucht wird in Namen und Tasten.')).toBeTruthy()
    expect(document.querySelectorAll('[data-shortcut]')).toHaveLength(0)
    expect(screen.queryByTestId('shortcut-list')).toBeNull()
  })

  it('UX-004 AC3: Kürzel greifen nicht im Eingabefeld, außer `globalInInput` (z. B. ⌘K)', () => {
    const plain = vi.fn()
    const global = vi.fn()
    reg({ id: 'x.plain', title: 'Plain', shortcut: 'Mod+J', run: plain })
    reg({ id: 'x.global', title: 'Global', shortcut: 'Mod+L', globalInInput: true, run: global })
    const input = document.createElement('textarea')
    document.body.append(input)
    try {
      key({ key: 'j', code: 'KeyJ', ctrlKey: true }, input)
      key({ key: 'l', code: 'KeyL', ctrlKey: true }, input)
      expect(plain).not.toHaveBeenCalled()
      expect(global).toHaveBeenCalledTimes(1)
      key({ key: 'j', code: 'KeyJ', ctrlKey: true }, document.body)
      expect(plain).toHaveBeenCalledTimes(1)
      // Die Palette ist als `globalInInput` deklariert.
      expect(commands().find((c) => c.id === 'palette.open')?.globalInInput).toBe(true)
    } finally {
      input.remove()
    }
  })
})
