import type { ReactNode } from 'react'
import { GitBranch, Square } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, StatusMark } from '@/app/harness'
import { Score, type ScoreEvent, type ScoreVoice } from '@/app/score'
import { AgentMessage, ApprovalCard } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { HarnessId, SessionStatus } from '@/mock/data'
import { Btn, Code, Meter, Note, Pill, SectionTitle } from '@/app/kit/harnesses'

/** Schneidet Ereignisse am Playhead ab: Laufende Turns enden bei `now`, Zukünftiges fehlt. */
function upTo(events: ScoreEvent[], now?: number): ScoreEvent[] {
  if (now === undefined) return events
  return events.flatMap<ScoreEvent>((e) => {
    if (e.kind === 'turn') return e.start > now ? [] : [{ ...e, end: Math.min(e.end, now) }]
    return e.at > now ? [] : [e]
  })
}

const T = (start: number, end: number, label: string): ScoreEvent => ({ kind: 'turn', start, end, label })
const tool = (at: number, label: string, failed?: boolean): ScoreEvent => ({ kind: 'tool', at, label, failed })
const hand = (at: number, to: string, label: string): ScoreEvent => ({ kind: 'handoff', at, to, label })

function voices(state: string): ScoreVoice[] {
  const sameVendor = state === 'same-vendor'
  const needsHuman = state === 'needs-human'
  const running = state === 'running'
  const reviewer = sameVendor ? 'review-claude' : 'review-codex'

  const maestra: ScoreVoice = {
    id: 'maestra',
    name: 'maestra',
    role: 'Planerin · Claude',
    voice: 'claude',
    events: [
      T(0, 3, '/plan: Ziel in 2 Teilaufgaben zerlegt'),
      tool(1.2, 'Lesen: src/checkout/**'),
      hand(3, 'impl-a', 'session_spawn impl-claude · Teilaufgabe 1'),
      hand(3.6, 'impl-b', `session_spawn ${sameVendor ? 'impl-claude' : 'impl-codex'} · Teilaufgabe 2`),
      T(15, 16.2, 'Ergebnis Teilaufgabe 2 erhalten'),
      hand(15.6, 'review-b', 'Cross-Review Teilaufgabe 2'),
      T(17, 18, 'Ergebnis Teilaufgabe 1 erhalten'),
      hand(17.5, 'review-a', 'Cross-Review Teilaufgabe 1'),
      T(23.4, 24.4, 'Befunde Runde 1 an Implementierer A'),
      hand(24, 'impl-a', 'Runde 2: 2 blockierende Punkte beheben'),
      T(31.4, 32.4, 'Runde 2 an Review'),
      hand(31.9, 'review-a', 'Review Runde 2'),
      ...(needsHuman
        ? [
            T(37.6, 38.4, 'Runde 3 beauftragt'),
            hand(38, 'impl-a', 'Runde 3'),
            T(45.4, 47, 'Nach 3 Runden: an dich übergeben'),
            { kind: 'fermata', at: 46.6, label: 'Entscheidung nötig: 1 Blocker bleibt' } as ScoreEvent,
          ]
        : [T(38.6, 44, '/cross-review abgeschlossen, Zusammenfassung')]),
    ],
  }

  const implA: ScoreVoice = {
    id: 'impl-a',
    name: 'Implementierer A',
    role: 'impl-claude · Claude',
    voice: 'claude',
    events: [
      T(4, 16.4, 'Teilaufgabe 1: Payment Element im Checkout'),
      tool(5, 'Lesen: CheckoutForm.tsx'),
      tool(7, 'Bearbeiten: CheckoutForm.tsx'),
      tool(9, 'Bearbeiten: usePaymentIntent.ts'),
      tool(11, 'pnpm vitest run checkout', true),
      tool(13, 'Bearbeiten: CheckoutForm.tsx'),
      tool(15, 'pnpm vitest run checkout'),
      tool(16, 'git commit'),
      hand(16.6, 'maestra', 'agent.completed: Teilaufgabe 1'),
      T(24.8, 30.4, 'Runde 2: Befunde beheben'),
      tool(26, 'Bearbeiten: usePaymentIntent.ts'),
      tool(28, 'pnpm vitest run checkout'),
      tool(29.4, 'git commit'),
      { kind: 'fermata', at: 30.2, label: 'Freigabe: git push -u origin beton/maestra-k2q/payment-element', resolved: !running },
      hand(31.2, 'maestra', 'agent.completed: Runde 2'),
      ...(needsHuman ? [T(38.6, 43.6, 'Runde 3'), tool(40, 'Bearbeiten'), tool(42.5, 'pnpm vitest run', true), hand(43.8, 'maestra', 'agent.completed: Runde 3')] : []),
    ],
  }

  const implB: ScoreVoice = {
    id: 'impl-b',
    name: 'Implementierer B',
    role: sameVendor ? 'impl-claude · Claude' : 'impl-codex · Codex',
    voice: sameVendor ? 'claude' : 'codex',
    events: [
      T(4.4, 14.6, 'Teilaufgabe 2: Webhook-Handler payment_intent.*'),
      tool(6, 'Lesen: api/webhooks.ts'),
      tool(8, 'apply_patch: api/webhooks/stripe.ts'),
      tool(10, 'apply_patch: api/webhooks/stripe.spec.ts'),
      tool(12.5, 'pnpm vitest run webhooks'),
      tool(14, 'git commit'),
      hand(14.8, 'maestra', 'agent.completed: Teilaufgabe 2'),
    ],
  }

  const reviewA: ScoreVoice = {
    id: 'review-a',
    name: 'Reviewer A',
    role: sameVendor ? 'review-claude · Claude' : `${reviewer} · Codex`,
    voice: sameVendor ? 'claude' : 'codex',
    events: [
      T(18.2, 23, 'Review Runde 1: 2 blockierende Punkte'),
      tool(19.5, 'git diff main…impl-a'),
      tool(21.5, 'Lesen: usePaymentIntent.ts'),
      hand(23.2, 'maestra', 'Review Runde 1'),
      ...(needsHuman
        ? [T(32.6, 37.2, 'Runde 2: 1 Blocker bleibt'), hand(37.4, 'maestra', 'Review Runde 2'), T(44, 45.2, 'Runde 3: 1 Blocker bleibt'), hand(45.3, 'maestra', 'Review Runde 3')]
        : [T(32.6, 37.6, 'Review Runde 2: keine Blocker'), tool(34, 'git diff'), hand(37.8, 'maestra', 'Review Runde 2')]),
    ],
  }

  const reviewB: ScoreVoice = {
    id: 'review-b',
    name: 'Reviewer B',
    role: 'review-claude · Claude',
    voice: 'claude',
    events: [T(16.4, 21, 'Review: 1 Hinweis, kein Blocker'), tool(18, 'git diff main…impl-b'), hand(21.2, 'maestra', 'Review Teilaufgabe 2')],
  }

  return [maestra, implA, implB, reviewA, reviewB]
}

