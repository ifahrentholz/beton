import { useState, type ReactNode } from 'react'
import { Download, Info, Minimize2, Settings2 } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, StatusMark, VoiceDot, voiceVar } from '@/app/harness'
import { Composer } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, TurnFooter, UserMessage } from '@/app/stream'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { harnesses, type HarnessId } from '@/mock/data'

/* ───────────────────────── Beispieldaten ───────────────────────── */

const DAYS = ['So 20.', 'Mo 21.', 'Di 22.', 'Mi 23.', 'Do 24.', 'Fr 25.', 'Sa 26.', 'So 27.', 'Mo 28.', 'Di 29.', 'Mi 30.', 'Do 1.', 'Fr 2.', 'Sa 3.']
/** Tausend Tokens pro Tag und Harness. */
const SERIES: Record<HarnessId, number[]> = {
  claude: [0, 1430, 960, 1720, 2210, 1980, 120, 0, 1460, 2390, 1810, 2840, 1960, 640],
  codex: [0, 420, 310, 660, 540, 880, 0, 0, 720, 410, 530, 930, 610, 0],
  gemini: [0, 0, 120, 0, 90, 0, 0, 0, 0, 210, 0, 0, 0, 0],
  ollama: [0, 0, 0, 140, 0, 0, 0, 0, 380, 0, 0, 120, 0, 0],
}
/** Euro pro Tag aus API-Key-/Gateway-Sessions (nur Zustand „mit API-Key“). */
const COST_EUR = [0, 0, 0.84, 0, 1.92, 0.31, 0, 0, 2.47, 0.62, 0, 3.18, 2.71, 0.59]

const fmtTok = (k: number) => (k >= 1000 ? `${(k / 1000).toLocaleString('de-DE', { maximumFractionDigits: 1 })} Mio.` : `${k.toLocaleString('de-DE')} k`)
const eur = (v: number) => v.toLocaleString('de-DE', { style: 'currency', currency: 'EUR' })

type Dim = 'day' | 'session' | 'project' | 'harness' | 'model' | 'auth' | 'user' | 'team'
const DIMS: { id: Dim; label: string; central?: boolean }[] = [
  { id: 'day', label: 'Tag' },
  { id: 'session', label: 'Session' },
  { id: 'project', label: 'Projekt' },
  { id: 'harness', label: 'Harness' },
  { id: 'model', label: 'Modell' },
  { id: 'auth', label: 'Anmeldung' },
  { id: 'user', label: 'Person', central: true },
  { id: 'team', label: 'Team', central: true },
]

type UsageRow = { name: ReactNode; sub?: ReactNode; turns: number; input: number; output: number; cache: number; billing: ReactNode }

const SUB = <span className="text-muted-foreground">Subscription</span>
const LOCAL = <span className="text-muted-foreground">lokal, kostenlos</span>

