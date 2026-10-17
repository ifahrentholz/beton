import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import type { PriceOverrideView, UsageItem } from '@beton/sdk'
import { client } from '@/lib/client'
import { useT } from '@/lib/i18n'
import { localDate, tzOffset } from '@/lib/usage'
import { rich } from '../kbd'

/*
 * Preise (Verbrauch › Preise) nach dem Entwurf `usage-pricing`: eigene Preise in Prüfreihenfolge,
 * der mitgelieferte Katalog und der Hinweis auf Turns ohne Preis – das Ziel des Links „ohne
 * Preis – eigenen Preis anlegen“ der Verbrauchsseite (USE-006 AC4). Nur lesend: Eigene Preise
 * stehen in der User-Konfiguration; Anlegen aus der UI folgt mit der Einstellungsseite.
 */

/** Preis wie im Entwurf, z. B. `0,40 $`; fehlt er, ein Gedankenstrich. */
export function usd(v: number | null | undefined): string {
  if (v === null || v === undefined) return '—'
  return `${v.toLocaleString('de-DE', { minimumFractionDigits: 2, maximumFractionDigits: 3 })} $`
}

function ctx(n: number | null | undefined): string {
  if (!n) return '—'
  if (n >= 1_000_000) return `${(n / 1_000_000).toLocaleString('de-DE', { maximumFractionDigits: 1 })} Mio.`
  return `${Math.round(n / 1000)} k`
}

/** Modelle mit Turns ohne Preis (aus `GET /v1/usage?group_by=harness,model`). */
export function unpricedModels(groups: UsageItem[]): { label: string; count: number }[] {
  const out = new Map<string, number>()
  for (const g of groups) {
    if (g.unpriced_count <= 0 || !g.model) continue
    const provider = (g.harness ?? '').replace(/^direct:/, '')
    const label = provider ? `${provider}/${g.model}` : g.model
    out.set(label, (out.get(label) ?? 0) + g.unpriced_count)
  }
  return [...out.entries()].map(([label, count]) => ({ label, count }))
}

function OverrideRow({ o }: { o: PriceOverrideView }) {
  return (
    <tr className="border-t border-border">
      <td className="py-2 pl-3 text-muted-foreground">{o.position}</td>
      <td className="font-mono text-[12px]">
        {o.provider ?? '*'} / {o.model ?? '*'}
      </td>
      {o.free ? (
        <td colSpan={3} className="px-2 text-right text-[12px] text-muted-foreground">
          kostenlos
        </td>
      ) : (
        <>
          <td className="px-2 text-right tabular-nums">{usd(o.input_per_mtok)}</td>
          <td className="px-2 text-right tabular-nums">{usd(o.output_per_mtok)}</td>
          <td
            className={o.cache_estimated ? 'px-2 text-right text-muted-foreground tabular-nums' : 'px-2 text-right tabular-nums'}
            title={o.cache_estimated ? 'Fehlende Cache-Preise: 0,1 × bzw. 1,25 × Eingabe' : undefined}
          >
            {usd(o.cache_read_per_mtok)} / {usd(o.cache_write_per_mtok)}
            {o.cache_estimated && <span className="text-[10px]"> geschätzt</span>}
          </td>
        </>
      )}
      <td className="px-2 text-right text-muted-foreground tabular-nums">{o.context_window ? ctx(o.context_window) : o.free ? 'aus Modell' : '—'}</td>
    </tr>
  )
}