type VoiceRow = { name: string; agent: string; harness: HarnessId; branch?: string; status: SessionStatus; note: string }

function voiceRows(state: string): VoiceRow[] {
  const sv = state === 'same-vendor'
  const running = state === 'running'
  const nh = state === 'needs-human'
  return [
    { name: 'maestra', agent: 'builtin:maestra', harness: 'claude', status: nh ? 'waiting' : running ? 'running' : 'idle', note: 'nur planen, merged nie' },
    { name: 'Implementierer A', agent: 'impl-claude', harness: 'claude', branch: 'beton/maestra-k2q/payment-element', status: running ? 'waiting' : 'idle', note: running ? 'wartet auf deine Freigabe' : nh ? 'Runde 3 von 3' : 'Runde 2 von 3' },
    { name: 'Implementierer B', agent: sv ? 'impl-claude' : 'impl-codex', harness: sv ? 'claude' : 'codex', branch: 'beton/maestra-k2q/stripe-webhooks', status: 'idle', note: 'fertig' },
    { name: 'Reviewer A', agent: sv ? 'review-claude' : 'review-codex', harness: sv ? 'claude' : 'codex', status: running ? 'idle' : nh ? 'idle' : 'idle', note: nh ? '1 Blocker offen' : running ? 'wartet auf Runde 2' : 'keine Blocker' },
    { name: 'Reviewer B', agent: 'review-claude', harness: 'claude', status: 'idle', note: 'keine Blocker' },
  ]
}

function Phases({ state }: { state: string }) {
  const steps: [string, 'done' | 'now' | 'next' | 'you'][] =
    state === 'running'
      ? [['Plan', 'done'], ['Umsetzung (2 parallel)', 'done'], ['Cross-Review · Runde 2 von 3', 'now'], ['Zusammenfassung', 'next']]
      : state === 'needs-human'
        ? [['Plan', 'done'], ['Umsetzung', 'done'], ['Cross-Review · 3 von 3 Runden', 'done'], ['Du entscheidest', 'you']]
        : [['Plan', 'done'], ['Umsetzung', 'done'], ['Cross-Review · 2 Runden', 'done'], ['Zusammenfassung', 'done']]
  return (
    <ol className="flex flex-wrap items-center gap-x-1 gap-y-1 text-[12px]">
      {steps.map(([label, s], i) => (
        <li key={label} className="flex items-center gap-1">
          {i > 0 && <span className="px-1 text-muted-foreground">→</span>}
          <span
            className={cn(
              'rounded-sm px-1.5 py-0.5',
              s === 'done' && 'text-muted-foreground',
              s === 'now' && 'border border-foreground font-medium',
              s === 'next' && 'border border-dashed border-border text-muted-foreground',
              s === 'you' && 'chamfer-sm bg-signal font-semibold text-signal-foreground',
            )}
          >
            {s === 'done' && '✓ '}
            {label}
          </span>
        </li>
      ))}
    </ol>
  )
}

