import type { ReactNode } from 'react'
import { Braces, FlaskConical, HelpCircle, X } from 'lucide-react'
import { SessionHeader } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, C, Shell, Tag, Verdict, type VerdictKind } from './kit'

type Hit = { scope: string; set?: string; rule?: string; v: VerdictKind; expr?: ReactNode; note?: ReactNode }

function Step({ n, title, children, last }: { n: number; title: string; children: ReactNode; last?: boolean }) {
  return (
    <div className="relative flex gap-3 pb-3">
      {!last && <span className="absolute top-5 bottom-0 left-[9px] w-px bg-border" />}
      <span className="relative z-10 flex size-5 shrink-0 items-center justify-center rounded-full border border-border bg-background text-[10px] font-semibold tabular-nums">
        {n}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-semibold">{title}</div>
        <div className="mt-1 text-[12px]">{children}</div>
      </div>
    </div>
  )
}

function Hits({ hits }: { hits: Hit[] }) {
  return (
    <div className="divide-y divide-border/70 rounded-md border border-border bg-card">
      {hits.map((h, i) => (
        <div key={i} className="grid grid-cols-[64px_minmax(0,1fr)_auto] gap-x-3 px-2.5 py-1.5">
          <span className="font-medium">{h.scope}</span>
          <div className="min-w-0">
            {h.rule ? (
              <span className="font-mono">
                {h.set && <span className="text-muted-foreground">{h.set} / </span>}
                {h.rule}
              </span>
            ) : (
              <span className="text-muted-foreground">keine passende Regel</span>
            )}
            {h.expr && <div className="mt-0.5 font-mono text-[11px] text-muted-foreground">{h.expr}</div>}
            {h.note && <div className="mt-0.5 text-[11px] text-muted-foreground">{h.note}</div>}
          </div>
          <Verdict v={h.v} />
        </div>
      ))}
    </div>
  )
}

