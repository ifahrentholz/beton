import { Bell, ChevronLeft, Eye, Inbox, Smartphone } from 'lucide-react'
import { HarnessBadge } from '@/app/harness'
import { AgentMessage, ApprovalCard, SystemNote, ToolCall } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { HarnessId } from '@/mock/data'
import { Btn, Code, Mark, Meter, Note, Row, SectionTitle, type MarkKind } from '@/app/kit/harnesses'
import { AutomationShell, RunStatusMark, TriggerLabel, type RunStatus, type TriggerKind } from './parts'

type Step = { label: string; time: string; kind: MarkKind; detail?: string }

type RunInfo = {
  id: string
  agent: string
  task: string
  harness: HarnessId
  model: string
  status: RunStatus
  statusExtra?: string
  trigger: TriggerKind
  triggerName: string
  steps: Step[]
}

function info(state: string): RunInfo {
  switch (state) {
    case 'running':
      return {
        id: 'run_99y', agent: 'pr-fixer', task: 'CI auf fix/checkout-timeout (PR 418)', harness: 'claude', model: 'claude-sonnet-5-5', status: 'running', trigger: 'webhook', triggerName: 'ci-failed · PR 418',
        steps: [
          { label: 'Eingereiht', time: '07:38:04', kind: 'ok' },
          { label: 'Diesem Mac zugeteilt', time: '07:38:04', kind: 'ok' },
          { label: 'Läuft', time: '07:38:06', kind: 'run', detail: 'niemand schaut zu' },
        ],
      }
    case 'completed':
      return {
        id: 'run_98e', agent: 'pr-fixer', task: 'CI von develop prüfen und beheben', harness: 'claude', model: 'claude-sonnet-5-5', status: 'completed', statusExtra: 'PR #417', trigger: 'schedule', triggerName: 'nightly-ci · manuell gestartet',
        steps: [
          { label: 'Eingereiht', time: 'Fr. 14:22', kind: 'ok' },
          { label: 'Diesem Mac zugeteilt', time: '14:22', kind: 'ok' },
          { label: 'Läuft', time: '14:22', kind: 'ok' },
          { label: 'Fertig', time: '14:43', kind: 'ok', detail: '21 Min.' },
        ],
      }
    case 'budget-exceeded':
      return {
        id: 'run_97b', agent: 'dep-updater', task: 'Abhängigkeiten aktualisieren', harness: 'codex', model: 'gpt-5.3-codex', status: 'budget_exceeded', statusExtra: '150 von 150 Turns', trigger: 'schedule', triggerName: 'weekly-deps',
        steps: [
          { label: 'Eingereiht', time: '26.09. 07:30', kind: 'ok' },
          { label: 'Läuft', time: '07:30', kind: 'ok' },
          { label: 'Hinweis bei 80 %', time: '08:14', kind: 'ok', detail: '120 Turns' },
          { label: 'Budget erreicht, beendet', time: '08:28', kind: 'deny' },
        ],
      }
    case 'no-runner':
      return {
        id: 'run_a02', agent: 'pr-fixer', task: 'CI auf release/2.4 prüfen', harness: 'claude', model: 'claude-sonnet-5-5', status: 'failed', statusExtra: 'kein Runner', trigger: 'webhook', triggerName: 'ci-failed · release/2.4',
        steps: [
          { label: 'Eingereiht', time: '07:20', kind: 'ok' },
          { label: 'Sucht Runner os=linux,repo=beton', time: '07:20 – 07:30', kind: 'deny', detail: 'keiner online' },
          { label: 'Fehlgeschlagen', time: '07:30', kind: 'deny' },
        ],
      }
    default: {
      const timedOut = state === 'timeout-denied'
      return {
        id: 'run_9a1', agent: 'pr-fixer', task: 'CI von develop prüfen und beheben', harness: 'claude', model: 'claude-sonnet-5-5', status: timedOut ? 'running' : 'paused_approval', statusExtra: timedOut ? undefined : 'seit 03:14', trigger: 'schedule', triggerName: 'nightly-ci',
        steps: [
          { label: 'Eingereiht', time: '03:00:00', kind: 'ok' },
          { label: 'Diesem Mac zugeteilt', time: '03:00:01', kind: 'ok' },
          { label: 'Läuft', time: '03:00:02', kind: 'ok' },
          { label: 'Pausiert: wartet auf Freigabe', time: '03:14', kind: timedOut ? 'ok' : 'wait', detail: timedOut ? undefined : 'Push und Inbox gesendet' },
          ...(timedOut ? [{ label: 'Ohne Antwort abgelehnt, läuft weiter', time: '11:14', kind: 'run' as MarkKind }] : []),
        ],
      }
    }
  }
}

