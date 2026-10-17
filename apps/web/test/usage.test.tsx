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
  it('USE-006: während des Ladens stehen Platzhalter, CSV ist noch gesperrt', async () => {
    api.report.mockImplementation(() => new Promise(() => {}))
    renderView(<UsageView />)
    expect(screen.getByRole('heading', { name: 'Verbrauch' })).toBeTruthy()
    expect(screen.getByRole('radiogroup', { name: 'Zeitraum' })).toBeTruthy()
    const kpis = screen.getByTestId('usage-kpis')
    expect(kpis.getAttribute('aria-busy')).toBe('true')
    expect(within(kpis).getAllByText('–')).toHaveLength(4)
    expect(screen.getByRole('status').textContent).toBe('Verbrauch wird geladen …')
    const csv = screen.getByRole('button', { name: /CSV exportieren/ })
    expect(csv).toHaveProperty('disabled', true)
    expect(csv.getAttribute('title')).toBe('Erst möglich, wenn der Verbrauch geladen ist')
  })

  it('USE-006: ist der Daemon nicht erreichbar, steht der Fehler mit „Erneut laden“', async () => {
    api.report.mockRejectedValue(new TypeError('Failed to fetch'))
    renderView(<UsageView />)
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('Verbrauch nicht verfügbar – der lokale Daemon antwortet nicht. Läuft beton serve?Erneut laden')
    expect(screen.queryByTestId('usage-kpis')).toBeNull()
    expect(screen.getByRole('button', { name: /CSV exportieren/ })).toHaveProperty('disabled', true)
    // Erneut laden holt alles neu.
    mockApi()
    fireEvent.click(within(alert).getByRole('button', { name: 'Erneut laden' }))
    expect(await screen.findByTestId('usage-table')).toBeTruthy()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('USE-006: eigener Zeitraum über das Popover – Von/Bis einschließlich, Kurzform im Segment und im CSV-Dateinamen', async () => {
    renderView(<UsageView />)
    await screen.findByTestId('usage-table')
    // Ohne eigenen Zeitraum führt Abbrechen zurück zum vorigen.
    fireEvent.click(screen.getByRole('radio', { name: 'Zeitraum …' }))
    expect(screen.getByRole('radio', { name: 'Zeitraum …' }).getAttribute('aria-checked')).toBe('true')
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Abbrechen' }))
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(screen.getByRole('radio', { name: '14 Tage' }).getAttribute('aria-checked')).toBe('true')
    fireEvent.click(screen.getByRole('radio', { name: 'Zeitraum …' }))
    const dialog = screen.getByRole('dialog', { name: 'Eigener Zeitraum' })
    expect(within(dialog).getByText('Tage in deiner Zeitzone, beide einschließlich')).toBeTruthy()
    const from = within(dialog).getByLabelText('Von')
    const to = within(dialog).getByLabelText('Bis')
    expect(document.activeElement).toBe(from)
    // „Von“ nach „Bis“ sperrt „Anwenden“ mit Hinweis.
    fireEvent.change(from, { target: { value: '2026-10-03' } })
    fireEvent.change(to, { target: { value: '2026-09-20' } })
    expect(within(dialog).getByRole('button', { name: 'Anwenden' })).toHaveProperty('disabled', true)
    expect(within(dialog).getByText('„Von“ liegt nach „Bis“ – tausche die Tage.')).toBeTruthy()
    fireEvent.change(from, { target: { value: '2026-09-20' } })
    fireEvent.change(to, { target: { value: '2026-10-03' } })
    fireEvent.click(within(dialog).getByRole('button', { name: 'Anwenden' }))
    expect(screen.queryByRole('dialog')).toBeNull()
    const segment = screen.getByRole('radio', { name: '20.09.–03.10.' })
    expect(segment.getAttribute('aria-checked')).toBe('true')
    expect(segment.getAttribute('title')).toBe('20.09.–03.10., beide Tage einschließlich')
    await vi.waitFor(() => expect(api.report).toHaveBeenCalledWith(expect.objectContaining({ from: '2026-09-20', to: '2026-10-03', groupBy: ['session', 'harness'] })))
    await vi.waitFor(() => expect(screen.getByRole('button', { name: /CSV exportieren/ }).getAttribute('title')).toBe('verbrauch_2026-09-20_2026-10-03_nach-session.csv'))
    // Esc bricht ab und lässt den Zeitraum, wie er war.
    fireEvent.click(segment)
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(screen.getByRole('radio', { name: '20.09.–03.10.' }).getAttribute('aria-checked')).toBe('true')
    // Ein anderer Zeitraum und zurück: Der eigene bleibt gemerkt.
    fireEvent.click(screen.getByRole('radio', { name: '7 Tage' }))
    expect(screen.getByRole('radio', { name: '20.09.–03.10.' }).getAttribute('aria-checked')).toBe('false')
    fireEvent.click(screen.getByRole('radio', { name: '20.09.–03.10.' }))
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Abbrechen' }))
    expect(screen.getByRole('radio', { name: '20.09.–03.10.' }).getAttribute('aria-checked')).toBe('true')
  })

  it('USE-004 AC1: das API-Äquivalent erscheint nur auf Wunsch, als „informativ“ und nie als Kosten', async () => {
    api.report.mockImplementation(async (opts: { groupBy: string[] }) => {
      const by = opts.groupBy.join(',')
      const sub = { billing: 'subscription', cost_usd: null }
      if (by === 'day') return report([item({ day: TODAY, ...sub, turns: 7, input_tokens: 600_000, output_tokens: 40_000, equivalent_usd: 12.3 })])
      if (by === 'day,harness')
        return report([item({ day: TODAY, harness: 'claude', ...sub, turns: 7, input_tokens: 600_000, output_tokens: 40_000, equivalent_usd: 12.3 })], {
          subscriptions: [{ harness: 'claude', vendor: 'anthropic', window: '5h', used_pct: 38, resets_at: new Date(NOW.getTime() + 3600_000).toISOString(), observed_at: '', session_id: 'ses_a', expired: false }],
        })
      return report([item({ session_id: 'ses_a', session_title: 'Rate-Limiter', harness: 'claude', ...sub, turns: 7, input_tokens: 600_000, output_tokens: 40_000, equivalent_usd: 4.1 })])
    })
    renderView(<UsageView />)
    const row = await screen.findByTestId('subscription-row')
    await screen.findByTestId('usage-table')
    expect(document.body.textContent).not.toContain('$')
    fireEvent.click(screen.getByRole('switch', { name: 'API-Äquivalent zeigen' }))
    expect(screen.getByRole('switch', { name: 'API-Äquivalent zeigen' }).getAttribute('aria-checked')).toBe('true')
    const eq = within(row).getByText('≈ 12,30 $ zu API-Preisen · informativ')
    expect(eq.getAttribute('title')).toBe('Was diese Nutzung zu API-Preisen gekostet hätte (Katalog 2026-10-14). Nicht abgerechnet.')
    const billing = screen.getByTestId('usage-table').querySelector('tr[data-key="ses_a"] td:last-child')!
    expect(billing.textContent!.replace(/\s/g, ' ')).toBe('Subscription · ≈ 4,10 $ informativ')
    expect(billing.querySelector('[title]')?.getAttribute('title')).toContain('Nicht abgerechnet.')
    // Die Kennzahl „Kosten“ bleibt unberührt (ADR-0034).
    expect(screen.getByTestId('usage-kpis').textContent).toContain('Kostenkeine')
    // CSV bleibt ohne Äquivalent.
    expect(toCsv('session', rowsFor('session', [item({ session_id: 'ses_a', session_title: 'X', billing: 'subscription', cost_usd: null, equivalent_usd: 4.1 })], TODAY))).not.toContain('4,1')
  })

  it('USE-004: ein abgelaufenes Abo-Fenster steht als „Kontingent zurückgesetzt“ statt als Balken', async () => {
    api.report.mockImplementation(async (opts: { groupBy: string[] }) => {
      const by = opts.groupBy.join(',')
      if (by === 'day,harness')
        return report([item({ day: TODAY, harness: 'codex', billing: 'subscription', input_tokens: 4_000_000, output_tokens: 200_000, cost_usd: null })], {
          subscriptions: [
            { harness: 'codex', vendor: 'openai', window: '5h', used_pct: 81, resets_at: new Date(NOW.getTime() - 60_000).toISOString(), observed_at: '', session_id: 'ses_a', expired: true },
          ],
          reporting: [{ harness: 'codex', sessions: 1, tokens: true, rate_limits: true }],
        })
      return report([item({ day: TODAY, billing: 'subscription', input_tokens: 1, cost_usd: null })])
    })
    renderView(<UsageView />)
    const row = await screen.findByTestId('subscription-row')
    expect(row.textContent).toContain('Kontingent zurückgesetzt – der nächste Turn meldet den neuen Stand. 4,2 Mio. Tokens in 14 Tagen.')
    expect(row.textContent).toContain('heute 4,2 Mio. Tokens')
    expect(within(row).queryByRole('meter')).toBeNull()
    expect(row.textContent).not.toContain('nicht gemeldet')
    expect(subscriptionRows([{ harness: 'codex', vendor: 'openai', window: '5h', used_pct: 81, resets_at: '', observed_at: '', session_id: 's', expired: true }], [], [], TODAY)[0]).toMatchObject({ windows: [], expired: true })
  })

  it('USE-003: ohne eigene Preise sagt die Tabelle, wo sie stehen', async () => {
    api.pricing.mockResolvedValue({ catalog_version: '2026-10-14', currency: 'USD', overrides: [], catalog: [] })
    renderView(<PricingView />)
    const overrides = await screen.findByTestId('overrides-table')
    expect(overrides.textContent).toContain('Noch keine eigenen Preise. Sie stehen in ~/.beton/config.yaml unter pricing.overrides; der erste Treffer gewinnt.')
    expect(overrides.querySelectorAll('tbody tr')).toHaveLength(1)
  })
})
