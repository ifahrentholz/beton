import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { axis, dayShort, dollars, fmtTokens, type SeriesDay } from '@/lib/usage'
import { voiceOf, voiceVar } from '../harness'

/* Bausteine der Verbrauchsseite (USE-006, USE-004) nach dem Entwurf `usage-overview`. */

export function Segmented({
  value,
  options,
  label,
  onChange,
}: {
  value: string
  options: { id: string; label: ReactNode; disabled?: boolean; title?: string | undefined }[]
  label: string
  onChange?: (id: string) => void
}) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex flex-wrap rounded-md border border-border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
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

export function Kpi({ label, value, sub }: { label: string; value: ReactNode; sub?: ReactNode }) {
  return (
    <div className="min-w-0 border-l border-border pl-4 first:border-l-0 first:pl-0">
      <div className="text-[12px] text-muted-foreground">{label}</div>
      <div className="type-wide mt-0.5 text-[22px] font-[650] tabular-nums">{value}</div>
      {sub && <div className="text-[11px] text-muted-foreground">{sub}</div>}
    </div>
  )
}

/** Kontingent-Balken eines Rate-Limit-Fensters (USE-004 AC2): Anteil, Reset, ab 90 % Rost. */
export function QuotaBar({ label, pct, reset, warn }: { label: string; pct: number; reset: string; warn?: boolean }) {
  const shown = Math.round(pct)
  const high = pct >= 90
  const full = pct >= 100
  return (
    <div className="grid grid-cols-[120px_1fr_auto] items-center gap-3 py-1">
      <span className="text-[12px] text-muted-foreground">{label}</span>
      <span className="relative h-2 overflow-hidden rounded-full bg-muted" role="meter" aria-valuenow={shown} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
        <span className={cn('absolute inset-y-0 left-0', full ? 'bg-deny' : high ? 'bg-warn' : 'bg-foreground/70')} style={{ width: `${Math.min(100, Math.max(0, pct))}%` }} />
        {[25, 50, 75].map((m) => (
          <span key={m} className="absolute inset-y-0 w-px bg-background/70" style={{ left: `${m}%` }} />
        ))}
      </span>
      <span className="text-[12px] whitespace-nowrap tabular-nums">
        <strong className={cn('font-semibold', full ? 'text-deny' : high && 'text-warn')}>{shown} %</strong>
        <span className="text-muted-foreground"> genutzt · {reset}</span>
        {high && warn && <span className={cn('ml-2', full ? 'text-deny' : 'text-warn')}>{full ? 'aufgebraucht' : 'fast aufgebraucht'}</span>}
      </span>
    </div>
  )
}

/** Ruhiges, gestapeltes Säulendiagramm in CSS, ohne Chart-Bibliothek (ADR-0033, WEB-015). */
export function StackedBars({ mode, days, harnesses, today }: { mode: 'tokens' | 'cost'; days: SeriesDay[]; harnesses: string[]; today: string }) {
  const max = Math.max(0, ...days.map((d) => (mode === 'tokens' ? d.total : d.cost)))
  const ticks = axis(max)
  const top = ticks[ticks.length - 1] ?? 1
  const tickLabel = (t: number) => {
    if (mode === 'cost') return dollars(t).replace(',00', '')
    if (t === 0) return '0'
    return fmtTokens(t)
  }
  return (
    <div className="flex gap-2" data-testid="usage-chart">
      <div className="relative h-44 w-14 shrink-0 text-right text-[10px] text-muted-foreground tabular-nums">
        {ticks.map((t) => (
          <span key={t} className="absolute right-1 -translate-y-1/2" style={{ top: `${100 - (t / top) * 100}%` }}>
            {tickLabel(t)}
          </span>
        ))}
      </div>
      <div className="min-w-0 flex-1">
        <div className="relative h-44 border-b border-foreground/40">
          {ticks.slice(1).map((t) => (
            <span key={t} className="absolute inset-x-0 h-px bg-border" style={{ top: `${100 - (t / top) * 100}%` }} />
          ))}
          <div className="absolute inset-0 flex items-end gap-[6px] px-1">
            {days.map((d) => (
              <div
                key={d.day}
                className="flex h-full flex-1 flex-col-reverse"
                title={`${dayShort(d.day)}: ${mode === 'tokens' ? `${fmtTokens(d.total)} Tokens` : dollars(d.cost)}`}
              >
                {mode === 'tokens'
                  ? harnesses.map((h) =>
                      d.tokens[h] ? (
                        <span
                          key={h}
                          className="block w-full border-t border-background first:rounded-b-none last:rounded-t-[2px]"
                          style={{ height: `${((d.tokens[h] ?? 0) / top) * 100}%`, background: voiceVar[voiceOf(h)] }}
                        />
                      ) : null,
                    )
                  : d.cost > 0 && <span className="block w-full rounded-t-[2px] bg-foreground/75" style={{ height: `${(d.cost / top) * 100}%` }} />}
                {d.day === today && <span className="sr-only">heute</span>}
              </div>
            ))}
          </div>
        </div>
        <div className="mt-1 flex gap-[6px] px-1 text-[10px] text-muted-foreground">
          {days.map((d) => (
            <span key={d.day} className={cn('flex-1 truncate text-center whitespace-nowrap', d.day === today && 'font-semibold text-foreground')}>
              {dayShort(d.day)}
            </span>
          ))}
        </div>
      </div>
    </div>
  )
}
