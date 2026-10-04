import { WifiOff } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Callout, KV, PageHead, Premise, Scroll, Section, Shell, Tag } from '@/app/kit/policies'

function Meter({ used, limit, marks = [], unit = 'USD' }: { used: number; limit: number; marks?: number[]; unit?: string }) {
  const pct = Math.min(100, (used / limit) * 100)
  return (
    <div className="flex items-center gap-3">
      <div className="relative h-2 flex-1 rounded-full bg-muted">
        <span className={cn('absolute inset-y-0 left-0 rounded-full', pct >= 100 ? 'bg-deny' : 'bg-foreground/70')} style={{ width: `${pct}%` }} />
        {marks.map((m) => (
          <span key={m} className="absolute -top-1 h-4 w-px bg-foreground" style={{ left: `${(m / limit) * 100}%` }} title={`Rückfrage bei ${m} ${unit}`} />
        ))}
      </div>
      <span className="w-36 text-right text-[12px] tabular-nums">
        {used.toLocaleString('de-DE', { minimumFractionDigits: 2 })} / {limit} {unit}
      </span>
    </div>
  )
}

export function PolicyBudgets({ state }: { state: string }) {
  const team = state !== 'local'
  const offline = state === 'offline' || state === 'exhausted'
  const exhausted = state === 'exhausted'
  return (
    <Shell nav="policies" connection={offline ? 'offline' : team ? 'server' : 'local'}>
      <PageHead
        title="Budgets"
        sub="Kostengrenzen aus deinen Policies. Bei Subscriptions zählt beton das Listenpreis-Äquivalent – deine Rechnung ändert sich dadurch nicht."
      >
        <div className="mt-2 flex flex-wrap gap-x-5 gap-y-1">
          {!team && <Premise>Lokal-only: Budgets werden auf diesem Rechner gezählt · keine Leases, kein Server</Premise>}
          {team && !offline && <Premise kind="team">Team-Budgets prüft der Team-Server; dieser Rechner hält eine Offline-Reserve (Lease)</Premise>}
          {offline && (
            <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
              <WifiOff className="size-3.5" /> Offline seit 2 Std. 10 Min. · Kosten werden von der Lease abgezogen und beim Reconnect abgerechnet
            </span>
          )}
        </div>
      </PageHead>
      <Scroll>
        <div className="flex gap-8 px-6 py-4">
          <div className="min-w-0 flex-1 space-y-5">
            {exhausted && (
              <F id={['POL-021', 'SYNC-007']}>
                <Callout
                  tone="wait"
                  title="Offline-Budget aufgebraucht – „Rate-Limiter für die Login-API“ wartet"
                  actions={
                    <>
                      <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Diesen Turn trotzdem erlauben</button>
                      <Btn>Session anhalten</Btn>
                    </>
                  }
                >
                  <p>
                    Die Reserve von 10,00 USD für diesen Rechner ist verbraucht. Ohne Verbindung zum Team-Server kann beton nicht nachbuchen. Die Org
                    hat „bei erschöpfter Lease nachfragen“ eingestellt.
                  </p>
                  <p className="text-muted-foreground">Was du jetzt erlaubst, wird beim Reconnect als Überzug gebucht und Admins gemeldet.</p>
                </Callout>
              </F>
            )}
            <F id="POL-011">
              <Section title="Pro Session" hint="spend_cap · Projekt shop-frontend">
                <div className="space-y-2.5">
                  {[
                    { t: 'Rate-Limiter für die Login-API', u: exhausted ? 13.4 : 10.12, l: 25 },
                    { t: 'Review: Rate-Limiter', u: 2.31, l: 25 },
                    { t: 'Nächtliches Dependency-Update (Teilbaum, 3 Child-Sessions)', u: 6.8, l: 15 },
                  ].map((s) => (
                    <div key={s.t}>
                      <div className="mb-1 text-[13px]">{s.t}</div>
                      <Meter used={s.u} limit={s.l} marks={[10, 20].filter((m) => m < s.l)} />
                    </div>
                  ))}
                </div>
              </Section>
            </F>
            <F id="POL-012">
              <Section title="Pro Tag" hint="daily_budget · Tagesgrenze 00:00 Europe/Berlin">
                <div className="space-y-2.5">
                  <div>
                    <div className="mb-1 flex items-center gap-2 text-[13px]">
                      Du (alle Sessions) <Tag mono={false}>User-Policy</Tag>
                    </div>
                    <Meter used={exhausted ? 27.5 : 24.1} limit={40} marks={[30]} />
                  </div>
                  {team && (
                    <div>
                      <div className="mb-1 flex items-center gap-2 text-[13px]">
                        Team plattform <Tag mono={false}>Org-Policy · vom Team-Server</Tag>
                      </div>
                      <Meter used={182} limit={300} marks={[250]} />
                      {offline && <p className="mt-1 text-[11px] text-muted-foreground">Stand vor 2 Std. 10 Min. – aktuell nicht abrufbar</p>}
                    </div>
                  )}
                </div>
              </Section>
            </F>
          </div>
          <aside className="w-[340px] shrink-0">
            {team ? (
              <F id={['SYNC-007', 'POL-021']}>
                <h3 className="type-wide mb-2 text-[13px] font-semibold">Offline-Reserve dieses Rechners</h3>
                <div className="rounded-md border border-border bg-card p-3">
                  <Meter used={exhausted ? 10 : offline ? 6.6 : 1.9} limit={10} />
                  <div className="mt-3">
                    <KV k="Lease">lse_01JB… · Team plattform, täglich</KV>
                    <KV k="Gewährt">10,00 USD (Hälfte des Rests, höchstens 10)</KV>
                    <KV k="Verbraucht">{exhausted ? '10,00' : offline ? '6,60' : '1,90'} USD</KV>
                    <KV k="Gültig bis">heute 24:00 (Periodenende)</KV>
                    <KV k="Zuletzt gemeldet">{offline ? 'vor 2 Std. 10 Min.' : 'vor 40 s'}</KV>
                    <KV k="Wenn aufgebraucht">nachfragen (Org-Einstellung)</KV>
                  </div>
                </div>
                <ol className="mt-3 space-y-1.5 text-[12px] text-muted-foreground">
                  <li>
                    <span className="text-foreground">1 · Reservieren:</span> Der Team-Server reserviert einen Teil des Team-Budgets für diesen Rechner.
                  </li>
                  <li>
                    <span className="text-foreground">2 · Lokal verbrauchen:</span> Kosten werden sofort abgezogen, auch ohne Netz.
                  </li>
                  <li>
                    <span className="text-foreground">3 · Melden:</span> online alle 60 s; unter 20 % Rest wird aufgestockt.
                  </li>
                  <li>
                    <span className="text-foreground">4 · Abrechnen:</span> beim Reconnect bucht der Server den Verbrauch und gibt den Rest frei.
                  </li>
                </ol>
              </F>
            ) : (
              <F id="POL-021">
                <h3 className="type-wide mb-2 text-[13px] font-semibold">Offline-Reserve</h3>
                <p className="text-[12px] text-muted-foreground">
                  Nicht nötig: Ohne Team-Server zählt beton alle Budgets lokal und vollständig. Leases gibt es nur, wenn Org- oder Team-Budgets von
                  einem Team-Server kommen.
                </p>
              </F>
            )}
          </aside>
        </div>
      </Scroll>
    </Shell>
  )
}