function ExplainPanel({ state }: { state: string }) {
  if (state === 'eval') return <EvalPanel />
  const deny = state === 'deny'
  const modify = state === 'modify'
  const dflt = state === 'default'
  return (
    <F id={['POL-027', 'POL-025', 'POL-007']} className="flex h-full flex-col">
      <div className="flex items-start gap-2 border-b border-border px-4 py-3">
        <div className="min-w-0 flex-1">
          <h3 className="type-wide text-[15px] font-semibold">
            {deny ? 'Warum wurde das abgelehnt?' : modify ? 'Warum wurde das Modell gewechselt?' : 'Warum wurde die Navigation abgelehnt?'}
          </h3>
          <p className="text-[12px] text-muted-foreground">
            Rekonstruiert aus <span className="font-mono">policy.decision</span> seq {deny ? 1907 : modify ? 2214 : 1960} und dem damals gültigen Set
          </p>
        </div>
        <button aria-label="Schließen" className="rounded-md p-1 text-muted-foreground hover:bg-accent">
          <X className="size-4" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        <Step n={1} title="Anfrage, normalisiert">
          {deny && (
            <F id="POL-005">
              <p>
                Phase <C>tool_call</C> · Claude Code <C>Bash</C> → <C>tool.kind = shell</C>
              </p>
              <div className="mt-1 rounded-md border border-border bg-card px-2.5 py-1.5 font-mono text-[11px]">
                <div className="text-muted-foreground">tool.argv (2 Teilkommandos)</div>
                <div>1 · cd packages/api</div>
                <div>2 · git push --force-with-lease origin main</div>
                <div className="mt-1 text-muted-foreground">erkannt: force_push, Ziel-Branch main (geschützt) · parse_uncertain = false</div>
              </div>
            </F>
          )}
          {modify && (
            <p>
              Phase <C>model_request</C> (pro Turn) · angefragt <C>claude-opus-5-5</C>, Effort hoch · <C>session.cost_usd = 21,40</C>
            </p>
          )}
          {dflt && (
            <p>
              Phase <C>browser_navigate</C> · Agent „pr-fixer“ will <C>https://github.com/ifahrentholz/shop/pulls</C> öffnen
            </p>
          )}
        </Step>
        <Step n={2} title="Effektives Set">
          <p>
            5 Ebenen, 17 Regeln · <span className="font-mono">policy_set_hash sha256:4be1…c0</span>
          </p>
          <p className="text-muted-foreground">
            Version vom 03.10. 14:02 – die Projekt-Datei wurde seitdem geändert; gezeigt wird die damals gültige Fassung.
          </p>
        </Step>
        <Step n={3} title="Durchgang 1 – Änderungen">
          {modify ? (
            <Hits
              hits={[
                { scope: 'Org', set: 'acme-baseline', rule: 'models', v: 'modify', expr: 'allowed_models → Opus nicht erlaubt, Route: claude-sonnet-5-5' },
                {
                  scope: 'Projekt',
                  set: 'git-and-budget',
                  rule: 'route-when-expensive',
                  v: 'modify',
                  expr: 'session.cost_usd > 20.0 → true',
                  note: 'Wollte claude-sonnet-5-5 setzen; erster Schreiber gewinnt (Org), kein Konflikt.',
                },
                {
                  scope: 'Agent',
                  set: 'pr-fixer',
                  rule: 'fast-model',
                  v: 'modify',
                  note: (
                    <>
                      Wollte <C>claude-haiku-4-5</C> setzen – verworfen, Org hat zuerst geschrieben. Steht in <C>conflicts</C>.
                    </>
                  ),
                },
              ]}
            />
          ) : (
            <p className="text-muted-foreground">Keine Änderungsregel hat gegriffen.</p>
          )}
        </Step>
        <Step n={4} title="Durchgang 2 – Entscheidungen je Ebene">
          {deny && (
            <Hits
              hits={[
                {
                  scope: 'Org',
                  set: 'acme-baseline',
                  rule: 'no-force-push',
                  v: 'deny',
                  expr: (
                    <>
                      shell.has_flag(tool.argv, "--force-with-lease") → <b className="text-foreground">true</b>
                    </>
                  ),
                },
                { scope: 'Team', v: 'skip' },
                { scope: 'User', set: 'defaults', rule: 'git-guard', v: 'deny', expr: 'force_push: deny (eingebauter Typ git_guard)' },
                { scope: 'Projekt', set: 'git-and-budget', rule: 'confirm-destructive-shell', v: 'ask', expr: 'tool.command.matches("rm -rf|git push") → true' },
                { scope: 'Projekt', set: 'git-and-budget', rule: 'count-pushes', v: 'notify', note: 'Meldung wird trotzdem ausgeführt; Zähler zählt nur bei allow/deny.' },
                { scope: 'Agent', v: 'skip', note: 'Session ohne Agent' },
              ]}
            />
          )}
          {modify && (
            <Hits
              hits={[
                { scope: 'Org', set: 'acme-baseline', rule: 'team-day', v: 'allow', expr: 'team: 182 von 300 USD' },
                { scope: 'User', set: 'defaults', rule: 'my-day', v: 'allow', expr: 'user.daily_cost_usd = 24,10 < 30' },
                { scope: 'Projekt', set: 'git-and-budget', rule: 'budget', v: 'allow', expr: 'spend_cap: 21,40 von 25 USD, Schwelle 20 schon gefragt' },
              ]}
            />
          )}
          {dflt && (
            <Hits
              hits={[
                { scope: 'Team', set: 'plattform-team', rule: 'browser', v: 'allow', expr: 'navigate.allow: "*.github.com" → true' },
                { scope: 'Projekt', set: 'git-and-budget', rule: 'allow-docs', v: 'skip', expr: 'glob(browser.host, "*.rust-lang.org") → false' },
                { scope: 'Agent', set: 'pr-fixer', rule: 'allow-github', v: 'allow', expr: 'browser.host == "github.com" → true' },
              ]}
            />
          )}
        </Step>
        <Step n={5} title="Defaults">
          {dflt ? (
            <div className="space-y-1">
              <div className="flex items-center gap-2">
                <span className="w-16 font-medium">Projekt</span>
                <C>browser_navigate: deny</C>
                <Verdict v="deny" label="nicht aufgehoben" className="ml-auto" />
              </div>
              <p className="text-muted-foreground">
                Kein „allow“ derselben Ebene (Projekt) passt. Das „allow“ aus Team und Agent hebt den Projekt-Default nicht auf –
                Allowlists verschiedener Ebenen wirken als Schnittmenge.
              </p>
            </div>
          ) : (
            <p className="text-muted-foreground">Alle Ebenen: Default „allow“ für diese Phase.</p>
          )}
        </Step>
        <Step n={6} title="Freigaben und Fehler">
          <p className="text-muted-foreground">Keine gemerkte Freigabe passt · keine Auswertungsfehler · Durchsetzung: voll (Claude Code nativ)</p>
        </Step>
        <Step n={7} title="Ergebnis" last>
          <div className={cn('rounded-md border p-2.5', deny || dflt ? 'border-deny/50 bg-deny-soft' : 'border-border bg-card')}>
            {deny && (
              <>
                <Verdict v="deny" className="font-semibold" />
                <p className="mt-1">
                  Primärer Grund: Org-Regel <span className="font-mono">no-force-push</span>. Weitere Gründe: User <span className="font-mono">git-guard</span>.
                  Die Rückfrage aus dem Projekt entfällt, weil eine Ablehnung stärker ist.
                </p>
                <p className="mt-1.5 text-muted-foreground">An das Modell: „Force-Push ist in der Org verboten. Pushe auf einen Feature-Branch und öffne einen PR.“</p>
              </>
            )}
            {modify && (
              <>
                <Verdict v="allow" label="erlaubt, mit Änderung" className="font-semibold" />
                <p className="mt-1">
                  Modell <C>claude-opus-5-5</C> → <C>claude-sonnet-5-5</C> (live gewechselt). 1 Konflikt protokolliert.
                </p>
              </>
            )}
            {dflt && (
              <>
                <Verdict v="deny" className="font-semibold" />
                <p className="mt-1">Projekt-Default „browser_navigate: deny“. Lösung: im Projekt eine allow-Regel für github.com ergänzen.</p>
              </>
            )}
          </div>
        </Step>
      </div>
      <div className="flex flex-wrap items-center gap-2 border-t border-border px-4 py-2.5">
        <Btn>
          <FlaskConical className="size-3.5" /> Als Policy-Test übernehmen
        </Btn>
        <Btn>Regel öffnen</Btn>
        <Btn tone="quiet">
          <Braces className="size-3.5" /> Ereignis als JSON
        </Btn>
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">beton policy explain --session ses_7f3k --seq {deny ? 1907 : modify ? 2214 : 1960}</span>
      </div>
    </F>
  )
}

