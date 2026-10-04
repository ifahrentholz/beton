import type { ReactNode } from 'react'
import { Check, Minus } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, StatusMark } from '@/app/harness'
import { Composer } from '@/app/session-chrome'
import { Score, type ScoreEvent, type ScoreVoice } from '@/app/score'
import { AgentMessage, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Pill } from '@/app/kit/harnesses'

const ROUNDS = ['Antworten', 'Kritik 1', 'Kritik 2', 'Urteil']

function scoreFor(state: string): { voices: ScoreVoice[]; now?: number } {
  const all: ScoreVoice[] = [
    {
      id: 'duetto',
      name: 'duetto',
      role: 'Moderation · Claude Code',
      voice: 'claude',
      events: [
        { kind: 'turn', start: 0, end: 0.6, label: 'Frage an beide Stimmen' },
        { kind: 'handoff', at: 0.4, to: 'vc', label: 'session_spawn voce-claude' },
        { kind: 'handoff', at: 0.6, to: 'vx', label: 'session_spawn voce-codex' },
        { kind: 'turn', start: 4, end: 4.5, label: 'Antworten ausgetauscht' },
        { kind: 'turn', start: 7.6, end: 8, label: 'Kritik 1 ausgetauscht' },
        { kind: 'turn', start: 11.4, end: 14, label: 'Synthese' },
      ],
    },
    {
      id: 'vc',
      name: 'voce-claude',
      role: 'Stimme · nur lesend',
      voice: 'claude',
      events: [
        { kind: 'turn', start: 0.8, end: 3.4, label: 'Antwort' },
        { kind: 'tool', at: 1.6, label: 'Lesen: src/middleware/rate-limit.ts' },
        { kind: 'handoff', at: 3.6, to: 'duetto', label: 'Antwort fertig' },
        { kind: 'turn', start: 4.6, end: 6.8, label: 'Kritik 1 an Codex' },
        { kind: 'handoff', at: 7, to: 'duetto', label: 'Kritik 1' },
        { kind: 'turn', start: 8.2, end: 10.4, label: 'Kritik 2 an Codex' },
        { kind: 'handoff', at: 10.6, to: 'duetto', label: 'Kritik 2' },
      ],
    },
    {
      id: 'vx',
      name: 'voce-codex',
      role: 'Stimme · nur lesend',
      voice: 'codex',
      events: [
        { kind: 'turn', start: 0.8, end: 3.8, label: 'Antwort' },
        { kind: 'tool', at: 2, label: 'Suche: rg "gateway" infra/' },
        { kind: 'handoff', at: 3.9, to: 'duetto', label: 'Antwort fertig' },
        { kind: 'turn', start: 4.6, end: 7.4, label: 'Kritik 1 an Claude' },
        { kind: 'handoff', at: 7.5, to: 'duetto', label: 'Kritik 1' },
        { kind: 'turn', start: 8.2, end: 11, label: 'Kritik 2 an Claude' },
        { kind: 'handoff', at: 11.2, to: 'duetto', label: 'Kritik 2' },
      ],
    },
  ]
  const now = state === 'answers' ? 4.2 : state === 'debate' ? 9.6 : undefined
  if (now === undefined) return { voices: all }
  return {
    now,
    voices: all.map((v) => ({
      ...v,
      events: v.events.flatMap<ScoreEvent>((e) => (e.kind === 'turn' ? (e.start > now ? [] : [{ ...e, end: Math.min(e.end, now) }]) : e.at > now ? [] : [e])),
    })),
  }
}

function Column({ harness, title, children, streaming }: { harness: 'claude' | 'codex'; title: string; children: ReactNode; streaming?: boolean }) {
  return (
    <div className="min-w-0 flex-1 space-y-3 px-5 py-4">
      <div className="flex items-center gap-2 text-[12px]">
        <HarnessBadge id={harness} model={harness === 'claude' ? 'claude-opus-5-5' : 'gpt-5.3'} />
        <span className="text-muted-foreground">· {title}</span>
      </div>
      <AgentMessage harness={harness} streaming={streaming}>
        {children}
      </AgentMessage>
    </div>
  )
}

function Point({ kind, children }: { kind: 'yes' | 'no'; children: ReactNode }) {
  return (
    <li className="flex gap-2">
      {kind === 'yes' ? <Check className="mt-0.5 size-3.5 shrink-0 text-ok" /> : <Minus className="mt-0.5 size-3.5 shrink-0 text-deny" />}
      <span>{children}</span>
    </li>
  )
}

