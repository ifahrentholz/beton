import { cleanup, render, screen, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ShadowReport } from '@beton/sdk'

const api = vi.hoisted(() => ({ shadow: vi.fn() }))
vi.mock('@/lib/client', () => ({ client: { policies: { shadow: api.shadow } } }))
vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, params }: { children: ReactNode; params: { sessionId: string } }) => <a href={`/s/${params.sessionId}`}>{children}</a>,
}))

import { PolicyShadowView, dayLabel } from '@/components/policy-shadow'

const days = ['2026-10-08', '2026-10-09', '2026-10-10', '2026-10-11', '2026-10-12', '2026-10-13', '2026-10-14']

function view() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <PolicyShadowView />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  api.shadow.mockReset()
})
afterEach(cleanup)

describe('Shadow-Modus (POL-027)', () => {
  it('POL-027 AC4: Treffer je Regel und Tag als „würde ablehnen“ mit letztem Treffer', async () => {
    const report: ShadowReport = {
      days,
      rules: [
        {
          rule_id: 'no-npm-publish',
          scope: 'project',
          set: 'shadow-npm',
          verdict: 'deny',
          counts: [0, 1, 0, 3, 2, 0, 1],
          total: 7,
          last: { session_id: 'ses_6p9z', session_title: 'Release vorbereiten', seq: 42, at: new Date().toISOString(), summary: 'Bash: pnpm publish --dry-run' },
        },
      ],
    }
    api.shadow.mockResolvedValue(report)
    view()
    const row = await screen.findByTestId('shadow-rule')
    expect(api.shadow).toHaveBeenCalledWith(7)
    expect(within(row).getByText('no-npm-publish')).toBeTruthy()
    expect(within(row).getByText('shadow-npm (Projekt)')).toBeTruthy()
    expect(within(row).getByText('würde ablehnen')).toBeTruthy()
    const counts = within(row).getAllByTestId('shadow-count').map((c) => c.textContent)
    expect(counts).toEqual(['0', '1', '0', '3', '2', '0', '1'])
    expect(within(row).getByText(/pnpm publish --dry-run/)).toBeTruthy()
    expect(within(row).getByRole('link').getAttribute('href')).toBe('/s/ses_6p9z')
    expect(screen.getByText('heute')).toBeTruthy()
    // Kein Scharfschalten in der Web-UI (schreibender Endpunkt fehlt).
    expect(screen.queryByText('Scharf schalten')).toBeNull()
  })

  it('POL-027 AC4: ohne Shadow-Regel erklärt die Ansicht, wie man eine anlegt', async () => {
    api.shadow.mockResolvedValue({ days, rules: [] })
    view()
    const empty = await screen.findByTestId('shadow-empty')
    expect(empty.textContent).toContain('mode: dry_run')
  })

  it('POL-027 AC4: ein Ladefehler nennt Ursache und nächsten Schritt', async () => {
    api.shadow.mockRejectedValue(new Error('Verbindung abgelehnt'))
    view()
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toContain('Verbindung abgelehnt')
    expect(alert.textContent).toContain('beton serve')
  })

  it('beschriftet Tage mit Wochentag und den letzten mit „heute“', () => {
    expect(dayLabel('2026-10-09', false)).toBe('Fr')
    expect(dayLabel('2026-10-14', true)).toBe('heute')
  })
})