function EvalPanel() {
  return (
    <F id="POL-027" className="flex h-full flex-col">
      <div className="border-b border-border px-4 py-3">
        <h3 className="type-wide text-[15px] font-semibold">Was wäre, wenn …</h3>
        <p className="text-[12px] text-muted-foreground">Wertet eine gedachte Anfrage gegen die Policies dieser Session aus. Erzeugt keine Events, keine Freigaben, ändert keinen Zustand.</p>
      </div>
      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-3 text-[12px]">
        <div className="grid grid-cols-[90px_1fr] items-center gap-2">
          <span className="text-muted-foreground">Phase</span>
          <span className="rounded-md border border-input bg-card px-2 py-1 font-mono">tool_call</span>
          <span className="text-muted-foreground">Harness</span>
          <span className="rounded-md border border-input bg-card px-2 py-1">Codex (nativ)</span>
          <span className="text-muted-foreground">Tool</span>
          <span className="rounded-md border border-input bg-card px-2 py-1 font-mono">exec_command</span>
          <span className="self-start pt-1 text-muted-foreground">Kommando</span>
          <span className="rounded-md border border-input bg-card px-2 py-1.5 font-mono">git push origin feature/rate-limit</span>
          <span className="text-muted-foreground">Kontext</span>
          <span>Session „Rate-Limiter für die Login-API“ (Kosten, Zähler, Branch)</span>
        </div>
        <button className="rounded-md bg-foreground px-3 py-1.5 text-[13px] font-semibold text-background">Auswerten</button>
        <div className="space-y-2 border-t border-border pt-3">
          <div className="flex items-center gap-2">
            <span className="font-semibold">Ergebnis:</span> <Verdict v="ask" />
            <Tag mono={false} className="ml-auto">
              keine Seiteneffekte
            </Tag>
          </div>
          <Hits
            hits={[
              { scope: 'User', set: 'defaults', rule: 'git-guard', v: 'ask', expr: 'push: ask · Ziel feature/rate-limit nicht geschützt' },
              { scope: 'Projekt', set: 'git-and-budget', rule: 'confirm-destructive-shell', v: 'ask' },
              { scope: 'Projekt', set: 'shadow-npm', rule: 'no-npm-publish', v: 'skip', note: 'Shadow-Regel, beeinflusst das Ergebnis nicht' },
            ]}
          />
          <p className="text-muted-foreground">
            Durchsetzung bei Codex nativ: „nur Freigabe“ für Shell – eine Rückfrage ist möglich. Agents erreichen dieselbe Auswertung über das
            System-Tool <C>policy_query</C>.
          </p>
        </div>
      </div>
      <div className="border-t border-border px-4 py-2 font-mono text-[11px] text-muted-foreground">beton policy eval --phase tool_call --input req.json --session ses_7f3k</div>
    </F>
  )
}

