import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to, ...rest }: { children: React.ReactNode; to: string }) => (
    <a href={to} {...rest}>
      {children}
    </a>
  ),
}))

import { AppearanceSettingsView, formatSample } from '@/components/settings/appearance'
import { de } from '@/i18n/de'
import { en } from '@/i18n/en'
import { ago, cost } from '@/lib/format'
import { detectLanguage, translate, useI18n } from '@/lib/i18n'

afterEach(() => {
  cleanup()
  act(() => useI18n.getState().setPref('de'))
})

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort()

describe('Internationalisierung (WEB-017)', () => {
  it('WEB-017 AC1: der Check schlägt fehl, wenn ein Schlüssel in `de` oder `en` fehlt', () => {
    const deKeys = Object.keys(de).sort()
    const enKeys = Object.keys(en).sort()
    expect(enKeys.filter((k) => !deKeys.includes(k))).toEqual([])
    expect(deKeys.filter((k) => !enKeys.includes(k))).toEqual([])
    // Keine leeren Texte, gleiche Platzhalter in beiden Sprachen.
    for (const k of deKeys as (keyof typeof de)[]) {
      expect(de[k].trim(), k).not.toBe('')
      expect(en[k].trim(), k).not.toBe('')
      expect(placeholders(en[k]), k).toEqual(placeholders(de[k]))
    }
  })

  it('WEB-017 AC2: Umschalten der Sprache wirkt ohne Neuladen', () => {
    render(<AppearanceSettingsView />)
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Darstellung')
    expect(document.documentElement.lang).toBe('de')
    fireEvent.click(screen.getByRole('radio', { name: 'English' }))
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Appearance')
    expect(screen.getByRole('switch', { name: 'High contrast' })).toBeTruthy()
    expect(document.documentElement.lang).toBe('en')
    expect(localStorage.getItem('beton.lang')).toBe('en')
    // Formate folgen der Sprache (Intl).
    expect(screen.getByTestId('format-sample').textContent).toContain('1,234.5 · $4.82')
    expect(cost(4_820_000)).toBe('$4.82')
    fireEvent.click(screen.getByRole('radio', { name: 'Deutsch' }))
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Darstellung')
    expect(cost(4_820_000).replace(/\s/g, ' ')).toBe('4,82 $')
    expect(screen.getByTestId('format-sample').textContent).toContain('1.234,5 · 4,82')
    expect(formatSample('de-DE', new Date('2026-10-03T14:05:00')).replace(/\s/g, ' ')).toBe('03.10.2026 · 14:05 · 1.234,5 · 4,82 $')
  })

  it('WEB-017: Sprache automatisch aus Browser/OS, Englisch als Fallback', () => {
    expect(detectLanguage(['de-AT', 'en-US'])).toBe('de')
    expect(detectLanguage(['fr-FR', 'en-GB'])).toBe('en')
    expect(detectLanguage(['fr-FR'])).toBe('en')
    expect(detectLanguage([])).toBe('en')
    act(() => useI18n.getState().setPref('auto'))
    expect(localStorage.getItem('beton.lang')).toBeNull()
    expect(translate('en', 'cmd.inboxHint', { count: 2 })).toBe('2 open')
    expect(translate('de', 'cmd.inboxHint', { count: 2 })).toBe('2 offen')
    act(() => useI18n.getState().setPref('en'))
    const now = Date.parse('2026-10-03T14:05:00Z')
    expect(ago('2026-10-03T14:00:00Z', now)).toBe('5 min ago')
    act(() => useI18n.getState().setPref('de'))
    expect(ago('2026-10-03T14:00:00Z', now)).toBe('vor 5 Min.')
  })
})