function Steps({ steps }: { steps: Step[] }) {
  return (
    <ol className="space-y-0">
      {steps.map((s, i) => (
        <li key={i} className="relative flex gap-3 pb-3 last:pb-0">
          {i < steps.length - 1 && <span aria-hidden className="absolute top-4 bottom-0 left-[6px] w-px bg-border" />}
          <Mark kind={s.kind} />
          <div className="min-w-0 text-[13px]">
            <div className={cn(s.kind === 'wait' && 'font-semibold')}>{s.label}</div>
            <div className="text-[11.5px] text-muted-foreground tabular-nums">
              {s.time}
              {s.detail && ` · ${s.detail}`}
            </div>
          </div>
        </li>
      ))}
    </ol>
  )
}

export function RunDetail({ state }: { state: string }) {
  const r = info(state)
  const exceeded = state === 'budget-exceeded'
  return (
    <AutomationShell tab="runs">
      <div className="flex shrink-0 items-start gap-4 border-b border-border px-6 py-3">
        <div className="min-w-0 flex-1">
          <button className="mb-0.5 inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground">
            <ChevronLeft className="size-3.5" /> Läufe
          </button>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <h3 className="type-wide text-[17px] font-[700]">{r.agent}</h3>
            <span className="font-mono text-[11px] text-muted-foreground">{r.id}</span>
            <RunStatusMark status={r.status} extra={r.statusExtra} />
            <HarnessBadge id={r.harness} model={r.model} />
          </div>
          <p className="text-[13px] text-muted-foreground">{r.task}</p>
        </div>
        <div className="flex gap-2 pt-4">
          {(state === 'running' || state === 'paused' || state === 'timeout-denied') && (
            <>
              <Btn variant="outline">
                <Eye className="size-3.5" /> Live zuschauen
              </Btn>
              <Btn variant="outline">Session übernehmen</Btn>
              <Btn variant="ghost">Lauf abbrechen</Btn>
            </>
          )}
          {state === 'no-runner' && <Btn variant="outline">Erneut einreihen</Btn>}
          {state === 'completed' && <Btn variant="outline">Session öffnen</Btn>}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="grid grid-cols-[220px_minmax(0,1fr)_300px] gap-8 px-6 py-5">
          <F id="ASY-001" as="section">
            <SectionTitle>Verlauf des Laufs</SectionTitle>
            <Steps steps={r.steps} />
          </F>

          <div className="min-w-0 space-y-4">
            {state === 'paused' && (
              <F id={['ASY-008', 'ASY-009']}>
                <SectionTitle aside="Regel git-push-fragen · Projekt shop-frontend">Wartet auf dich</SectionTitle>
                <div className="-ml-9">
                  <ApprovalCard
                    tool="Shell"
                    command="git push -u origin beton/pr-fixer-9a1/develop-ci"
                    rule="git-push-fragen"
                    reason="Der Agent hat den Fehler in der CI behoben und will den Branch pushen, um einen PR zu öffnen."
                  />
                </div>
                <dl className="mt-3 divide-y divide-border border-y border-border">
                  <Row label="Ohne Antwort">
                    Um <strong>11:14</strong> abgelehnt (in 3 Std. 32 Min.). Der Agent macht dann ohne Push weiter.
                  </Row>
                  <Row label="Benachrichtigt">
                    <span className="inline-flex flex-wrap items-center gap-3">
                      <span className="inline-flex items-center gap-1">
                        <Inbox className="size-3.5" /> Inbox
                      </span>
                      <span className="inline-flex items-center gap-1">
                        <Bell className="size-3.5" /> Desktop
                      </span>
                      <span className="inline-flex items-center gap-1">
                        <Smartphone className="size-3.5" /> Handy (PWA)
                      </span>
                    </span>
                  </Row>
                  <Row label="Während der Pause">Kein Modell-Traffic. Claude Code wurde nach 1 Std. beendet und wird beim Fortsetzen mit vollem Verlauf neu gestartet.</Row>
                </dl>
              </F>
            )}

            {state === 'timeout-denied' && (
              <F id="ASY-008">
                <ToolCall kind="git" name="Shell" target="git push -u origin beton/pr-fixer-9a1/develop-ci" status="denied" policy="git_guard" />
                <div className="mt-3">
                  <SystemNote>Keine Antwort bis 11:14 · Freigabe abgelehnt (Zeitablauf) · der Lauf geht weiter</SystemNote>
                </div>
                <div className="mt-3">
                  <AgentMessage harness="claude" streaming>
                    <p>Der Push wurde nicht freigegeben. Ich lasse den Commit auf dem lokalen Branch und schreibe in die Zusammenfassung, was noch fehlt.</p>
                  </AgentMessage>
                </div>
              </F>
            )}

            {state === 'running' && (
              <F id="ASY-001">
                <Note title="Läuft ohne Zuschauer">Du kannst jederzeit zuschauen oder die Session übernehmen; du siehst sie dann vollständig von Anfang an.</Note>
                <div className="mt-3 flex flex-col gap-1">
                  <ToolCall kind="shell" name="Shell" target="gh pr checks 418" duration="1,2 s" />
                  <ToolCall kind="read" name="Lesen" target="e2e/checkout.spec.ts" duration="0,1 s" />
                  <ToolCall kind="shell" name="Shell" target="pnpm playwright test checkout.spec.ts" status="running" />
                </div>
              </F>
            )}

            {state === 'completed' && (
              <F id={['ASY-001', 'ASY-009']}>
                <SectionTitle aside="letzte Nachricht des Agents">Ergebnis</SectionTitle>
                <AgentMessage harness="claude">
                  <p>
                    Die CI auf develop war rot, weil <Code>checkout.spec.ts</Code> auf eine feste Wartezeit setzte. Ich habe auf{' '}
                    <Code>waitForResponse</Code> umgestellt; alle 214 Tests laufen. PR #417 ist offen, nicht gemergt.
                  </p>
                </AgentMessage>
                <p className="mt-3 ml-9 text-[12px] text-muted-foreground">Zugestellt als Inbox-Eintrag „pr-fixer fertig“ · gelesen 14:51</p>
              </F>
            )}

            {exceeded && (
              <F id={['ASY-010', 'ASY-009']}>
                <Note tone="deny" title="Beendet: 150 von 150 Turns erreicht">
                  dep-updater hat 14 von 23 Paketgruppen aktualisiert. Der laufende Turn wurde abgebrochen; die fertigen Gruppen liegen als Commits auf{' '}
                  <Code>beton/dep-updater-97b</Code>. Ein Inbox-Eintrag wurde erstellt.
                </Note>
                <div className="mt-3 flex gap-2">
                  <Btn variant="outline">Mit doppeltem Budget fortsetzen</Btn>
                  <Btn variant="ghost">Grenzen im Schedule ändern</Btn>
                </div>
              </F>
            )}

            {state === 'no-runner' && (
              <F id={['ASY-006', 'ASY-009']}>
                <Note tone="deny" title="Kein passender Runner online">
                  Der Lauf verlangt <Code>os=linux,repo=beton</Code>. build-linux-01 und build-linux-02 waren in den 10 Minuten offline; dein Mac passt nicht
                  (os=macos). Starte einen der Runner oder ändere den Selektor im Trigger.
                </Note>
              </F>
            )}
          </div>

          <div className="space-y-6">
            <F id="ASY-010" as="section">
              <SectionTitle aside="inkl. Sub-Agents">Grenzen</SectionTitle>
              <div className="space-y-3">
                <Meter value={exceeded ? 58 : 14} max={60} label="Laufzeit" detail={exceeded ? '58 von 60 Min.' : '14 von 60 Min.'} />
                <Meter value={exceeded ? 150 : 41} max={150} label="Turns" detail={exceeded ? '150 von 150' : '41 von 150'} tone={exceeded ? 'deny' : undefined} />
                <Meter value={exceeded ? 312 : 96} max={400} label="Tool-Calls" detail={exceeded ? '312 von 400' : '96 von 400'} />
                <p className="text-[12px] text-muted-foreground">
                  {r.harness === 'codex' ? 'ChatGPT Pro · 9 % des Fensters' : 'Claude Max · 3 % des Fensters'} · Kosten 0 € (Abo)
                </p>
              </div>
            </F>
            <F id={['ASY-001', 'ASY-006']} as="section">
              <SectionTitle>Auslöser</SectionTitle>
              <dl className="space-y-1.5 text-[13px]">
                <div>
                  <TriggerLabel kind={r.trigger} name={r.triggerName} />
                </div>
                <div className="text-muted-foreground">{state === 'no-runner' ? 'Runner: os=linux,repo=beton (Team-Server)' : 'Runner: dieser Mac'}</div>
                <div className="text-muted-foreground">Ergebnis an: Inbox, Desktop-Benachrichtigung</div>
              </dl>
            </F>
          </div>
        </div>
      </div>
    </AutomationShell>
  )
}