export function PricingView() {
  const t = useT()
  const pricing = useQuery({ queryKey: ['usage', 'pricing'], queryFn: () => client.usage.pricing() })
  const now = useMemo(() => new Date(), [])
  const unpriced = useQuery({
    queryKey: ['usage', 'unpriced', localDate(now)],
    queryFn: () => client.usage.report({ tz: tzOffset(now), groupBy: ['harness', 'model'] }),
  })
  const missing = unpricedModels(unpriced.data?.groups ?? [])
  const turns = missing.reduce((s, m) => s + m.count, 0)
  const p = pricing.data
  const failed = pricing.error ?? unpriced.error

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-5xl px-4 py-6 sm:px-8">
        <div className="border-b border-border pb-4">
          <div className="text-[12px] text-muted-foreground">
            <Link to="/usage" className="hover:text-foreground">
              Verbrauch
            </Link>{' '}
            ›
          </div>
          <h1 className="type-wide text-xl font-[700]">Preise</h1>
          <p className="mt-1 max-w-2xl text-[13px] text-muted-foreground">
            Preise braucht beton nur für Sessions mit API-Key oder Gateway. Subscriptions über die offiziellen CLIs werden nicht in Geld
            umgerechnet.
          </p>
        </div>

        {failed && (
          <div className="mt-4 border border-deny/40 bg-deny-soft px-4 py-1.5 text-[12px]" role="alert">
            {failed instanceof Error ? failed.message : 'Preise nicht verfügbar'}
          </div>
        )}

        {turns > 0 && (
          <div className="chamfer mt-5 border-l-4 border-signal bg-signal-soft p-3 text-[13px]" data-testid="unpriced-hint">
            <div className="font-semibold">
              {turns} {turns === 1 ? 'Turn' : 'Turns'} ohne Preis
            </div>
            <p className="mt-0.5">
              {missing.map((m, i) => (
                <span key={m.label}>
                  {i > 0 && ', '}
                  <span className="font-mono">{m.label}</span>
                </span>
              ))}{' '}
              {missing.length === 1 ? 'steht' : 'stehen'} weder im Katalog noch in deinen eigenen Preisen. Diese Turns zählen nicht in Budgets,
              bis du einen Preis anlegst.
            </p>
          </div>
        )}

        {p && (
          <section className="mt-6" aria-label="Eigene Preise">
            <div className="flex flex-wrap items-baseline gap-3">
              <h2 className="text-[14px] font-semibold">Eigene Preise für Gateways und eigene Modelle</h2>
              <span className="ml-auto text-[12px] text-muted-foreground">
                gespeichert in <span className="font-mono">~/.beton/config.yaml</span> · erster Treffer gewinnt
              </span>
            </div>
            <div className="mt-2 overflow-x-auto rounded-md border border-border">
              <table className="w-full text-[13px]" data-testid="overrides-table">
                <thead className="bg-sunken text-[12px] text-muted-foreground">
                  <tr>
                    <th className="w-8 py-2 pl-3 text-left font-medium">#</th>
                    <th className="text-left font-medium">Gilt für</th>
                    <th className="px-2 text-right font-medium">Eingabe / 1 Mio.</th>
                    <th className="px-2 text-right font-medium">Ausgabe / 1 Mio.</th>
                    <th className="px-2 text-right font-medium">Cache lesen / schreiben</th>
                    <th className="px-2 text-right font-medium">Kontext</th>
                  </tr>
                </thead>
                <tbody className="type-narrow">
                  {p.overrides.length === 0 ? (
                    // Keine eigenen Preise (Entwurf `usage-pricing`, Zustand `no-overrides`).
                    <tr className="border-t border-border">
                      <td colSpan={6} className="px-3 py-3 font-sans text-[12px] text-muted-foreground">
                        {rich(t('pricing.noOverrides'), {
                          file: <span className="font-mono">~/.beton/config.yaml</span>,
                          key: <span className="font-mono">pricing.overrides</span>,
                        })}
                      </td>
                    </tr>
                  ) : (
                    p.overrides.map((o) => <OverrideRow key={o.position} o={o} />)
                  )}
                </tbody>
              </table>
            </div>
            <p className="mt-2 text-[12px] text-muted-foreground">Preise in USD, wie bei den Anbietern.</p>
          </section>
        )}

        {p && (
          <section className="mt-8" aria-label="Preis-Katalog">
            <div className="flex flex-wrap items-baseline gap-3">
              <h2 className="text-[14px] font-semibold">Mitgelieferter Preis-Katalog</h2>
              <span className="rounded-sm border border-border px-1.5 font-mono text-[11px]">{p.catalog_version}</span>
              <span className="ml-auto text-[12px] text-muted-foreground">kommt mit jedem beton-Release, kein Online-Abruf</span>
            </div>
            <div className="mt-2 overflow-x-auto rounded-md border border-border">
              <table className="w-full text-[13px]" data-testid="catalog-table">
                <thead className="bg-sunken text-[12px] text-muted-foreground">
                  <tr>
                    <th className="py-2 pl-3 text-left font-medium">Modell</th>
                    <th className="text-left font-medium">Anbieter</th>
                    <th className="px-2 text-right font-medium">Kontext</th>
                    <th className="px-2 text-right font-medium">Eingabe</th>
                    <th className="px-2 text-right font-medium">Ausgabe</th>
                    <th className="px-2 text-right font-medium">Cache lesen</th>
                    <th className="px-2 pr-3 text-right font-medium">Cache schreiben</th>
                  </tr>
                </thead>
                <tbody className="type-narrow">
                  {p.catalog.map((c) => (
                    <tr key={c.model} className="border-t border-border">
                      <td className="py-1.5 pl-3 font-mono text-[12px]">{c.model}</td>
                      <td className="text-[12px] text-muted-foreground">{c.provider}</td>
                      <td className="px-2 text-right tabular-nums">{ctx(c.context_window)}</td>
                      <td className="px-2 text-right tabular-nums">{usd(c.input_per_mtok)}</td>
                      <td className="px-2 text-right tabular-nums">{usd(c.output_per_mtok)}</td>
                      <td className="px-2 text-right tabular-nums">{usd(c.cache_read_per_mtok)}</td>
                      <td className="px-2 pr-3 text-right tabular-nums">{usd(c.cache_write_per_mtok)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="mt-2 text-[12px] text-muted-foreground">
              Jeder berechnete Turn merkt sich die Katalogversion. Nach einem Update werden alte Kosten nicht neu bewertet.
            </p>
          </section>
        )}
      </div>
    </div>
  )
}