export function DuettoRun({ state }: { state: string }) {
  const { voices, now } = scoreFor(state)
  const round = state === 'answers' ? 0 : state === 'debate' ? 2 : 3
  return (
    <AppLayout nav="agents">
      <div className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
        <StatusMark status={state === 'verdict' ? 'idle' : 'running'} />
        <h2 className="min-w-0 truncate text-[15px] font-semibold">duetto · Redis oder Token-Bucket im Gateway?</h2>
        <Pill>/debate rounds=2</Pill>
        <span className="ml-auto text-[12px] text-muted-foreground">shop-frontend · beide Stimmen nur lesend · Abos</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <F id={['AGT-012', 'AGT-009']} as="section" className="border-b border-border px-5 pt-3 pb-3">
          <Score voices={voices} length={14} now={now} ticks={[0, 4, 8, 12].map((m) => ({ at: m, label: `${m} Min.` }))} />
        </F>
        <div className="mx-auto max-w-5xl px-5 pt-4">
          <UserMessage>Sollten wir für den Rate-Limiter Redis nehmen oder das Token-Bucket im API-Gateway (Envoy) nutzen?</UserMessage>
        </div>
        <F id="AGT-012" className="mx-auto mt-4 max-w-5xl px-5">
          <div role="tablist" className="flex gap-1 border-b border-border">
            {ROUNDS.map((r, i) => (
              <button
                key={r}
                role="tab"
                aria-selected={i === round}
                disabled={i > round}
                className={cn(
                  '-mb-px border-b-2 px-2 pb-2 text-[13px]',
                  i === round ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground',
                  i > round && 'opacity-50',
                )}
              >
                {r}
              </button>
            ))}
          </div>

          {state !== 'verdict' ? (
            <div className="flex divide-x divide-border">
              {state === 'answers' ? (
                <>
                  <Column harness="claude" title="voce-claude">
                    <p>Redis. Das Limit muss pro IP über alle drei App-Instanzen gelten, und Redis ist mit dem Session-Store schon da.</p>
                    <p>Mit INCR + EXPIRE pro Fenster bleibt es atomar; Ausfall von Redis → fail open mit Warnung.</p>
                  </Column>
                  <Column harness="codex" title="voce-codex">
                    <p>Gateway. Envoy hat einen lokalen Token-Bucket und kann global über den Rate-Limit-Service zählen.</p>
                    <p>Vorteil: Missbrauch wird vor der App abgefangen, die App bleibt zustandslos.</p>
                  </Column>
                </>
              ) : (
                <>
                  <Column harness="claude" title="Kritik 2 an Codex">
                    <p>
                      Der globale Envoy-Rate-Limit-Service ist im Repo nicht deployt (<code className="rounded-sm bg-muted px-1 text-[13px]">infra/envoy</code> hat nur
                      den lokalen Filter). Lokal pro Pod zählt das Limit dreifach.
                    </p>
                  </Column>
                  <Column harness="codex" title="Kritik 2 an Claude" streaming>
                    <p>Zugestanden. Aber fail open bei Redis-Ausfall heißt: genau im Störfall ist der Login ungeschützt. Mindestens ein lokales Fallback-Limit</p>
                  </Column>
                </>
              )}
            </div>
          ) : (
            <div className="grid grid-cols-[1fr_1fr] gap-8 py-4 text-[13px]">
              <div>
                <h4 className="mb-2 font-semibold">Konsens</h4>
                <ul className="space-y-1.5">
                  <Point kind="yes">Das Limit muss instanzübergreifend gelten; ein Zähler pro Pod reicht nicht.</Point>
                  <Point kind="yes">Kurzfristig ist Redis die realistische Wahl, weil der globale Envoy-Dienst fehlt.</Point>
                  <Point kind="yes">Bei Redis-Ausfall braucht es ein lokales Fallback-Limit statt fail open.</Point>
                </ul>
              </div>
              <div>
                <h4 className="mb-2 font-semibold">Dissens</h4>
                <ul className="space-y-1.5">
                  <Point kind="no">
                    <span className="text-muted-foreground">Claude:</span> Gateway-Limit erst, wenn mehr Routen betroffen sind.{' '}
                    <span className="text-muted-foreground">Codex:</span> jetzt als zweite Schicht planen.
                  </Point>
                </ul>
              </div>
              <div className="col-span-2 border-t border-border pt-3">
                <AgentMessage harness="claude">
                  <p>
                    <strong>Urteil:</strong> Redis mit INCR/EXPIRE, dazu ein lokales Fallback-Limit von 20 Versuchen pro Minute und Pod. Das Gateway-Limit als
                    eigenes Ticket für Q1.
                  </p>
                </AgentMessage>
              </div>
            </div>
          )}
        </F>
      </div>
      <Composer harness="claude" placeholder="Nachfrage an duetto – /debate rounds=3 für eine weitere Runde" running={state === 'debate'} />
    </AppLayout>
  )
}
