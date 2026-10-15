import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import type { UsageItem, UsageReportView } from '@beton/sdk'

const api = vi.hoisted(() => ({ report: vi.fn(), pricing: vi.fn(), info: vi.fn(), navigate: vi.fn() }))
vi.mock('@/lib/client', () => ({ client: { usage: { report: api.report, pricing: api.pricing }, info: api.info } }))
vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => api.navigate,
  Link: ({ to, params, children, ...rest }: { to: string; params?: Record<string, string>; children: ReactNode }) => (
    <a href={params?.sessionId ? to.replace('$sessionId', params.sessionId) : to} {...rest}>
      {children}
    </a>
  ),
}))

import { UsageView } from '@/components/usage/usage-view'
import { PricingView, unpricedModels } from '@/components/usage/pricing-view'
import { csvFilename, kpis, localDate, resetText, rowsFor, subscriptionRows, toCsv } from '@/lib/usage'

const NOW = new Date()
const TODAY = localDate(NOW)

function item(p: Partial<UsageItem>): UsageItem {
  return {
    billing: 'metered',
    turns: 1,
    sessions: 1,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    cost_usd: 0,
    equivalent_usd: null,
    unpriced_count: 0,
    ...p,
  }
}

function report(groups: UsageItem[], extra: Partial<UsageReportView> = {}): UsageReportView {
  return {
    from: '',
    to: '',
    tz: '+00:00',
    group_by: [],
    pricing_version: '2026-10-14',
    groups,
    totals: { metered: item({}), subscription: item({ billing: 'subscription' }) },
    subscriptions: [],
    reporting: [],
    ...extra,
  }
}

const SESSIONS = [
  item({ session_id: 'ses_a', session_title: 'Rate-Limiter', harness: 'claude', billing: 'subscription', turns: 14, input_tokens: 2_210_000, output_tokens: 186_000, cache_read_tokens: 1_840_000, cost_usd: null }),
  item({ session_id: 'ses_b', session_title: 'Lasttest auswerten', harness: 'direct:litellm', turns: 4, input_tokens: 480_000, output_tokens: 66_000, cost_usd: 0.24 }),
  item({ session_id: 'ses_c', session_title: 'Mistral testen', harness: 'direct:openrouter', model: 'mistral-large-2511', turns: 2, input_tokens: 140_000, output_tokens: 18_000, cost_usd: null, unpriced_count: 2 }),
]

function mockApi() {
  api.report.mockImplementation(async (opts: { groupBy: string[] }) => {
    const by = opts.groupBy.join(',')
    if (by === 'day') return report([item({ day: TODAY, billing: 'metered', turns: 6, input_tokens: 620_000, output_tokens: 84_000, cost_usd: 0.24 })])
    if (by === 'day,harness')
      return report([item({ day: TODAY, harness: 'claude', billing: 'subscription', turns: 14, input_tokens: 2_210_000, output_tokens: 186_000, cost_usd: null })], {
        subscriptions: [
          { harness: 'claude', vendor: 'anthropic', window: '5h', used_pct: 92, resets_at: new Date(NOW.getTime() + 41 * 60_000).toISOString(), observed_at: '', session_id: 'ses_a', expired: false },
        ],
        reporting: [{ harness: 'claude', sessions: 1, tokens: true, rate_limits: true }],
      })
    return report(SESSIONS)
  })
  api.info.mockResolvedValue({ version: '0.9.2' })
}

function renderView(node: ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>)
}

beforeEach(() => {
  api.report.mockReset()
  api.pricing.mockReset()
  api.info.mockReset()
  api.navigate.mockReset()
  mockApi()
})
afterEach(cleanup)