function Legend() {
  return (
    <div className="flex flex-wrap items-center gap-x-5 gap-y-1 pl-40 text-[11px] text-muted-foreground">
      <span className="inline-flex items-center gap-1.5">
        <span className="h-2 w-5 rounded-[2px] bg-voice-claude" />
        <span className="h-2 w-5 rounded-[2px] bg-voice-codex" /> Takt = Turn (Farbe = Harness)
      </span>
      <span className="inline-flex items-center gap-1.5">
        <span className="size-2 rounded-full bg-foreground" /> Tool-Call
      </span>
      <span className="inline-flex items-center gap-1.5">
        <span className="size-2 rounded-full bg-deny" /> fehlgeschlagen
      </span>
      <span className="inline-flex items-center gap-1.5">
        <svg width="14" height="8" viewBox="0 0 18 10" aria-hidden>
          <path d="M1 9 A8 8 0 0 1 17 9" fill="none" stroke="var(--signal)" strokeWidth="2" />
          <circle cx="9" cy="7" r="1.8" fill="var(--signal)" />
        </svg>
        Fermate = Freigabe (gelb: wartet auf dich)
      </span>
      <span className="inline-flex items-center gap-1.5">
        <span className="h-3 border-l border-dashed border-foreground/60" /> Übergabe
      </span>
      <span className="inline-flex items-center gap-1.5">
        <span className="h-3 w-px bg-foreground" /> jetzt
      </span>
    </div>
  )
}

function Side({ state, children }: { state: string; children?: ReactNode }) {
  const done = state === 'done' || state === 'same-vendor'
  return (
    <aside className="w-[380px] shrink-0 space-y-6 overflow-y-auto border-l border-border px-5 py-4">
      <F id={['AGT-009', 'AGT-011']} as="section">
        <SectionTitle aside="5 Sessions · eigene Worktrees">Stimmen</SectionTitle>
        <ul className="divide-y divide-border border-y border-border text-[13px]">
          {voiceRows(state).map((v) => (
            <li key={v.name} className="py-2">
              <div className="flex items-center gap-2">
                <StatusMark status={v.status} />
                <span className="font-medium">{v.name}</span>
                <HarnessBadge id={v.harness} className="ml-auto" />
              </div>
              <div className="flex items-center gap-1.5 pl-4 text-[11.5px] text-muted-foreground">
                <span className="font-mono">{v.agent}</span>
                <span>· {v.note}</span>
              </div>
              {v.branch && (
                <div className="flex items-center gap-1 pl-4 font-mono text-[11px] text-muted-foreground">
                  <GitBranch className="size-3" /> {v.branch}
                </div>
              )}
            </li>
          ))}
        </ul>
      </F>
      <F id={['ASY-010', 'HAR-021']} as="section">
        <SectionTitle aside="ganzer Baum inkl. Sub-Agents">Verbrauch dieses Laufs</SectionTitle>
        <div className="space-y-3">
          <Meter value={done ? 9 : 7} max={100} label="Claude Max" detail={`${done ? 9 : 7} % des 5-Std.-Fensters · gesamt 41 %`} />
          <Meter value={4} max={100} label="ChatGPT Pro" detail="4 % des 5-Std.-Fensters · gesamt 12 %" />
          <Meter value={done ? 44 : 31} max={120} label="Laufzeit" detail={`${done ? 44 : 31} von 120 Min.`} />
          <Meter value={done ? 63 : 48} max={150} label="Turns" detail={`${done ? 63 : 48} von 150`} />
          <p className="text-[12px] text-muted-foreground">Kosten: 0 € – alle Stimmen laufen über deine Abos.</p>
        </div>
      </F>
      {children}
    </aside>
  )
}