export function PolicyExplain({ state }: { state: string }) {
  return (
    <Shell nav="sessions" sessionList={false}>
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <SessionHeader title="Rate-Limiter für die Login-API" harness="claude" status="idle" branch="beton/rate-limiter-7f3k" />
          <div className="min-h-0 flex-1 overflow-y-auto">
            <div className="mx-auto flex max-w-2xl flex-col gap-3 px-6 py-5">
              {state === 'modify' ? (
                <>
                  <UserMessage>Lies dir bitte die ganze Middleware-Kette noch einmal gründlich durch.</UserMessage>
                  <F id="POL-013">
                    <SystemNote>Modell gewechselt: claude-opus-5-5 → claude-sonnet-5-5 · Policy „models“ (Org)</SystemNote>
                  </F>
                </>
              ) : state === 'default' ? (
                <>
                  <UserMessage>Schau nach, ob es zu dem Thema schon offene PRs gibt.</UserMessage>
                  <ToolCall kind="other" name="Browser" target="navigate https://github.com/ifahrentholz/shop/pulls" status="denied" policy="Projekt-Default" />
                </>
              ) : (
                <>
                  <UserMessage>Bitte den Fix direkt auf main pushen, ich brauche das heute noch.</UserMessage>
                  <F id="POL-017">
                    <ToolCall kind="git" name="Shell" target="cd packages/api && git push --force-with-lease origin main" status="denied" policy="no-force-push" />
                  </F>
                </>
              )}
              <div className="ml-9 flex items-center gap-2 text-[12px]">
                <span className={state === 'modify' ? 'text-muted-foreground' : 'text-deny'}>
                  {state === 'modify' ? 'Von der Policy geändert.' : 'Von der Policy abgelehnt.'}
                </span>
                <button className="inline-flex items-center gap-1 rounded-md border border-foreground/30 bg-card px-2 py-0.5">
                  <HelpCircle className="size-3.5" /> Warum?
                </button>
              </div>
              {state !== 'modify' && (
                <AgentMessage harness="claude">
                  {state === 'default' ? (
                    <p>Ich darf github.com in diesem Projekt nicht im Browser öffnen. Soll ich stattdessen <code className="rounded-sm bg-muted px-1 text-[13px]">gh pr list</code> nutzen?</p>
                  ) : (
                    <p>
                      Ein Force-Push auf <code className="rounded-sm bg-muted px-1 text-[13px]">main</code> ist in eurer Org nicht erlaubt. Ich habe
                      stattdessen den Branch <code className="rounded-sm bg-muted px-1 text-[13px]">beton/rate-limiter-7f3k</code> vorbereitet – soll
                      ich ihn pushen und einen PR öffnen?
                    </p>
                  )}
                </AgentMessage>
              )}
            </div>
          </div>
        </div>
        <aside className="w-[480px] shrink-0 border-l border-border bg-background">
          <ExplainPanel state={state} />
        </aside>
      </div>
    </Shell>
  )
}
