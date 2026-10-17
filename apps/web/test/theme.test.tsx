import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to, ...rest }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...rest}>
      {children}
    </a>
  ),
}))

import { AppearanceSettingsView } from '@/components/settings/appearance'
import { currentCodeTheme } from '@/lib/highlight'
import { applyToDocument, monacoBase, resolveTheme, setHighContrast, shikiTheme, startTheme, terminalTheme, toggleMode, useTheme } from '@/lib/theme'

/** `matchMedia` mit umschaltbarer Systemeinstellung. */
function mockSystem(dark: boolean) {
  const listeners = new Set<() => void>()
  const mq = {
    get matches() {
      return dark
    },
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
  }
  window.matchMedia = vi.fn().mockReturnValue(mq) as unknown as typeof window.matchMedia
  return {
    set(next: boolean) {
      dark = next
      listeners.forEach((fn) => fn())
    },
  }
}

const root = document.documentElement
let stop: (() => void) | undefined

beforeEach(() => {
  localStorage.removeItem('beton.appearance.v1')
  useTheme.getState().update({ theme: 'system', motion: 'system', focusAlways: true, announce: true, fontSize: 14, beforeContrast: undefined })
})
afterEach(() => {
  stop?.()
  stop = undefined
  cleanup()
})

describe('Themes (UX-005)', () => {
  it('UX-005 AC1: Theme-Wechsel wirkt ohne Neuladen auf UI-Tokens, Monaco, xterm und Code-Blöcke', () => {
    mockSystem(false)
    stop = startTheme()
    expect(root.classList.contains('light')).toBe(true)
    expect(currentCodeTheme()).toBe('github-light')
    act(() => useTheme.getState().update({ theme: 'nord' }))
    // UI: Klassen am Wurzelelement wählen den Token-Satz.
    expect(root.classList.contains('theme-nord')).toBe(true)
    expect(root.classList.contains('light')).toBe(true)
    act(() => useTheme.getState().update({ theme: 'concrete-dark' }))
    expect(root.classList.contains('theme-nord')).toBe(false)
    expect(root.classList.contains('dark')).toBe(true)
    // Code-Blöcke (Shiki) und Monaco folgen Familie und Modus.
    expect(currentCodeTheme()).toBe('github-dark')
    expect(shikiTheme('nord', 'dark')).toBe('nord')
    expect(shikiTheme('hc', 'light')).toBe('github-light-high-contrast')
    expect(monacoBase('hc', 'dark')).toBe('hc-black')
    expect(monacoBase('concrete', 'light')).toBe('vs')
    // xterm: Farben aus den Tokens des Themes.
    const tokens: Record<string, string> = { '--sunken': '#000000', '--foreground': '#ffffff', '--deny': '#ff9f97' }
    const term = terminalTheme((n) => tokens[n] ?? '')
    expect(term).toMatchObject({ background: '#000000', foreground: '#ffffff', red: '#ff9f97' })
    // „Theme wechseln“ schaltet hell/dunkel.
    act(() => toggleMode())
    expect(root.classList.contains('light')).toBe(true)
    expect(useTheme.getState().theme).toBe('concrete-light')
    // Die Auswahl gilt je Gerät.
    expect(JSON.parse(localStorage.getItem('beton.appearance.v1')!).theme).toBe('concrete-light')
  })

  it('UX-005 AC2: alle Themes erfüllen WCAG AA für Fließtext, „Hoher Kontrast“ AAA (Token-Paare)', () => {
    const css = readFileSync(join(import.meta.dirname, '..', 'src', 'index.css'), 'utf8')
    const sets = [...css.matchAll(/\/\* ([^*]+?) \*\/\s*([^{}]+)\{([^}]*)\}/g)]
      .map((m) => ({ name: m[1]!.trim(), tokens: Object.fromEntries([...m[3]!.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((t) => [t[1]!, t[2]!])) }))
      .filter((s) => 'background' in s.tokens)
    expect(sets.map((s) => s.name.split(' (')[0]!.split(':')[0])).toEqual(['Sichtbeton hell', 'Sichtbeton dunkel', 'Nord hell', 'Nord dunkel', 'Hoher Kontrast hell', 'Hoher Kontrast dunkel'])
    const lum = (hex: string) => {
      const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
      return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!
    }
    const ratio = (a: string, b: string) => {
      const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x)
      return (hi! + 0.05) / (lo! + 0.05)
    }
    // Text auf allen Flächen, auf denen er vorkommt.
    const pairs: [string, string][] = [
      ['foreground', 'background'],
      ['foreground', 'sidebar'],
      ['foreground', 'sunken'],
      ['foreground', 'muted'],
      ['foreground', 'accent'],
      ['card-foreground', 'card'],
      ['popover-foreground', 'popover'],
      ['muted-foreground', 'background'],
      ['muted-foreground', 'sidebar'],
      ['muted-foreground', 'card'],
      ['muted-foreground', 'popover'],
      ['muted-foreground', 'muted'],
      ['muted-foreground', 'sunken'],
      ['muted-foreground', 'accent'],
      ['primary-foreground', 'primary'],
      ['secondary-foreground', 'secondary'],
      ['accent-foreground', 'accent'],
      ['signal-foreground', 'signal'],
      ['deny', 'background'],
      ['deny', 'card'],
      ['ok', 'background'],
      ['warn', 'background'],
      ['foreground', 'signal-soft'],
      ['foreground', 'deny-soft'],
      ['foreground', 'ok-soft'],
      ['foreground', 'warn-soft'],
    ]
    const failures: string[] = []
    for (const set of sets) {
      const min = set.name.startsWith('Hoher Kontrast') ? 7 : 4.5
      for (const [fg, bg] of pairs) {
        const r = ratio(set.tokens[fg]!, set.tokens[bg]!)
        if (!(r >= min)) failures.push(`${set.name}: ${fg} auf ${bg} = ${r.toFixed(2)} < ${min}`)
      }
    }
    expect(failures).toEqual([])
  })

  it('UX-005 AC3: „System“ folgt einer Änderung der OS-Einstellung live', () => {
    const system = mockSystem(false)
    stop = startTheme()
    expect(root.classList.contains('dark')).toBe(false)
    act(() => system.set(true))
    expect(root.classList.contains('dark')).toBe(true)
    expect(useTheme.getState().mode).toBe('dark')
    // Nord und Hoher Kontrast folgen dem System ebenso, feste Varianten nicht.
    act(() => useTheme.getState().update({ theme: 'hc' }))
    expect(root.classList.contains('theme-hc')).toBe(true)
    act(() => system.set(false))
    expect(root.classList.contains('dark')).toBe(false)
    act(() => useTheme.getState().update({ theme: 'concrete-dark' }))
    act(() => system.set(false))
    expect(root.classList.contains('dark')).toBe(true)
    expect(resolveTheme('system', true)).toEqual({ family: 'concrete', mode: 'dark' })
  })
})

