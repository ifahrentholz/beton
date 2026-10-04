import type { ReactNode } from 'react'
import { Clock, Eye } from 'lucide-react'
import { SessionHeader } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Shell, Tag } from '@/app/kit/policies'
import { scopeLabel, type Scope } from './data'

type Reason = { rule: string; scope: Scope; set: string; text: string; severity?: 'low' | 'medium' | 'high' }

/** Freigabe-Karte mit allen Gründen, Timeout und Merk-Option (POL-010). */
export function PolicyApproval({
  title,
  subject,
  reasons,
  timeout,
  remember = true,
  resolved,
  readOnly,
  children,
}: {
  title: string
  subject: string
  reasons: Reason[]
  timeout?: ReactNode
  remember?: boolean | string
  resolved?: ReactNode
  readOnly?: ReactNode
  children?: ReactNode
}) {
  const open = !resolved
  return (
    <div className="ml-9">
      <div className={cn('border-l-4 p-3', open ? 'chamfer border-signal bg-signal-soft' : 'rounded-r-md border-border bg-card')}>
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-semibold">{open ? 'Freigabe nötig' : 'Freigabe'}: {title}</span>
          {timeout && open && (
            <span className="ml-auto inline-flex items-center gap-1 text-[11px] whitespace-nowrap text-muted-foreground">
              <Clock className="size-3" /> {timeout}
            </span>
          )}
        </div>
        <pre className="mt-1.5 overflow-x-auto rounded-sm bg-card px-2 py-1 font-mono text-[12px]">{subject}</pre>
        <ul className="mt-2 space-y-1">
          {reasons.map((r) => (
            <li key={r.rule} className="flex items-baseline gap-2 text-[13px]">
              <span className="text-muted-foreground">·</span>
              <span className="min-w-0 flex-1">{r.text}</span>
              <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                {scopeLabel[r.scope]} · {r.set}/{r.rule}
              </span>
              {r.severity === 'high' && <Tag mono={false}>hoch</Tag>}
            </li>
          ))}
        </ul>
        {children}
        {resolved && <div className="mt-2 text-[13px]">{resolved}</div>}
        {open && !readOnly && (
          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            <button className="chamfer-sm bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Erlauben</button>
            {remember === true && <button className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">Für diese Session erlauben</button>}
            <button className="rounded-md border border-foreground/30 px-3 py-1.5 text-[13px]">Ablehnen …</button>
            {typeof remember === 'string' && <span className="text-[11px] text-muted-foreground">{remember}</span>}
            <span className="ml-auto text-[11px] text-muted-foreground">
              <kbd className="font-mono">⌘↵</kbd> erlauben · <kbd className="font-mono">⌘⌫</kbd> ablehnen
            </span>
          </div>
        )}
        {open && readOnly && <div className="mt-2.5 flex items-start gap-2 text-[12px] text-muted-foreground">{readOnly}</div>}
      </div>
    </div>
  )
}

export function PolicyApprovals({ state }: { state: string }) {
  const title =
    state === 'session-start'
      ? 'Nächtliches Dependency-Update'
      : state === 'loop'
        ? 'Flaky Test in payment_spec'
        : state === 'view-only'
          ? 'Checkout-Formular barrierefrei machen'
          : 'Rate-Limiter für die Login-API'
  const waiting = !['granted', 'expired'].includes(state)
  return (
    <Shell nav="sessions" activeSession={state === 'loop' ? 'ses_6k8e' : state === 'view-only' ? 'ses_6q2a' : state === 'session-start' ? 'ses_6n1c' : 'ses_7f3k'}>
      <SessionHeader
        title={title}
        harness="claude"
        status={waiting ? 'waiting' : 'idle'}
        branch={title.startsWith('Rate') ? 'beton/rate-limiter-7f3k' : undefined}
        extra={
          state === 'view-only' ? (
            <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground">
              <Eye className="size-3.5" /> geteilt von Mara · du: Lesen
            </span>
          ) : undefined
        }
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          {state === 'two-reasons' && (
            <>
              <UserMessage>Bau das Bundle neu und push den Branch.</UserMessage>
              <ToolCall kind="shell" name="Shell" target="rm -rf dist && pnpm build && git push origin beton/rate-limiter-7f3k" status="waiting" policy="2 Regeln" />
              <F id={['POL-010', 'POL-014', 'POL-006']}>
                <PolicyApproval
                  title="Shell-Kommando"
                  subject="rm -rf dist && pnpm build && git push origin beton/rate-limiter-7f3k"
                  timeout="läuft in 27 Min. ab · dann abbrechen"
                  reasons={[
                    { rule: 'confirm-destructive-shell', scope: 'project', set: 'git-and-budget', text: 'Destruktives oder veröffentlichendes Shell-Kommando', severity: 'high' },
                    { rule: 'git-guard', scope: 'user', set: 'defaults', text: 'Push verlässt deinen Rechner (git_guard: push → ask)' },
                  ]}
                >
                  <p className="mt-2 text-[12px] text-muted-foreground">
                    Zwei Regeln fragen nach – eine Karte. Ablauf nach der kürzesten Frist (30 Min. aus git-and-budget); danach gilt der strengste
                    Wert beider Regeln: abbrechen.
                  </p>
                </PolicyApproval>
              </F>
            </>
          )}

          {state === 'granted' && (
            <F id={['POL-010', 'POL-009', 'POL-006']} className="flex flex-col gap-4">
              <ToolCall kind="git" name="Shell" target="git push origin beton/rate-limiter-7f3k" status="ok" policy="erlaubt von Ingo" duration="1,9 s" />
              <SystemNote>Für diese Session erlaubt: shell · git push origin beton/rate-limiter-7f3k</SystemNote>
              <SystemNote>Inbox: Push Nr. 2 (count-pushes, Projekt)</SystemNote>
              <AgentMessage harness="claude">
                <p>Review-Kommentare eingearbeitet. Ich pushe den Stand.</p>
              </AgentMessage>
              <ToolCall kind="git" name="Shell" target="git push origin beton/rate-limiter-7f3k" status="ok" policy="Freigabe der Session" duration="1,4 s" />
              <SystemNote>Inbox: Push Nr. 3 (count-pushes, Projekt)</SystemNote>
              <ToolCall kind="git" name="Shell" target="git push --tags origin" status="waiting" policy="git-guard" />
              <PolicyApproval
                title="Shell-Kommando"
                subject="git push --tags origin"
                timeout="läuft in 30 Min. ab · dann ablehnen"
                reasons={[{ rule: 'git-guard', scope: 'user', set: 'defaults', text: 'Anderes Kommando – die Freigabe der Session gilt nur für denselben Befehl' }]}
              />
            </F>
          )}

          {state === 'no-remember' && (
            <>
              <UserMessage>Öffne den PR.</UserMessage>
              <ToolCall kind="other" name="MCP github" target="create_pull_request · beton/rate-limiter-7f3k → main" status="waiting" policy="require_approval" />
              <F id={['POL-014', 'POL-010']}>
                <PolicyApproval
                  title="MCP-Tool github/create_pull_request"
                  subject={'repo: ifahrentholz/shop\nhead: beton/rate-limiter-7f3k → base: main\ntitle: "Rate-Limiter für die Login-API"'}
                  timeout="läuft in 1 Std. 58 Min. ab · dann ablehnen"
                  remember="Merken ist für diese Regel abgeschaltet (remember: none)."
                  reasons={[{ rule: 'pr-needs-human', scope: 'team', set: 'plattform-team', text: 'PRs im Team-Repo legt nur ein Mensch frei' }]}
                />
              </F>
            </>
          )}

          {state === 'expired' && (
            <F id={['POL-010']} className="flex flex-col gap-4">
              <ToolCall kind="shell" name="Shell" target="rm -rf node_modules/.cache && git push" status="denied" policy="Zeit abgelaufen" />
              <PolicyApproval
                title="Shell-Kommando"
                subject="rm -rf node_modules/.cache && git push"
                reasons={[
                  { rule: 'confirm-destructive-shell', scope: 'project', set: 'git-and-budget', text: 'Destruktives oder veröffentlichendes Shell-Kommando', severity: 'high' },
                  { rule: 'big-release', scope: 'user', set: 'defaults', text: 'Push nach 18 Uhr bestätigen' },
                ]}
                resolved={
                  <span>
                    Niemand hat innerhalb von 30 Min. entschieden. Angewendet: <b>abbrechen</b> (strengster Wert aus „ablehnen“ und
                    „abbrechen“). Der Turn wurde beendet.
                  </span>
                }
              />
              <SystemNote tone="deny">Turn abgebrochen · Freigabe abgelaufen um 15:42</SystemNote>
            </F>
          )}

          {state === 'view-only' && (
            <>
              <UserMessage author="Mara">Push bitte, dann schauen wir uns die Preview an.</UserMessage>
              <ToolCall kind="git" name="Shell" target="git push origin beton/checkout-a11y-6q2a" status="waiting" policy="git-guard" />
              <F id={['POL-010', 'AUTH-015']}>
                <PolicyApproval
                  title="Shell-Kommando"
                  subject="git push origin beton/checkout-a11y-6q2a"
                  timeout="läuft in 22 Min. ab"
                  reasons={[{ rule: 'git-guard', scope: 'user', set: 'defaults', text: 'Push verlässt Maras Rechner' }]}
                  readOnly={
                    <>
                      <Eye className="mt-0.5 size-3.5 shrink-0" />
                      <span>
                        Du hast diese Session nur zum Lesen. Entscheiden können Mara (Owner) und Personen mit „Kommentieren &amp; freigeben“
                        oder „Mitsteuern“. <button className="underline">Mara um Freigabe bitten</button>
                      </span>
                    </>
                  }
                />
              </F>
            </>
          )}

          {state === 'session-start' && (
            <F id={['POL-003', 'POL-010', 'POL-013']} className="flex flex-col gap-4">
              <SystemNote>Automation „Nächtliches Dependency-Update“ ausgelöst · 02:00</SystemNote>
              <PolicyApproval
                title="Session-Start"
                subject={'Harness: Claude Code · Modell: claude-opus-5-5 · Effort: hoch\nAuslöser: schedule · Sandbox: seatbelt, Preset dev'}
                timeout="läuft in 5 Std. 12 Min. ab · dann ablehnen"
                reasons={[{ rule: 'night-runs', scope: 'user', set: 'defaults', text: 'Async-Sessions mit Opus vor dem Start bestätigen' }]}
              >
                <p className="mt-2 text-[12px] text-muted-foreground">Der Harness ist noch nicht gestartet. Ohne Freigabe startet er nicht.</p>
              </PolicyApproval>
            </F>
          )}

          {state === 'loop' && (
            <F id={['POL-018', 'POL-010']} className="flex flex-col gap-4">
              <ToolCall kind="shell" name="Shell" target="pnpm vitest run payment_spec" status="failed" duration="8,1 s" />
              <ToolCall kind="edit" name="Bearbeiten" target="src/payment/retry.ts" duration="0,2 s" />
              <ToolCall kind="shell" name="Shell" target="pnpm vitest run payment_spec" status="failed" duration="8,3 s" />
              <ToolCall kind="shell" name="Shell" target="pnpm vitest run payment_spec" status="waiting" policy="loop" />
              <PolicyApproval
                title="Agent wiederholt sich"
                subject="pnpm vitest run payment_spec  (3. identischer Aufruf in den letzten 10)"
                timeout="läuft in 30 Min. ab · dann ablehnen"
                reasons={[{ rule: 'loop', scope: 'user', set: 'defaults', text: 'Gleicher Aufruf 3-mal im Fenster von 10 Tool-Calls' }]}
              >
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Tipp: Lehne ab und gib dem Agent einen Hinweis mit, z. B. „Der Test ist zeitabhängig, mocke die Uhr“.
                </p>
              </PolicyApproval>
            </F>
          )}

          {state === 'spend' && (
            <F id={['POL-011', 'POL-010']} className="flex flex-col gap-4">
              <AgentMessage harness="claude">
                <p>Ich prüfe jetzt alle 38 Aufrufer der alten Auth-API.</p>
              </AgentMessage>
              <PolicyApproval
                title="Kostenschwelle erreicht"
                subject="Session: 10,12 USD von 25 USD (spend_cap, Schwelle 10 USD)"
                timeout="läuft in 30 Min. ab · dann ablehnen"
                reasons={[{ rule: 'budget', scope: 'project', set: 'git-and-budget', text: 'Kostengrenze der Session: Rückfrage bei 10 und 20 USD' }]}
              >
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Claude Max ist eine Subscription: gezählt wird das Listenpreis-Äquivalent laut Modellkatalog, nicht deine Rechnung.
                </p>
              </PolicyApproval>
            </F>
          )}
        </div>
      </div>
    </Shell>
  )
}