function rowsFor(dim: Dim, mixed: boolean): UsageRow[] {
  const api = (v: number, src: string) => (
    <span className="tabular-nums">
      {eur(v)} <span className="text-[11px] text-muted-foreground">{src}</span>
    </span>
  )
  switch (dim) {
    case 'session':
      return [
        { name: 'Rate-Limiter für die Login-API', sub: <HarnessBadge id="claude" model="claude-opus-5-5" />, turns: 14, input: 2210, output: 186, cache: 1840, billing: SUB },
        { name: 'Review: Rate-Limiter', sub: <HarnessBadge id="codex" model="gpt-5.3-codex" />, turns: 6, input: 640, output: 72, cache: 410, billing: SUB },
        ...(mixed
          ? [{ name: 'CI: Release-Notes erzeugen', sub: <span className="text-[12px] text-muted-foreground">Claude Code · API-Key (Anthropic)</span>, turns: 3, input: 310, output: 41, cache: 0, billing: api(2.71, 'Katalog') }]
          : []),
        { name: 'Event-Log: seq lückenlos halten', sub: <HarnessBadge id="codex" model="gpt-5.3-codex" />, turns: 21, input: 1980, output: 204, cache: 1620, billing: SUB },
        { name: 'Nächtliches Dependency-Update', sub: <HarnessBadge id="claude" model="claude-sonnet-5-5" />, turns: 9, input: 860, output: 52, cache: 700, billing: SUB },
        ...(mixed
          ? [{ name: 'Lasttest auswerten', sub: <span className="text-[12px] text-muted-foreground">Gateway litellm-intern · llama-3.3-70b</span>, turns: 4, input: 480, output: 66, cache: 0, billing: api(0.24, 'eigener Preis') }]
          : []),
        { name: 'Lokales Modell testen', sub: <HarnessBadge id="ollama" model="qwen3-coder:30b" />, turns: 7, input: 520, output: 120, cache: 0, billing: LOCAL },
        { name: 'Terraform-Plan erklären', sub: <HarnessBadge id="gemini" model="gemini-3-pro" />, turns: 2, input: 190, output: 20, cache: 0, billing: <span className="text-muted-foreground">Kontingent nicht gemeldet</span> },
      ]
    case 'project':
      return [
        { name: 'shop-frontend', sub: <span className="font-mono text-[11px] text-muted-foreground">~/code/shop-frontend</span>, turns: 48, input: 9840, output: 812, cache: 7210, billing: mixed ? api(0.24, '1 Session') : SUB },
        { name: 'beton', sub: <span className="font-mono text-[11px] text-muted-foreground">~/Develop/ai/beton</span>, turns: 37, input: 6120, output: 590, cache: 4980, billing: mixed ? api(2.71, '1 Session') : SUB },
        { name: 'infra', sub: <span className="font-mono text-[11px] text-muted-foreground">~/code/infra</span>, turns: 12, input: 1460, output: 102, cache: 980, billing: SUB },
      ]
    case 'harness':
      return (['claude', 'codex', 'gemini', 'ollama'] as HarnessId[]).map((h) => ({
        name: <HarnessBadge id={h} />,
        sub: <span className="text-[11px] text-muted-foreground">{harnesses[h].auth}</span>,
        turns: { claude: 61, codex: 27, gemini: 4, ollama: 7 }[h],
        input: { claude: 11980, codex: 4210, gemini: 380, ollama: 520 }[h],
        output: { claude: 1020, codex: 380, gemini: 40, ollama: 120 }[h],
        cache: { claude: 9840, codex: 3110, gemini: 0, ollama: 0 }[h],
        billing: h === 'ollama' ? LOCAL : h === 'gemini' ? <span className="text-muted-foreground">nicht gemeldet</span> : SUB,
      }))
    case 'model':
      return [
        { name: <span className="font-mono text-[12px]">claude-opus-5-5</span>, turns: 38, input: 8420, output: 760, cache: 7110, billing: SUB },
        { name: <span className="font-mono text-[12px]">claude-sonnet-5-5</span>, turns: 23, input: 3560, output: 260, cache: 2730, billing: mixed ? api(2.71, 'Katalog') : SUB },
        { name: <span className="font-mono text-[12px]">gpt-5.3-codex</span>, turns: 27, input: 4210, output: 380, cache: 3110, billing: SUB },
        { name: <span className="font-mono text-[12px]">qwen3-coder:30b</span>, turns: 7, input: 520, output: 120, cache: 0, billing: LOCAL },
        ...(mixed
          ? [
              {
                name: <span className="font-mono text-[12px]">mistral-large-2511</span>,
                sub: <span className="text-[11px] text-muted-foreground">Gateway openrouter</span>,
                turns: 2,
                input: 140,
                output: 18,
                cache: 0,
                billing: (
                  <a className="text-[12px] underline underline-offset-2">
                    ohne Preis – eigenen Preis anlegen
                  </a>
                ),
              },
            ]
          : []),
      ]
    case 'auth':
      return [
        { name: 'Subscription über CLI-Login', sub: <span className="text-[11px] text-muted-foreground">Claude Max, ChatGPT Pro, Google-Konto</span>, turns: 92, input: 16570, output: 1440, cache: 12950, billing: SUB },
        { name: 'Lokal', sub: <span className="text-[11px] text-muted-foreground">Ollama auf diesem Rechner</span>, turns: 7, input: 520, output: 120, cache: 0, billing: LOCAL },
        ...(mixed
          ? [
              { name: 'API-Key', sub: <span className="text-[11px] text-muted-foreground">Anthropic, im Schlüsselbund</span>, turns: 3, input: 310, output: 41, cache: 0, billing: api(2.71, 'Katalog') },
              { name: 'Gateway', sub: <span className="text-[11px] text-muted-foreground">litellm-intern, openrouter</span>, turns: 6, input: 620, output: 84, cache: 0, billing: api(0.24, '1 ohne Preis') },
            ]
          : []),
      ]
    case 'user':
      return [
        { name: 'Ingo Fahrentholz', sub: <span className="text-[11px] text-muted-foreground">Plattform</span>, turns: 99, input: 17090, output: 1560, cache: 12950, billing: SUB },
        { name: 'Mara Lindqvist', sub: <span className="text-[11px] text-muted-foreground">Shop</span>, turns: 64, input: 11240, output: 980, cache: 8820, billing: SUB },
        { name: 'Jonas Becker', sub: <span className="text-[11px] text-muted-foreground">Shop</span>, turns: 31, input: 4120, output: 410, cache: 2210, billing: api(18.4, 'Katalog') },
      ]
    case 'team':
      return [
        { name: 'Shop', sub: <span className="text-[11px] text-muted-foreground">5 Personen</span>, turns: 212, input: 38410, output: 3120, cache: 27400, billing: api(18.4, 'API-Keys') },
        { name: 'Plattform', sub: <span className="text-[11px] text-muted-foreground">3 Personen</span>, turns: 141, input: 22860, output: 2040, cache: 17730, billing: SUB },
      ]
    default:
      return DAYS.slice()
        .reverse()
        .slice(0, 7)
        .map((d, i) => {
          const idx = 13 - i
          const tot = SERIES.claude[idx] + SERIES.codex[idx] + SERIES.gemini[idx] + SERIES.ollama[idx]
          return {
            name: d === 'Sa 3.' ? 'Heute, Sa 3.10.' : `${d}${idx < 11 ? '09.' : '10.'}`,
            turns: Math.round(tot / 95),
            input: Math.round(tot * 0.86),
            output: Math.round(tot * 0.08),
            cache: Math.round(tot * 0.64),
            billing: mixed && COST_EUR[idx] > 0 ? api(COST_EUR[idx], 'API & Gateway') : SUB,
          }
        })
  }
}

/* ───────────────────────── Bausteine ───────────────────────── */