describe('Verbrauch (USE-006)', () => {
  it('USE-006 AC1: Zeilen und Kennzahlen sind die Summen der API-Gruppen', () => {
    const rows = rowsFor('harness', [
      item({ harness: 'claude', auth_source: 'vendor_cli', billing: 'subscription', turns: 3, input_tokens: 100, output_tokens: 10, cache_read_tokens: 5, cost_usd: null }),
      item({ harness: 'claude', auth_source: 'api_key', turns: 2, input_tokens: 50, output_tokens: 5, cache_read_tokens: 1, cost_usd: 1.5 }),
    ], TODAY)
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({ turns: 5, input: 150, output: 15, cache: 6, billing: { kind: 'cost', usd: 1.5 } })
    const k = kpis([item({ day: TODAY, turns: 2, input_tokens: 1000, output_tokens: 100, cost_usd: 0.5 }), item({ day: TODAY, billing: 'subscription', turns: 1, input_tokens: 10, output_tokens: 1, cost_usd: null, equivalent_usd: 9 })], NOW)
    expect(k.today).toEqual({ tokens: 1111, turns: 3 })
    // Subscription trägt nie zu Kosten bei, auch nicht als API-Äquivalent (ADR-0034).
    expect(k.cost7).toBe(0.5)
  })

  it('USE-006 AC2: Klick auf eine Session in der Tabelle öffnet die Session', async () => {
    renderView(<UsageView />)
    const table = await screen.findByTestId('usage-table')
    const row = await within(table).findByText('Rate-Limiter')
    fireEvent.click(row.closest('tr')!.querySelector('td:nth-child(2)')!)
    expect(api.navigate).toHaveBeenCalledWith({ to: '/s/$sessionId', params: { sessionId: 'ses_a' } })
    expect(row.closest('a')?.getAttribute('href')).toBe('/s/ses_a')
  })

  it('USE-006 AC3: CSV-Export enthält die sichtbaren Spalten und Filter im Dateinamen', async () => {
    const rows = rowsFor('session', SESSIONS, TODAY)
    const csv = toCsv('session', rows)
    const [head, ...lines] = csv.trim().split('\r\n')
    expect(head).toBe('Session,Turns,Eingabe,Ausgabe,Cache gelesen,Abrechnung')
    expect(lines).toContain('Rate-Limiter,14,2210000,186000,1840000,Subscription')
    expect(lines).toContain('Mistral testen,2,140000,18000,0,ohne Preis')
    expect(csvFilename('session', { from: '2026-09-20', to: '2026-10-03' })).toBe('verbrauch_2026-09-20_2026-10-03_nach-session.csv')

    const created: Blob[] = []
    const clicked: string[] = []
    URL.createObjectURL = vi.fn((b: Blob) => {
      created.push(b)
      return 'blob:x'
    }) as typeof URL.createObjectURL
    URL.revokeObjectURL = vi.fn()
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      clicked.push(this.download)
    })
    renderView(<UsageView />)
    await screen.findByTestId('usage-table')
    const button = screen.getByRole('button', { name: /CSV exportieren/ })
    fireEvent.click(button)
    click.mockRestore()
    expect(clicked[0]).toMatch(/^verbrauch_\d{4}-\d{2}-\d{2}_\d{4}-\d{2}-\d{2}_nach-session\.csv$/)
    expect(button.getAttribute('title')).toBe(clicked[0])
    const text = await new Promise<string>((resolve) => {
      const reader = new FileReader()
      reader.onload = () => resolve(String(reader.result))
      reader.readAsText(created[0]!)
    })
    expect(text).toContain('Lasttest auswerten,4,480000,66000,0,"0,24')
    // Andere Gruppierung: anderer Dateiname.
    fireEvent.click(screen.getByRole('radio', { name: 'Modell' }))
    expect(screen.getByRole('button', { name: /CSV exportieren/ }).getAttribute('title')).toMatch(/_nach-modell\.csv$/)
  })

  it('USE-006 AC4: Records ohne Preis erscheinen mit Hinweis und Link zu den Preisen', async () => {
    renderView(<UsageView />)
    const link = await screen.findByTestId('unpriced-link')
    expect(link.textContent).toBe('ohne Preis – eigenen Preis anlegen')
    expect(link.getAttribute('href')).toBe('/usage/pricing')
    // Ziel: die Preise-Seite nennt das Modell ohne Preis.
    expect(unpricedModels(SESSIONS)).toEqual([{ label: 'openrouter/mistral-large-2511', count: 2 }])
  })

  it('USE-006: Subscriptions zeigen Kontingente ohne Geldbetrag und warnen ab 90 %', async () => {
    renderView(<UsageView />)
    const row = await screen.findByTestId('subscription-row')
    expect(row.textContent).toContain('5-Stunden-Fenster')
    expect(row.textContent).toContain('92 %')
    expect(row.textContent).toContain('fast aufgebraucht')
    expect(row.textContent).toMatch(/setzt in 4[01] Min\. zurück/)
    expect(row.textContent).not.toContain('$')
    const monday = new Date(NOW.getTime() + 3 * 24 * 3600_000)
    expect(resetText(monday.toISOString(), NOW)).toMatch(/^setzt (Mo|Di|Mi|Do|Fr|Sa|So)\. \d{2}:\d{2} zurück$/)
    const subs = subscriptionRows([], [{ harness: 'acp:gemini', sessions: 1, tokens: true, rate_limits: false }], [item({ day: TODAY, harness: 'acp:gemini', billing: 'subscription', input_tokens: 380_000, cost_usd: null })], TODAY)
    expect(subs[0]).toMatchObject({ harness: 'acp:gemini', reportsLimits: false, windows: [], periodTokens: 380_000 })
  })

  it('USE-006: ohne Verbrauch steht der Leerzustand', async () => {
    api.report.mockResolvedValue(report([]))
    renderView(<UsageView />)
    expect(await screen.findByText('Noch kein Verbrauch')).toBeTruthy()
  })

  it('USE-006: die Preise-Seite zeigt eigene Preise, Katalog und Turns ohne Preis', async () => {
    api.pricing.mockResolvedValue({
      catalog_version: '2026-10-14',
      currency: 'USD',
      overrides: [
        { position: 1, provider: 'litellm-intern', model: 'llama-3.3-70b*', free: false, input_per_mtok: 0.4, output_per_mtok: 0.8, cache_read_per_mtok: 0.04, cache_write_per_mtok: 0.5, cache_estimated: true, context_window: 131_072 },
        { position: 2, provider: 'ollama-local', model: null, free: true, input_per_mtok: 0, output_per_mtok: 0, cache_read_per_mtok: 0, cache_write_per_mtok: 0, cache_estimated: false, context_window: null },
      ],
      catalog: [{ model: 'claude-opus-5-5', aliases: [], provider: 'anthropic', context_window: 1_000_000, input_per_mtok: 5, output_per_mtok: 25, cache_read_per_mtok: 0.5, cache_write_per_mtok: null, valid_from: '2026-10-14' }],
    })
    renderView(<PricingView />)
    const overrides = await screen.findByTestId('overrides-table')
    expect(overrides.textContent).toContain('litellm-intern / llama-3.3-70b*')
    expect(overrides.textContent).toContain('0,04 $ / 0,50 $ geschätzt')
    expect(overrides.textContent).toContain('ollama-local / *')
    expect(overrides.textContent).toContain('kostenlos')
    const catalog = screen.getByTestId('catalog-table')
    expect(catalog.textContent).toContain('claude-opus-5-5')
    expect(catalog.textContent).toContain('—')
    expect((await screen.findByTestId('unpriced-hint')).textContent).toContain('2 Turns ohne Preis')
  })
})
