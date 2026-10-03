import { useMemo, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { features, specUrl } from '@/catalog/types'
import { cn } from '@/lib/utils'
import { coverage, coverageStats, screenById } from './registry'

const MILESTONES = ['alle', 'M0', 'M1', 'M2', 'M3', 'M4', 'M5', 'v2'] as const
const STATUS = ['alle', 'mit Screen', 'kein UI', 'offen'] as const

export function CatalogPage() {
  const [query, setQuery] = useState('')
  const [milestone, setMilestone] = useState<(typeof MILESTONES)[number]>('alle')
  const [status, setStatus] = useState<(typeof STATUS)[number]>('alle')
  const stats = coverageStats()

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase()
    return features.filter((f) => {
      const c = coverage.get(f.id)!
      const st = c.screens.length ? 'mit Screen' : c.noUi ? 'kein UI' : 'offen'
      if (milestone !== 'alle' && f.milestone !== milestone) return false
      if (status !== 'alle' && st !== status) return false
      if (q && !`${f.id} ${f.title} ${f.summary}`.toLowerCase().includes(q)) return false
      return true
    })
  }, [query, milestone, status])

  return (
    <div className="flex flex-col gap-4 p-6">
      <div>
        <h1 className="type-wide text-2xl font-[700]">Feature-Katalog</h1>
        <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
          Jedes Feature der Spec und wo es im Prototyp zu sehen ist. Features ohne eigene Oberfläche
          verweisen auf den Screen, in dem man ihre Wirkung sieht.
        </p>
      </div>
      <div className="flex flex-wrap gap-6 text-sm">
        <Stat value={stats.withScreen} label="mit Screen" />
        <Stat value={stats.noUi} label="ohne eigene Oberfläche" />
        <Stat value={stats.open} label="noch nicht zugeordnet" warn={stats.open > 0} />
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Suchen nach ID, Titel oder Inhalt"
          aria-label="Features durchsuchen"
          className="h-8 w-72 rounded-md border border-input bg-card px-2.5 text-sm"
        />
        <Segmented label="Meilenstein" options={MILESTONES} value={milestone} onChange={setMilestone} />
        <Segmented label="Status" options={STATUS} value={status} onChange={setStatus} />
        <span className="text-xs text-muted-foreground">{rows.length} Treffer</span>
      </div>
      <div className="overflow-hidden rounded-md border border-border bg-card">
        <table className="w-full text-[13px]">
          <thead className="sticky top-0 bg-sunken text-left text-xs text-muted-foreground">
            <tr>
              <th className="px-3 py-2 font-medium">ID</th>
              <th className="px-3 py-2 font-medium">Feature</th>
              <th className="px-3 py-2 font-medium">Meilenstein</th>
              <th className="px-3 py-2 font-medium">Wo sichtbar</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((f) => {
              const c = coverage.get(f.id)!
              return (
                <tr key={f.id} className="border-t border-border align-top">
                  <td className="px-3 py-2 font-mono text-xs whitespace-nowrap">
                    <a href={specUrl(f)} target="_blank" rel="noreferrer" className="hover:underline">
                      {f.id}
                    </a>
                  </td>
                  <td className="px-3 py-2">
                    <div className="font-medium">{f.title}</div>
                    <div className="line-clamp-2 max-w-xl text-xs text-muted-foreground">{f.summary}</div>
                  </td>
                  <td className="px-3 py-2 text-xs whitespace-nowrap">
                    {f.milestone} · {f.priority}
                  </td>
                  <td className="px-3 py-2 text-xs">
                    {c.screens.length > 0 && (
                      <div className="flex flex-wrap gap-1">
                        {c.screens.map((s) => (
                          <Link
                            key={s.id}
                            to="/s/$screenId"
                            params={{ screenId: s.id }}
                            className="rounded-sm border border-border px-1.5 py-0.5 hover:border-foreground"
                          >
                            {s.title}
                          </Link>
                        ))}
                      </div>
                    )}
                    {!c.screens.length && c.noUi && (
                      <div className="text-muted-foreground">
                        Kein eigenes UI: {c.noUi.reason}
                        {c.noUi.visibleIn && screenById.get(c.noUi.visibleIn) && (
                          <>
                            {' '}
                            Sichtbar in{' '}
                            <Link to="/s/$screenId" params={{ screenId: c.noUi.visibleIn }} className="underline">
                              {screenById.get(c.noUi.visibleIn)!.title}
                            </Link>
                            .
                          </>
                        )}
                      </div>
                    )}
                    {!c.screens.length && !c.noUi && <span className="text-deny">noch nicht zugeordnet</span>}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}

function Stat({ value, label, warn }: { value: number; label: string; warn?: boolean }) {
  return (
    <div className="flex items-baseline gap-1.5">
      <span className={cn('type-wide text-2xl font-[700] tabular-nums', warn && 'text-deny')}>{value}</span>
      <span className="text-muted-foreground">{label}</span>
    </div>
  )
}

function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string
  options: readonly T[]
  value: T
  onChange: (v: T) => void
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex overflow-hidden rounded-md border border-border">
      {options.map((o) => (
        <button
          key={o}
          role="radio"
          aria-checked={o === value}
          onClick={() => onChange(o)}
          className={cn('px-2 py-1 text-xs', o === value ? 'bg-foreground text-background' : 'hover:bg-accent')}
        >
          {o}
        </button>
      ))}
    </div>
  )
}