export function MaestraRun({ state }: { state: string }) {
  const now = state === 'running' ? 30.8 : undefined
  const vs = voices(state).map((v) => ({ ...v, events: upTo(v.events, now) }))
  const sameVendor = state === 'same-vendor'
  const status: SessionStatus = state === 'running' || state === 'needs-human' ? 'waiting' : 'idle'

  return (
    <AppLayout nav="agents">
      <div className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
        <StatusMark status={status} />
        <h2 className="min-w-0 truncate text-[15px] font-semibold">maestra · Checkout auf Stripe Payment Element umstellen</h2>
        <Pill>Lauf run_8k2q</Pill>
        <span className="text-[12px] text-muted-foreground">shop-frontend · gestartet 09:12</span>
        <div className="ml-auto flex items-center gap-2">
          <Btn variant="outline" size="sm">
            Als Sessions öffnen
          </Btn>
          {state === 'running' && (
            <Btn variant="outline" size="sm">
              <Square className="size-3 fill-current" /> Lauf abbrechen
            </Btn>
          )}
        </div>
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1 overflow-y-auto">
          <F id={['AGT-011', 'AGT-009', 'AGT-007', 'ASY-002']} as="section" className="border-b border-border px-5 pt-4 pb-3">
            <div className="mb-3 flex items-center gap-4">
              <h3 className="type-wide text-[13px] font-[650]">Partitur</h3>
              <Phases state={state} />
            </div>
            <Score
              voices={vs}
              length={48}
              now={now}
              ticks={[0, 10, 20, 30, 40].map((m) => ({ at: m, label: `${m} Min.` }))}
            />
            <div className="mt-3">
              <Legend />
            </div>
          </F>

          <div className="space-y-4 px-5 py-4">
            {sameVendor && (
              <F id="AGT-011">
                <Note tone="warn" title="Same-vendor review">
                  Codex war beim Start nicht eingerichtet. Alle Stimmen liefen mit Claude Code; die Reviews kommen damit nicht von einem zweiten Vendor.
                </Note>
              </F>
            )}

            {state === 'running' && (
              <F id={['AGT-011', 'HAR-005']}>
                <SectionTitle aside="Fermate bei 30 Min. · Implementierer A">Wartet auf dich</SectionTitle>
                <div className="-ml-9">
                  <ApprovalCard
                    tool="Shell (Implementierer A, Claude Code)"
                    command="git push -u origin beton/maestra-k2q/payment-element"
                    rule="git-push-fragen (Projekt shop-frontend)"
                    reason="Der Implementierer will seinen Branch für den PR pushen. maestra selbst darf weder pushen noch mergen."
                  />
                </div>
                <p className="mt-3 text-[12px] text-muted-foreground">
                  Bis zu deiner Antwort pausiert nur Implementierer A. Danach geht Runde 2 an Codex zum Review.
                </p>
              </F>
            )}

            {state === 'needs-human' && (
              <F id="AGT-011">
                <div className="chamfer border-l-4 border-signal bg-signal-soft p-3 text-[13px]">
                  <p className="font-semibold">Nach 3 Review-Runden ist Teilaufgabe 1 nicht freigegeben</p>
                  <p className="mt-1">
                    Codex bemängelt weiterhin: <em>„confirmPayment wird bei Doppelklick zweimal aufgerufen; der Button wird erst nach der Antwort deaktiviert.“</em>{' '}
                    Claude hält das für ausgeschlossen, weil der Hook entprellt. maestra übergibt die Entscheidung an dich.
                  </p>
                  <div className="mt-2.5 flex flex-wrap gap-2">
                    <Btn variant="primary" className="chamfer-sm rounded-none">
                      Session von Implementierer A übernehmen
                    </Btn>
                    <Btn variant="outline">Eine weitere Runde erlauben</Btn>
                    <Btn variant="ghost">Review-Befund ansehen</Btn>
                  </div>
                </div>
              </F>
            )}

            {(state === 'done' || sameVendor) && (
              <F id={['AGT-011', 'ASY-002']}>
                <SectionTitle aside="letzte Nachricht von maestra">Ergebnis</SectionTitle>
                <AgentMessage harness="claude">
                  <p>Beide Teilaufgaben sind umgesetzt und {sameVendor ? 'reviewt (same-vendor review)' : 'jeweils vom anderen Vendor reviewt'}. Ich habe nichts gemergt.</p>
                  <ul className="list-disc space-y-1 pl-5">
                    <li>
                      <Code>beton/maestra-k2q/payment-element</Code> – Payment Element im Checkout, Button sperrt sofort beim Absenden. Review: 2 Runden.
                    </li>
                    <li>
                      <Code>beton/maestra-k2q/stripe-webhooks</Code> – Handler für <Code>payment_intent.succeeded</Code> und <Code>.payment_failed</Code>, idempotent über die Event-ID.
                    </li>
                  </ul>
                  <p>Offen: Stripe-Webhook-Secret in der Staging-Umgebung hinterlegen.</p>
                </AgentMessage>
                <div className="mt-3 ml-9 flex gap-2">
                  <Btn variant="outline" size="sm">
                    PR für payment-element öffnen
                  </Btn>
                  <Btn variant="outline" size="sm">
                    PR für stripe-webhooks öffnen
                  </Btn>
                </div>
              </F>
            )}
          </div>
        </div>
        <Side state={state} />
      </div>
    </AppLayout>
  )
}