function Segmented({ value, options, label, onChange }: { value: string; options: { id: string; label: ReactNode; disabled?: boolean; title?: string }[]; label: string; onChange?: (id: string) => void }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex rounded-md border border-border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.id}
          role="radio"
          aria-checked={o.id === value}
          disabled={o.disabled}
          title={o.title}
          onClick={() => onChange?.(o.id)}
          className={cn(
            'rounded-[3px] px-2.5 py-1 text-[12px] whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-45',
            o.id === value ? 'bg-foreground text-background' : 'text-muted-foreground enabled:hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

/** Ruhiges, gestapeltes Säulendiagramm in CSS (keine Chart-Bibliothek). */
function StackedBars({ mode }: { mode: 'tokens' | 'cost' }) {
  const order: HarnessId[] = ['claude', 'codex', 'gemini', 'ollama']
  const totals = DAYS.map((_, i) => order.reduce((s, h) => s + SERIES[h][i], 0))
  const max = mode === 'tokens' ? 4000 : 4
  const ticks = mode === 'tokens' ? [0, 1000, 2000, 3000, 4000] : [0, 1, 2, 3, 4]
  return (
    <div className="flex gap-2">
      <div className="relative h-44 w-14 shrink-0 text-right text-[10px] text-muted-foreground tabular-nums">
        {ticks.map((t) => (
          <span key={t} className="absolute right-1 -translate-y-1/2" style={{ top: `${100 - (t / max) * 100}%` }}>
            {mode === 'tokens' ? (t === 0 ? '0' : `${t / 1000} Mio.`) : eur(t).replace(',00', '')}
          </span>
        ))}
      </div>
      <div className="min-w-0 flex-1">
        <div className="relative h-44 border-b border-foreground/40">
          {ticks.slice(1).map((t) => (
            <span key={t} className="absolute inset-x-0 h-px bg-border" style={{ top: `${100 - (t / max) * 100}%` }} />
          ))}
          <div className="absolute inset-0 flex items-end gap-[6px] px-1">
            {DAYS.map((d, i) => (
              <div key={d} className="flex h-full flex-1 flex-col-reverse" title={`${d}: ${mode === 'tokens' ? fmtTok(totals[i]) + ' Tokens' : eur(COST_EUR[i])}`}>
                {mode === 'tokens' ? (
                  order.map((h) =>
                    SERIES[h][i] ? (
                      <span
                        key={h}
                        className="block w-full border-t border-background first:rounded-b-none last:rounded-t-[2px]"
                        style={{ height: `${(SERIES[h][i] / max) * 100}%`, background: voiceVar[harnesses[h].voice] }}
                      />
                    ) : null,
                  )
                ) : COST_EUR[i] ? (
                  <span className="block w-full rounded-t-[2px] bg-foreground/75" style={{ height: `${(COST_EUR[i] / max) * 100}%` }} />
                ) : null}
                {i === 13 && <span className="sr-only">heute</span>}
              </div>
            ))}
          </div>
        </div>
        <div className="mt-1 flex gap-[6px] px-1 text-[10px] text-muted-foreground">
          {DAYS.map((d, i) => (
            <span key={d} className={cn('flex-1 text-center whitespace-nowrap', i === 13 && 'font-semibold text-foreground')}>
              {d}
            </span>
          ))}
        </div>
      </div>
    </div>
  )
}

/** Kontingent-Balken eines Rate-Limit-Fensters (USE-004). */
function QuotaBar({ label, pct, reset, warn }: { label: string; pct: number; reset: string; warn?: boolean }) {
  const high = pct >= 90
  const full = pct >= 100
  return (
    <div className="grid grid-cols-[120px_1fr_auto] items-center gap-3 py-1">
      <span className="text-[12px] text-muted-foreground">{label}</span>
      <span className="relative h-2 overflow-hidden rounded-full bg-muted" role="meter" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
        <span className={cn('absolute inset-y-0 left-0', full ? 'bg-deny' : high ? 'bg-warn' : 'bg-foreground/70')} style={{ width: `${pct}%` }} />
        {[25, 50, 75].map((m) => (
          <span key={m} className="absolute inset-y-0 w-px bg-background/70" style={{ left: `${m}%` }} />
        ))}
      </span>
      <span className="text-[12px] whitespace-nowrap tabular-nums">
        <strong className={cn('font-semibold', full ? 'text-deny' : high && 'text-warn')}>{pct} %</strong>
        <span className="text-muted-foreground"> genutzt · {reset}</span>
        {high && warn && <span className={cn('ml-2', full ? 'text-deny' : 'text-warn')}>{full ? 'aufgebraucht' : 'fast aufgebraucht'}</span>}
      </span>
    </div>
  )
}

function Kpi({ label, value, sub }: { label: string; value: ReactNode; sub?: ReactNode }) {
  return (
    <div className="min-w-0 border-l border-border pl-4 first:border-l-0 first:pl-0">
      <div className="text-[12px] text-muted-foreground">{label}</div>
      <div className="type-wide mt-0.5 text-[22px] font-[650] tabular-nums">{value}</div>
      {sub && <div className="text-[11px] text-muted-foreground">{sub}</div>}
    </div>
  )
}

/* ───────────────────────── Verbrauchsseite ───────────────────────── */

function UsageOverview({ state }: { state: string }) {
  const mixed = state === 'mixed' || state === 'team'
  const team = state === 'team'
  const high = state === 'quota-high'
  const [dim, setDim] = useState<Dim>(team ? 'user' : 'session')
  const [stack, setStack] = useState<'tokens' | 'cost'>('tokens')
  const effectiveDim = !team && DIMS.find((d) => d.id === dim)?.central ? 'session' : dim
  const rows = rowsFor(effectiveDim, mixed)

  if (state === 'empty') {
    return (
      <AppLayout nav="usage" sessionList={false}>
        <div className="concrete-grain flex flex-1 flex-col items-center justify-center gap-2 p-8 text-center">
          <p className="type-wide text-xl font-[700]">Noch kein Verbrauch</p>
          <p className="max-w-md text-sm text-muted-foreground">
            Sobald eine Session läuft, siehst du hier Tokens und die Kontingente deiner Subscriptions. Euro-Kosten erscheinen nur,
            wenn du einen API-Key oder ein Gateway nutzt.
          </p>
        </div>
      </AppLayout>
    )
  }

  return (
    <AppLayout nav="usage" sessionList={false} connection={team ? 'server' : 'local'}>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-6xl px-8 py-6">
          <div className="flex flex-wrap items-center gap-3 border-b border-border pb-4">
            <h1 className="type-wide mr-auto text-xl font-[700]">Verbrauch</h1>
            <Segmented
              label="Zeitraum"
              value="14d"
              options={[
                { id: 'today', label: 'Heute' },
                { id: '7d', label: '7 Tage' },
                { id: '14d', label: '14 Tage' },
                { id: 'month', label: 'Oktober' },
                { id: 'custom', label: 'Zeitraum …' },
              ]}
            />
            <button className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border px-3 text-[13px] hover:bg-accent" title="verbrauch_2026-09-20_2026-10-03_nach-session.csv">
              <Download className="size-3.5" /> CSV exportieren
            </button>
          </div>

          <F id={['USE-006', 'USE-005']} className="mt-5 grid grid-cols-4 gap-4">
            <Kpi label="Heute" value={fmtTok(640)} sub="Tokens · 7 Turns" />
            <Kpi label="Letzte 7 Tage" value={fmtTok(15_010)} sub="Tokens · 99 Turns" />
            <Kpi label="Oktober" value={fmtTok(7_100)} sub="Tokens · 3 Tage" />
            {mixed ? (
              <Kpi label="Kosten 7 Tage" value={eur(team ? 18.4 : 9.07)} sub="nur API-Key & Gateway" />
            ) : (
              <Kpi label="Kosten" value="keine" sub="alle Sessions über Subscriptions oder lokal" />
            )}
          </F>

          <F id="USE-004" className="mt-6">
            <div className="flex items-baseline gap-3">
              <h2 className="text-[14px] font-semibold">Subscriptions</h2>
              <span className="text-[12px] text-muted-foreground">Kontingente, wie die CLIs sie melden – nicht in Euro</span>
            </div>
            <div className="mt-2 divide-y divide-border rounded-md border border-border">
              <div className={cn('grid grid-cols-[230px_1fr] gap-4 px-4 py-3', high && 'bg-warn-soft/60')}>
                <div>
                  <HarnessBadge id="claude" />
                  <div className="mt-0.5 text-[11px] text-muted-foreground">Claude Max · angemeldet über claude-CLI</div>
                  <div className="mt-1 text-[11px] text-muted-foreground tabular-nums">heute 640 k Tokens</div>
                </div>
                <div>
                  <QuotaBar label="5-Stunden-Fenster" pct={high ? 92 : 38} reset={high ? 'setzt in 41 Min. zurück (14:46)' : 'setzt in 2 Std. 14 Min. zurück (16:19)'} warn />
                  <QuotaBar label="Wochenlimit" pct={high ? 78 : 61} reset="setzt Mo. 09:00 zurück" />
                  {high && (
                    <p className="mt-1 text-[12px]">
                      Neue Turns mit Claude Code können gleich abgelehnt werden. Du kannst mit Codex weiterarbeiten (Session forken) oder bis
                      14:46 warten. Ein Hinweis liegt in der Inbox.
                    </p>
                  )}
                </div>
              </div>
              <div className="grid grid-cols-[230px_1fr] gap-4 px-4 py-3">
                <div>
                  <HarnessBadge id="codex" />
                  <div className="mt-0.5 text-[11px] text-muted-foreground">ChatGPT Pro · angemeldet über codex-CLI</div>
                  <div className="mt-1 text-[11px] text-muted-foreground tabular-nums">heute 0 Tokens</div>
                </div>
                <div>
                  <QuotaBar label="5-Stunden-Fenster" pct={12} reset="setzt in 4 Std. 2 Min. zurück (18:07)" />
                  <QuotaBar label="Wochenlimit" pct={23} reset="setzt Do. 11:30 zurück" />
                </div>
              </div>
              <div className="grid grid-cols-[230px_1fr] items-center gap-4 px-4 py-3">
                <div>
                  <HarnessBadge id="gemini" />
                  <div className="mt-0.5 text-[11px] text-muted-foreground">Google-Konto · angemeldet über gemini-CLI</div>
                </div>
                <div className="text-[12px] text-muted-foreground">Kontingent nicht gemeldet – die Gemini CLI liefert keine Rate-Limit-Daten. 380 k Tokens in 14 Tagen.</div>
              </div>
              <div className="grid grid-cols-[230px_1fr] items-center gap-4 px-4 py-3">
                <div>
                  <HarnessBadge id="ollama" />
                  <div className="mt-0.5 text-[11px] text-muted-foreground">läuft auf diesem Rechner</div>
                </div>
                <div className="text-[12px] text-muted-foreground">Kein Kontingent, keine Kosten. 520 k Tokens in 14 Tagen.</div>
              </div>
            </div>
            <label className="mt-2 flex items-center gap-2 text-[12px] text-muted-foreground">
              <Switch size="sm" aria-label="API-Äquivalent zeigen" />
              API-Äquivalent zeigen (rein informativ: was die Subscription-Nutzung zu API-Preisen gekostet hätte)
            </label>
          </F>

          <F id="USE-006" className="mt-7">
            <div className="flex flex-wrap items-center gap-3">
              <h2 className="text-[14px] font-semibold">Verlauf</h2>
              <Segmented
                label="Größe"
                value={stack}
                onChange={(v) => setStack(v as 'tokens' | 'cost')}
                options={[
                  { id: 'tokens', label: 'Tokens nach Harness' },
                  { id: 'cost', label: 'Kosten (API & Gateway)', disabled: !mixed, title: mixed ? undefined : 'Keine Sessions mit API-Key oder Gateway' },
                ]}
              />
              <div className="ml-auto flex gap-4 text-[12px]">
                {(['claude', 'codex', 'gemini', 'ollama'] as HarnessId[]).map((h) => (
                  <span key={h} className="inline-flex items-center gap-1.5">
                    <VoiceDot voice={harnesses[h].voice} className="size-2.5 rounded-[2px]" />
                    {harnesses[h].name}
                  </span>
                ))}
              </div>
            </div>
            <div className="mt-3">
              <StackedBars mode={stack} />
            </div>
          </F>

          <F id={['USE-005', 'USE-001']} className="mt-7">
            <div className="flex flex-wrap items-center gap-3">
              <h2 className="text-[14px] font-semibold">Aufschlüsselung</h2>
              <Segmented
                label="Gruppieren nach"
                value={effectiveDim}
                onChange={(v) => setDim(v as Dim)}
                options={DIMS.map((d) => ({
                  id: d.id,
                  label: d.label,
                  disabled: d.central && !team,
                  title: d.central && !team ? 'Nur mit Team-Server (ab M4); lokal gibt es nur dich' : undefined,
                }))}
              />
              {team && <span className="text-[12px] text-muted-foreground">Du siehst als Team-Admin die Teams Shop und Plattform.</span>}
            </div>
            <div className="mt-2 overflow-hidden rounded-md border border-border">
              <table className="w-full text-[13px]">
                <thead className="bg-sunken text-[12px] text-muted-foreground">
                  <tr>
                    <th className="py-2 pl-3 text-left font-medium">{DIMS.find((d) => d.id === effectiveDim)?.label}</th>
                    <th className="px-2 text-right font-medium">Turns</th>
                    <th className="px-2 text-right font-medium">Eingabe</th>
                    <th className="px-2 text-right font-medium">Ausgabe</th>
                    <th className="px-2 text-right font-medium">Cache gelesen</th>
                    <th className="py-2 pr-3 pl-6 text-left font-medium">Abrechnung</th>
                  </tr>
                </thead>
                <tbody className="type-narrow">
                  {rows.map((r, i) => (
                    <tr key={i} className={cn('border-t border-border align-top', effectiveDim === 'session' && 'cursor-pointer hover:bg-accent/50')}>
                      <td className="py-2 pl-3">
                        <div className="font-sans">{r.name}</div>
                        {r.sub && <div className="mt-0.5">{r.sub}</div>}
                      </td>
                      <td className="px-2 py-2 text-right tabular-nums">{r.turns}</td>
                      <td className="px-2 py-2 text-right tabular-nums">{fmtTok(r.input)}</td>
                      <td className="px-2 py-2 text-right tabular-nums">{fmtTok(r.output)}</td>
                      <td className="px-2 py-2 text-right tabular-nums">{fmtTok(r.cache)}</td>
                      <td className="py-2 pr-3 pl-6 text-[12px]">{r.billing}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </F>

          <F id="USE-002" className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-1 border-t border-border pt-3 text-[11px] text-muted-foreground">
            <span className="inline-flex items-center gap-1">
              <Info className="size-3" /> Preise aus dem mitgelieferten Katalog <span className="font-mono">2026-10-01</span> (mit beton 0.9.2, kein
              Online-Abruf)
            </span>
            <span>Anzeige in Euro zum festen Kurs 1 USD = 0,92 € (in den Einstellungen änderbar)</span>
            <a className="underline underline-offset-2">Preise & eigene Preise</a>
          </F>
        </div>
      </div>
    </AppLayout>
  )
}

/* ───────────────────────── Preise (USE-002, USE-003) ───────────────────────── */

const CATALOG = [
  { model: 'claude-opus-5-5', provider: 'anthropic', ctx: '200 k', in: 15, out: 75, cr: 1.5, cw: 18.75 },
  { model: 'claude-sonnet-5-5', provider: 'anthropic', ctx: '200 k', in: 3, out: 15, cr: 0.3, cw: 3.75 },
  { model: 'claude-haiku-4-5', provider: 'anthropic', ctx: '200 k', in: 1, out: 5, cr: 0.1, cw: 1.25 },
  { model: 'gpt-5.3-codex', provider: 'openai', ctx: '400 k', in: 1.25, out: 10, cr: 0.125, cw: null },
  { model: 'gemini-3-pro', provider: 'google', ctx: '1 Mio.', in: 2, out: 12, cr: 0.2, cw: null },
]
const usd = (v: number | null) => (v === null ? '—' : `${v.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 3 })} $`)

function UsagePricing({ state }: { state: string }) {
  return (
    <AppLayout nav="usage" sessionList={false}>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-5xl px-8 py-6">
          <div className="border-b border-border pb-4">
            <div className="text-[12px] text-muted-foreground">Verbrauch ›</div>
            <h1 className="type-wide text-xl font-[700]">Preise</h1>
            <p className="mt-1 max-w-2xl text-[13px] text-muted-foreground">
              Preise braucht beton nur für Sessions mit API-Key oder Gateway. Subscriptions über die offiziellen CLIs werden nicht in
              Geld umgerechnet.
            </p>
          </div>

          {state === 'unpriced' && (
            <div className="chamfer mt-5 border-l-4 border-signal bg-signal-soft p-3 text-[13px]">
              <div className="font-semibold">2 Turns ohne Preis</div>
              <p className="mt-0.5">
                <span className="font-mono">openrouter/mistral-large-2511</span> steht weder im Katalog noch in deinen eigenen Preisen. Diese Turns
                zählen nicht in Budgets, bis du einen Preis anlegst.
              </p>
              <button className="chamfer-sm mt-2 bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Preis für mistral-large-2511 anlegen</button>
            </div>
          )}

          <F id="USE-003" className="mt-6">
            <div className="flex items-baseline gap-3">
              <h2 className="text-[14px] font-semibold">Eigene Preise für Gateways und eigene Modelle</h2>
              <span className="ml-auto text-[12px] text-muted-foreground">
                gespeichert in <span className="font-mono">~/.beton/config.yaml</span> · erster Treffer gewinnt
              </span>
            </div>
            <div className="mt-2 overflow-hidden rounded-md border border-border">
              <table className="w-full text-[13px]">
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
                  <tr className="border-t border-border">
                    <td className="py-2 pl-3 text-muted-foreground">1</td>
                    <td className="font-mono text-[12px]">litellm-intern / llama-3.3-70b*</td>
                    <td className="px-2 text-right tabular-nums">0,40 $</td>
                    <td className="px-2 text-right tabular-nums">0,80 $</td>
                    <td className="px-2 text-right tabular-nums">0,04 $ / 0,50 $</td>
                    <td className="px-2 text-right tabular-nums">131 k</td>
                  </tr>
                  <tr className="border-t border-border">
                    <td className="py-2 pl-3 text-muted-foreground">2</td>
                    <td className="font-mono text-[12px]">ollama-local / *</td>
                    <td colSpan={3} className="px-2 text-right text-[12px] text-muted-foreground">
                      kostenlos
                    </td>
                    <td className="px-2 text-right text-muted-foreground">aus Modell</td>
                  </tr>
                  <tr className="border-t border-border">
                    <td className="py-2 pl-3 text-muted-foreground">3</td>
                    <td className="font-mono text-[12px]">openrouter / *</td>
                    <td className="px-2 text-right tabular-nums">2,00 $</td>
                    <td className="px-2 text-right tabular-nums">6,00 $</td>
                    <td className="px-2 text-right text-muted-foreground tabular-nums" title="Fehlende Cache-Preise: 0,1 × bzw. 1,25 × Eingabe">
                      0,20 $ / 2,50 $ <span className="text-[10px]">geschätzt</span>
                    </td>
                    <td className="px-2 text-right text-muted-foreground">—</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <div className="mt-2 flex items-center gap-2">
              <button className="inline-flex h-8 items-center rounded-md border border-border px-3 text-[13px] hover:bg-accent">Eigenen Preis hinzufügen</button>
              <button className="inline-flex h-8 items-center rounded-md border border-border px-3 text-[13px] hover:bg-accent">Herleitung für ein Modell anzeigen …</button>
              <span className="ml-2 text-[12px] text-muted-foreground">Preise in USD, wie bei den Anbietern; Anzeige in Euro zum festen Kurs.</span>
            </div>

            {state === 'explain' && (
              <div className="mt-3 rounded-md border border-border bg-card p-3 text-[13px]">
                <div className="font-medium">
                  Herleitung für <span className="font-mono">litellm-intern/llama-3.3-70b-instruct</span>
                </div>
                <ol className="mt-2 space-y-1 text-[12px]">
                  <li className="flex gap-2">
                    <span className="w-4 text-right text-muted-foreground">1.</span>
                    <span>
                      Eigener Preis #1 passt (<span className="font-mono">llama-3.3-70b*</span>) → <strong>wird verwendet</strong>
                    </span>
                  </li>
                  <li className="flex gap-2 text-muted-foreground">
                    <span className="w-4 text-right">2.</span>
                    <span>Vom Harness gemeldete Kosten – nicht geprüft</span>
                  </li>
                  <li className="flex gap-2 text-muted-foreground">
                    <span className="w-4 text-right">3.</span>
                    <span>Katalog 2026-10-01 – kein Eintrag</span>
                  </li>
                </ol>
                <div className="mt-2 font-mono text-[12px]">1.000.000 Eingabe-Tokens × 0,40 $ = 0,40 $ (≈ 0,37 €) · Quelle: eigener Preis</div>
              </div>
            )}
          </F>

          <F id="USE-002" className="mt-8">
            <div className="flex items-baseline gap-3">
              <h2 className="text-[14px] font-semibold">Mitgelieferter Preis-Katalog</h2>
              <span className="rounded-sm border border-border px-1.5 font-mono text-[11px]">2026-10-01</span>
              <span className="ml-auto text-[12px] text-muted-foreground">kommt mit jedem beton-Release, kein Online-Abruf</span>
            </div>
            <div className="mt-2 overflow-hidden rounded-md border border-border">
              <table className="w-full text-[13px]">
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
                  {CATALOG.map((c) => (
                    <tr key={c.model} className="border-t border-border">
                      <td className="py-1.5 pl-3 font-mono text-[12px]">{c.model}</td>
                      <td className="text-[12px] text-muted-foreground">{c.provider}</td>
                      <td className="px-2 text-right tabular-nums">{c.ctx}</td>
                      <td className="px-2 text-right tabular-nums">{usd(c.in)}</td>
                      <td className="px-2 text-right tabular-nums">{usd(c.out)}</td>
                      <td className="px-2 text-right tabular-nums">{usd(c.cr)}</td>
                      <td className="px-2 pr-3 text-right tabular-nums">{usd(c.cw)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="mt-2 text-[12px] text-muted-foreground">
              Jeder berechnete Turn merkt sich die Katalogversion. Nach einem Update werden alte Kosten nicht neu bewertet. Werte
              illustrativ.
            </p>
          </F>
        </div>
      </div>
    </AppLayout>
  )
}

/* ───────────────────────── Kontext & Compaction (USE-008, USE-009) ───────────────────────── */

const CTX: Record<string, { used: number; window: number | null }> = {
  normal: { used: 84_200, window: 200_000 },
  warn: { used: 150_000, window: 200_000 },
  critical: { used: 186_400, window: 200_000 },
  compacting: { used: 186_400, window: 200_000 },
  compacted: { used: 38_200, window: 200_000 },
  auto: { used: 150_000, window: 200_000 },
  'no-window': { used: 150_000, window: null },
  unsupported: { used: 61_000, window: 131_072 },
}

function ContextRing({ used, window }: { used: number; window: number | null }) {
  const pct = window ? Math.round((used / window) * 100) : null
  const r = 9
  const c = 2 * Math.PI * r
  const level = pct === null ? 'plain' : pct >= 95 ? 'full' : pct >= 85 ? 'high' : pct >= 70 ? 'warn' : 'ok'
  const label = { plain: '', ok: '', warn: 'wird knapp', high: 'fast voll', full: 'voll' }[level]
  return (
    <span
      className="inline-flex items-center gap-2"
      title={window ? `${used.toLocaleString('de-DE')} / ${window.toLocaleString('de-DE')} Tokens · Quelle: Harness` : `${used.toLocaleString('de-DE')} Tokens · Fenstergröße unbekannt`}
    >
      {pct !== null && (
        <svg width="24" height="24" viewBox="0 0 24 24" aria-hidden className="-rotate-90">
          <circle cx="12" cy="12" r={r} fill="none" stroke="var(--muted)" strokeWidth="3" />
          <circle
            cx="12"
            cy="12"
            r={r}
            fill="none"
            stroke={level === 'full' ? 'var(--deny)' : level === 'warn' || level === 'high' ? 'var(--warn)' : 'var(--muted-foreground)'}
            strokeWidth={level === 'ok' ? 3 : 4}
            strokeDasharray={`${(c * pct) / 100} ${c}`}
          />
        </svg>
      )}
      <span className={cn('text-[12px] tabular-nums', level === 'full' && 'font-semibold text-deny', (level === 'warn' || level === 'high') && 'font-semibold text-warn')}>
        {pct !== null ? `${pct} % Kontext` : `${Math.round(used / 1000)} k Tokens`}
      </span>
      {label && <span className={cn('text-[11px]', level === 'full' ? 'text-deny' : 'text-warn')}>· {label}</span>}
    </span>
  )
}

function UsageContext({ state }: { state: string }) {
  const ctx = CTX[state] ?? CTX.normal
  const unsupported = state === 'unsupported'
  const harness: HarnessId = unsupported ? 'ollama' : 'claude'
  return (
    <AppLayout activeSession={unsupported ? 'ses_6h2f' : 'ses_7f3k'}>
      <div className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
        <StatusMark status={state === 'compacting' ? 'running' : 'idle'} />
        <h2 className="truncate text-[15px] font-semibold">{unsupported ? 'Lokales Modell testen' : 'Rate-Limiter für die Login-API'}</h2>
        <HarnessBadge id={harness} model={harnesses[harness].models[0]} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          <UserMessage>Jetzt bitte auch die Register- und Passwort-Reset-Route absichern, gleiche Regeln.</UserMessage>
          <ToolCall kind="read" name="Lesen" target="src/routes/register.ts" duration="0,1 s" />
          <ToolCall kind="read" name="Lesen" target="src/routes/password-reset.ts" duration="0,1 s" />
          <ToolCall kind="edit" name="Bearbeiten" target="src/routes/register.ts" duration="0,2 s" />
          <AgentMessage harness={harness}>
            <p>Beide Routen nutzen jetzt dieselbe Middleware. Die Tests für Register und Reset laufen grün.</p>
          </AgentMessage>
          <TurnFooter duration="48 s" tokens="21.900" cost="Subscription" />

          {state === 'compacting' && (
            <F id="USE-009">
              <SystemNote>Kontext wird kompaktiert … (von dir ausgelöst)</SystemNote>
            </F>
          )}
          {state === 'compacted' && (
            <F id="USE-009" className="flex flex-col gap-2">
              <SystemNote>Kontext kompaktiert · 186.400 → 38.200 Tokens · von dir ausgelöst</SystemNote>
              <p className="ml-9 text-[12px] text-muted-foreground">
                Claude Code hat den bisherigen Verlauf zusammengefasst. Der volle Verlauf bleibt in beton erhalten und sichtbar.
              </p>
            </F>
          )}
        </div>
      </div>

      <div className="relative">
        {state === 'auto' && (
          <F id="USE-009" className="absolute right-4 bottom-full z-10 mb-1 w-[380px] rounded-md border border-border bg-popover p-3 text-[13px] shadow-lg">
            <div className="font-semibold">Automatisch kompaktieren</div>
            <div className="mt-2 flex flex-col gap-1.5">
              {[
                { id: 'harness', l: 'Der Harness entscheidet', h: 'Standard. Claude Code und Codex kompaktieren selbst, wenn es nötig ist.' },
                { id: 'beton', l: 'beton löst aus ab', h: 'Vor dem nächsten Turn, genau einmal pro Überschreitung.' },
                { id: 'off', l: 'Nie automatisch', h: 'Nur über den Knopf, /compact oder ⌘K.' },
              ].map((o) => (
                <label key={o.id} className="flex gap-2">
                  <span className={cn('mt-1 size-3 shrink-0 rounded-full border border-foreground', o.id === 'beton' && 'border-4')} />
                  <span>
                    <span className="font-medium">{o.l}</span>
                    {o.id === 'beton' && <span className="ml-1 rounded-sm border border-input bg-card px-1.5 font-mono text-[12px]">80 %</span>}
                    <span className="block text-[12px] text-muted-foreground">{o.h}</span>
                  </span>
                </label>
              ))}
            </div>
            <div className="mt-2 border-t border-border pt-2 text-[11px] text-muted-foreground">Gilt für deine Sessions. Pro Session in den Session-Einstellungen änderbar.</div>
          </F>
        )}
        <div className="flex items-center gap-3 border-t border-border px-4 py-1.5">
          <F id="USE-008" as="span" badge="top-left">
            <ContextRing used={ctx.used} window={ctx.window} />
          </F>
          <span className="text-[11px] text-muted-foreground tabular-nums">
            {ctx.window ? `${ctx.used.toLocaleString('de-DE')} von ${ctx.window.toLocaleString('de-DE')} Tokens` : 'Fenstergröße nicht bekannt (weder Harness noch Katalog)'}
          </span>
          <F id="USE-009" as="span" className="ml-auto flex items-center gap-1" badge="top-right">
            <button
              disabled={unsupported || state === 'compacting'}
              title={unsupported ? 'Ollama (Direkt-API) kann nicht kompaktieren. Starte bei Bedarf eine neue Session.' : 'Verlauf zusammenfassen, um Platz im Kontext zu schaffen (/compact)'}
              className={cn(
                'inline-flex h-7 items-center gap-1 rounded-md border px-2 text-[12px] disabled:cursor-not-allowed disabled:opacity-50',
                state === 'critical' ? 'border-foreground font-semibold' : 'border-border hover:bg-accent',
              )}
            >
              <Minimize2 className="size-3.5" /> {state === 'compacting' ? 'Kompaktiert …' : 'Kompaktieren'}
            </button>
            {unsupported && <span className="text-[11px] text-muted-foreground">nicht verfügbar für diesen Harness</span>}
            <button aria-label="Automatisches Kompaktieren einstellen" className={cn('flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent', state === 'auto' && 'bg-accent text-foreground')}>
              <Settings2 className="size-3.5" />
            </button>
          </F>
        </div>
      </div>
      <Composer harness={harness} running={state === 'compacting'} />
    </AppLayout>
  )
}

/* ───────────────────────── CLI `beton usage` (USE-007) ───────────────────────── */

function UsageCli({ state }: { state: string }) {
  const dim = 'opacity-60'
  return (
    <F id="USE-007">
      {state === 'session' ? (
        <pre className="whitespace-pre">
          <span className={dim}>$ </span>beton usage --session ses_01J9X7F3K
          {`
TURN  MODEL              AUTH          INPUT   OUTPUT  CACHE_R  COST     SOURCE
   1  claude-opus-5-5    subscription  18.2k   1.1k    0        -        subscription
   2  claude-opus-5-5    subscription  41.0k   2.4k    16.8k    -        subscription
   3  claude-opus-5-5    subscription  62.7k   3.0k    39.2k    -        subscription
   4  claude-sonnet-5-5  subscription  21.9k   0.8k    18.1k    -        subscription
TOTAL                                  143.8k  7.3k    74.1k    -
`}
          <span className={dim}>{`note: subscription usage is not billed; equivalent API price: ~$1.92 (catalog 2026-10-01, informational)`}</span>
        </pre>
      ) : state === 'json' ? (
        <pre className="whitespace-pre">
          <span className={dim}>$ </span>beton usage --json --by model --since 1d
          {`
{
  "from": "2026-10-02T14:05:00+02:00",
  "to": "2026-10-03T14:05:00+02:00",
  "group_by": ["model"],
  "items": [
    { "model": "claude-opus-5-5", "auth_source": "vendor_cli",
      "input_tokens": 1803400, "output_tokens": 98200, "cache_read_tokens": 1412000,
      "cost_usd": null, "equivalent_usd": 34.91, "unpriced_count": 0 },
    { "model": "gpt-5.3-codex", "auth_source": "vendor_cli",
      "input_tokens": 610200, "output_tokens": 41300, "cache_read_tokens": 488000,
      "cost_usd": null, "equivalent_usd": 1.17, "unpriced_count": 0 }
  ],
  "totals": { "cost_usd": 0, "input_tokens": 2413600, "output_tokens": 139500 },
  "pricing_version": "2026-10-01"
}`}
        </pre>
      ) : (
        <pre className="whitespace-pre">
          <span className={dim}>$ </span>beton usage
          {`
Today (Sat 2026-10-03)
HARNESS       AUTH          TURNS  INPUT   OUTPUT  CACHE_R  COST
claude-code   subscription      7  551.0k  51.2k   409.6k   -
codex         subscription      0  0       0       0        -

Last 7 days
HARNESS       AUTH          TURNS  INPUT   OUTPUT  CACHE_R  COST
claude-code   subscription     61  11.98M  1.02M   9.84M    -
codex         subscription     27  4.21M   380.0k  3.11M    -
gemini-cli    subscription      4  380.0k  40.0k   0        -
ollama        local             7  520.0k  120.0k  0        $0.00

Subscription windows
claude-code   Claude Max    5h      38%  resets 16:19
                            weekly  61%  resets Mon 09:00
codex         ChatGPT Pro   5h      12%  resets 18:07
gemini-cli    Google        not reported
`}
          <span className={dim}>{`hint: cost is only shown for api_key and gateway sessions (stderr)`}</span>
          {`
`}
          <span className={dim}>$ </span>
          <span className="inline-block h-4 w-2 translate-y-0.5 bg-current" />
        </pre>
      )}
    </F>
  )
}

export const group: ScreenGroup = {
  id: 'usage',
  title: 'Verbrauch',
  order: 90,
  screens: [
    {
      id: 'usage-overview',
      title: 'Verbrauch',
      description:
        'Tokens nach Tag, Session, Projekt, Harness, Modell und Anmeldung; Subscription-Kontingente je CLI-Login als Rate-Limit-Fenster. Euro nur für API-Key- und Gateway-Sessions.',
      features: ['USE-001', 'USE-002', 'USE-004', 'USE-005', 'USE-006'],
      states: [
        { id: 'subscriptions', title: 'Nur Subscriptions (Normalfall)' },
        { id: 'quota-high', title: 'Kontingent fast aufgebraucht' },
        { id: 'mixed', title: 'Mit API-Key & Gateway' },
        { id: 'team', title: 'Team-Server (Person/Team)' },
        { id: 'empty', title: 'Noch kein Verbrauch' },
      ],
      component: UsageOverview,
    },
    {
      id: 'usage-pricing',
      title: 'Preise',
      description: 'Mitgelieferter, versionierter Preis-Katalog (kein Online-Abruf) und eigene Preise für Gateway- und lokale Modelle, mit Herleitung.',
      features: ['USE-002', 'USE-003'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'unpriced', title: 'Turns ohne Preis' },
        { id: 'explain', title: 'Herleitung' },
      ],
      component: UsagePricing,
    },
    {
      id: 'usage-context',
      title: 'Kontext & Compaction',
      description:
        'Kontextfüllstand als Ring über der Eingabe (Schwellen 70/85/95 % mit Text), manuelles Kompaktieren, Automatik-Einstellung und das Compaction-Ereignis im Verlauf.',
      features: ['USE-008', 'USE-009'],
      states: [
        { id: 'normal', title: '42 %' },
        { id: 'warn', title: '75 % – wird knapp' },
        { id: 'critical', title: '93 % – fast voll' },
        { id: 'compacting', title: 'Kompaktiert …' },
        { id: 'compacted', title: 'Kompaktiert' },
        { id: 'auto', title: 'Automatik einstellen' },
        { id: 'no-window', title: 'Fenster unbekannt' },
        { id: 'unsupported', title: 'Harness ohne Compaction' },
      ],
      component: UsageContext,
    },
    {
      id: 'usage-cli',
      title: 'beton usage (CLI)',
      description: 'Dieselben Zahlen auf der Kommandozeile; stdout nur Daten, Hinweise auf stderr. CLI-Ausgaben sind in v1 Englisch.',
      features: ['USE-007'],
      frame: 'terminal',
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'session', title: '--session' },
        { id: 'json', title: '--json' },
      ],
      component: UsageCli,
    },
  ],
}
