import { cn } from '@/lib/utils'
import type { Voice } from '@/mock/data'
import { voiceVar } from './harness'

/**
 * Die Partitur: Multi-Agent-Läufe als Notensystem. Jede Stimme (Agent auf einem Harness)
 * hat eine Notenlinie; Takte sind Turns, Noten Tool-Calls, Fermaten (gelb) Freigaben,
 * auf die der Dirigent – du – antworten muss. Pfeile zeigen Übergaben zwischen Stimmen.
 */
export type ScoreEvent =
  | { kind: 'turn'; start: number; end: number; label?: string }
  | { kind: 'tool'; at: number; label: string; failed?: boolean }
  | { kind: 'fermata'; at: number; label: string; resolved?: boolean }
  | { kind: 'handoff'; at: number; to: string; label?: string }

export type ScoreVoice = {
  id: string
  name: string
  role: string
  voice: Voice
  events: ScoreEvent[]
}

export function Score({
  voices,
  length,
  now,
  ticks = [],
  className,
}: {
  voices: ScoreVoice[]
  /** Gesamtlänge in Zeiteinheiten (z. B. Minuten). */
  length: number
  /** Aktueller Zeitpunkt (Playhead). */
  now?: number
  ticks?: { at: number; label: string }[]
  className?: string
}) {
  const x = (t: number) => `${(t / length) * 100}%`
  const rowIndex = new Map(voices.map((v, i) => [v.id, i]))
  const ROW = 44
  return (
    <div className={cn('select-none', className)}>
      <div className="flex">
        <div className="w-40 shrink-0" />
        <div className="relative h-5 flex-1 text-[10px] text-muted-foreground">
          {ticks.map((t) => (
            <span key={t.at} className="absolute -translate-x-1/2 tabular-nums" style={{ left: x(t.at) }}>
              {t.label}
            </span>
          ))}
        </div>
      </div>
      <div className="relative">
        {voices.map((v) => (
          <div key={v.id} className="flex" style={{ height: ROW }}>
            <div className="flex w-40 shrink-0 flex-col justify-center pr-3">
              <span className="truncate text-[13px] font-medium">{v.name}</span>
              <span className="truncate text-[11px] text-muted-foreground">{v.role}</span>
            </div>
            <div className="relative flex-1">
              {/* Notenlinien: drei feine Linien je Stimme */}
              {[0.3, 0.5, 0.7].map((p) => (
                <span key={p} className="absolute inset-x-0 h-px bg-border" style={{ top: `${p * 100}%` }} />
              ))}
              {v.events.map((e, i) => {
                if (e.kind === 'turn')
                  return (
                    <span
                      key={i}
                      title={e.label}
                      className="absolute top-[30%] h-[40%] rounded-[2px] opacity-90"
                      style={{ left: x(e.start), width: `calc(${x(e.end - e.start)} - 2px)`, background: voiceVar[v.voice] }}
                    />
                  )
                if (e.kind === 'tool')
                  return (
                    <span
                      key={i}
                      title={e.label}
                      className={cn('absolute top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-card', e.failed ? 'bg-deny' : 'bg-foreground')}
                      style={{ left: x(e.at) }}
                    />
                  )
                if (e.kind === 'fermata')
                  return (
                    <span
                      key={i}
                      title={e.label}
                      className={cn('absolute top-0 flex -translate-x-1/2 flex-col items-center', !e.resolved && 'animate-pulse')}
                      style={{ left: x(e.at) }}
                    >
                      <svg width="18" height="10" viewBox="0 0 18 10" aria-hidden>
                        <path d="M1 9 A8 8 0 0 1 17 9" fill="none" stroke={e.resolved ? 'var(--muted-foreground)' : 'var(--signal)'} strokeWidth="2" />
                        <circle cx="9" cy="7" r="1.8" fill={e.resolved ? 'var(--muted-foreground)' : 'var(--signal)'} />
                      </svg>
                    </span>
                  )
                if (e.kind === 'handoff') {
                  const from = rowIndex.get(v.id) ?? 0
                  const to = rowIndex.get(e.to) ?? 0
                  const dy = (to - from) * ROW
                  return (
                    <span
                      key={i}
                      title={e.label}
                      className="absolute top-1/2 w-px border-l border-dashed border-foreground/50"
                      style={{ left: x(e.at), height: Math.abs(dy), transform: dy < 0 ? `translateY(${dy}px)` : undefined }}
                    />
                  )
                }
                return null
              })}
            </div>
          </div>
        ))}
        {now !== undefined && (
          <div className="pointer-events-none absolute inset-y-0 right-0 left-40">
            <span className="absolute inset-y-0 w-px bg-foreground" style={{ left: x(now) }} />
          </div>
        )}
      </div>
    </div>
  )
}