describe('Darstellung (Entwurf `settings-appearance`)', () => {
  it('UX-005 WEB-016: Themes, Barrierefreiheit und Schriftgröße wirken sofort', () => {
    mockSystem(false)
    stop = startTheme()
    render(<AppearanceSettingsView />)
    const group = screen.getByRole('radiogroup', { name: 'Theme' })
    const tiles = group.querySelectorAll('[role="radio"]')
    expect([...tiles].map((t) => t.textContent)).toEqual([
      'SystemFolgt dem Betriebssystem, wechselt live',
      'Sichtbeton hellMarkenthema am Tag',
      'Sichtbeton dunkelMarkenthema bei Nacht',
      'NordKühles Blaugrau, hell und dunkel',
      'Hoher KontrastWCAG AAA, hell und dunkel',
    ])
    // Pfeiltasten wählen in der Radiogruppe.
    fireEvent.keyDown(group, { key: 'ArrowRight' })
    expect(useTheme.getState().theme).toBe('concrete-light')
    // Schalter „Hoher Kontrast“ wechselt auf das Theme und zurück.
    fireEvent.click(screen.getByRole('switch', { name: 'Hoher Kontrast' }))
    expect(root.classList.contains('theme-hc')).toBe(true)
    fireEvent.click(screen.getByRole('switch', { name: 'Hoher Kontrast' }))
    expect(useTheme.getState().theme).toBe('concrete-light')
    // Bewegung reduzieren, Fokus, Schriftgröße.
    fireEvent.click(screen.getByRole('radio', { name: 'An' }))
    expect(root.dataset.motion).toBe('reduce')
    fireEvent.click(screen.getByRole('radio', { name: 'Aus' }))
    expect(root.dataset.motion).toBe('full')
    fireEvent.click(screen.getByRole('switch', { name: 'Fokus immer deutlich zeigen' }))
    expect(root.dataset.focus).toBeUndefined()
    fireEvent.click(screen.getByRole('radio', { name: '16 px' }))
    expect(root.style.fontSize).toBe('16px')
    applyToDocument({ family: 'concrete', mode: 'light', motion: 'system', focusAlways: true, fontSize: 14 })
    expect(root.dataset.motion).toBeUndefined()
    setHighContrast(false)
  })
})
