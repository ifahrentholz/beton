import { Link } from '@tanstack/react-router'
import { coverageStats, groups } from './registry'
import { Wordmark } from './shell'

const SWATCHES = [
  { name: 'Beton', var: '--background', note: 'Grundfläche' },
  { name: 'Fläche', var: '--card', note: 'Inhalte, Eingaben' },
  { name: 'Graphit', var: '--foreground', note: 'Text, Primärflächen' },
  { name: 'Schalungsgelb', var: '--signal', note: 'Du bist dran' },
  { name: 'Freigabe', var: '--ok', note: 'erlaubt, fertig' },
  { name: 'Signalrot', var: '--deny', note: 'abgelehnt, Fehler' },
]
const VOICES = [
  { name: 'Claude Code', var: '--voice-claude' },
  { name: 'Codex', var: '--voice-codex' },
  { name: 'ACP-Agents', var: '--voice-acp' },
  { name: 'Direkt-API', var: '--voice-direct' },
  { name: 'Mensch', var: '--voice-human' },
]

export function OverviewPage() {
  const stats = coverageStats()
  return (
    <div className="concrete-grain min-h-full">
      <div className="mx-auto flex max-w-5xl flex-col gap-10 px-8 py-10">
        <section className="flex flex-col gap-3">
          <Wordmark className="text-6xl" />
          <p className="max-w-2xl text-base leading-relaxed">
            Ein Meta-Harness für Coding-Agents. Dieser Prototyp zeigt jeden Screen und jeden Zustand der
            App mit Beispieldaten. Er dient als verbindliche Vorlage für die Umsetzung (ADR-0032).
          </p>
          <p className="max-w-2xl text-sm text-muted-foreground">
            Schalte oben „Feature-IDs“ ein, um zu sehen, welcher Bereich welches Feature der Spec zeigt.
            Der Feature-Katalog listet alle {stats.total} Features und wo sie zu finden sind.
          </p>
        </section>

        <section className="grid gap-8 md:grid-cols-[1fr_1fr]">
          <div>
            <h2 className="type-wide mb-3 text-lg font-[700]">Farben</h2>
            <div className="flex flex-col gap-1.5">
              {SWATCHES.map((s) => (
                <div key={s.name} className="flex items-center gap-3">
                  <span className="size-8 rounded-sm border border-border" style={{ background: `var(${s.var})` }} />
                  <span className="w-32 text-sm font-medium">{s.name}</span>
                  <span className="text-xs text-muted-foreground">{s.note}</span>
                </div>
              ))}
            </div>
            <h3 className="mt-5 mb-2 text-sm font-semibold">Stimmen</h3>
            <div className="flex flex-wrap gap-3">
              {VOICES.map((v) => (
                <span key={v.name} className="flex items-center gap-1.5 text-xs">
                  <span className="h-2 w-5 rounded-full" style={{ background: `var(${v.var})` }} />
                  {v.name}
                </span>
              ))}
            </div>
          </div>
          <div>
            <h2 className="type-wide mb-3 text-lg font-[700]">Schrift</h2>
            <p className="type-wide text-3xl font-[750]">Archivo, breit</p>
            <p className="text-xl">Archivo, normal: Sessions, Freigaben, Policies</p>
            <p className="type-narrow text-base text-muted-foreground">
              Archivo, schmal für dichte Tabellen: 1.284 Events · 0,42 $ · 18 Tool-Calls
            </p>
            <p className="mt-2 font-mono text-[13px]">IBM Plex Mono: cargo nextest run --workspace</p>
            <h3 className="mt-5 mb-2 text-sm font-semibold">Die Fase</h3>
            <div className="flex items-center gap-3">
              <span className="chamfer bg-signal px-4 py-2 text-sm font-semibold text-signal-foreground">
                Erlauben
              </span>
              <span className="text-xs text-muted-foreground">
                45°-Ecken nur an Elementen, die auf dich warten.
              </span>
            </div>
          </div>
        </section>

        <section>
          <h2 className="type-wide mb-3 text-lg font-[700]">Bereiche</h2>
          <div className="grid gap-x-8 gap-y-4 sm:grid-cols-2 lg:grid-cols-3">
            {groups.map((g) => (
              <div key={g.id}>
                <div className="text-sm font-semibold">{g.title}</div>
                <ul className="mt-1 text-sm">
                  {g.screens.map((s) => (
                    <li key={s.id}>
                      <Link to="/s/$screenId" params={{ screenId: s.id }} className="text-muted-foreground hover:text-foreground hover:underline">
                        {s.title}
                      </Link>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </section>
      </div>
    </div>
  )
}
