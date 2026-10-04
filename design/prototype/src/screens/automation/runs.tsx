import { Moon, Plus } from 'lucide-react'
import { VoiceDot } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { harnesses, type HarnessId } from '@/mock/data'
import { Btn, Code, Note, Segmented } from '@/app/kit/harnesses'
import { AutomationShell, RunStatusMark, TriggerLabel, type RunStatus, type TriggerKind } from './parts'

type Run = {
  id: string
  agent: string
  task: string
  harness: HarnessId
  trigger: TriggerKind
  triggerName: string
  status: RunStatus
  statusExtra?: string
  start: string
  duration: string
  usage: string
  runner: string
}

const RUNS: Run[] = [
  { id: 'run_9a1', agent: 'pr-fixer', task: 'CI von develop prüfen und beheben', harness: 'claude', trigger: 'schedule', triggerName: 'nightly-ci', status: 'paused_approval', statusExtra: 'seit 03:14', start: '03:00', duration: '14 Min. + Pause', usage: 'Claude Max · 3 %', runner: 'dieser Mac' },
  { id: 'run_9a0', agent: 'dep-updater', task: 'Abhängigkeiten aktualisieren', harness: 'codex', trigger: 'schedule', triggerName: 'weekly-deps', status: 'running', start: '07:30', duration: '12 Min.', usage: 'ChatGPT Pro · 2 %', runner: 'dieser Mac' },
  { id: 'run_99y', agent: 'pr-fixer', task: 'CI auf fix/checkout-timeout (PR 418)', harness: 'claude', trigger: 'webhook', triggerName: 'ci-failed', status: 'running', start: '07:38', duration: '4 Min.', usage: 'Claude Max · 1 %', runner: 'dieser Mac' },
  { id: 'run_99z', agent: 'pr-fixer', task: 'CI auf feat/coupons (PR 421)', harness: 'claude', trigger: 'webhook', triggerName: 'ci-failed', status: 'queued', statusExtra: 'Position 1', start: '–', duration: '–', usage: '–', runner: '–' },
  { id: 'run_98k', agent: 'a11y-auditor', task: 'Checkout-Formular erneut prüfen', harness: 'gemini', trigger: 'timer', triggerName: 'in 2 Std. · Session Checkout a11y', status: 'completed', start: '06:58', duration: '6 Min.', usage: 'Google-Konto', runner: 'dieser Mac' },
  { id: 'run_97b', agent: 'dep-updater', task: 'Abhängigkeiten aktualisieren', harness: 'codex', trigger: 'schedule', triggerName: 'weekly-deps', status: 'budget_exceeded', statusExtra: '150 von 150 Turns', start: '26.09. 07:30', duration: '58 Min.', usage: 'ChatGPT Pro · 9 %', runner: 'dieser Mac' },
  { id: 'run_96c', agent: 'local-scout', task: 'Übersicht für infra schreiben', harness: 'ollama', trigger: 'spawn', triggerName: 'aus Session Terraform-Plan', status: 'failed', statusExtra: 'Ollama nicht erreichbar', start: 'Fr. 16:20', duration: '0 Min.', usage: '0 €', runner: 'dieser Mac' },
  { id: 'run_95d', agent: 'pr-fixer', task: 'CI von develop prüfen und beheben', harness: 'claude', trigger: 'schedule', triggerName: 'nightly-ci', status: 'timed_out', statusExtra: 'nach 2 Std.', start: 'Fr. 03:00', duration: '2 Std.', usage: 'Claude Max · 11 %', runner: 'dieser Mac' },
]

const SLEEP: Run[] = [
  { id: 'run_9b2', agent: 'pr-fixer', task: 'Nachgeholt: 3 verpasste Termine (Do.–Sa. 03:00)', harness: 'claude', trigger: 'schedule', triggerName: 'nightly-ci', status: 'running', statusExtra: 'nachgeholt', start: '07:32', duration: '3 Min.', usage: 'Claude Max · 1 %', runner: 'dieser Mac' },
  { id: 'sk_1', agent: 'weekly-report', task: 'Wochenbericht', harness: 'claude', trigger: 'schedule', triggerName: 'weekly-report', status: 'skipped', statusExtra: 'Rechner schlief, catch_up: skip', start: 'Fr. 18:00', duration: '–', usage: '–', runner: '–' },
]

function RunTable({ runs, highlight }: { runs: Run[]; highlight?: string }) {
  return (
    <table className="type-narrow w-full text-[12.5px]">
      <thead>
        <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
          <th className="py-2 pl-6 font-normal">Status</th>
          <th className="font-normal">Agent · Aufgabe</th>
          <th className="font-normal">Auslöser</th>
          <th className="font-normal">Start</th>
          <th className="font-normal">Dauer</th>
          <th className="font-normal">Verbrauch</th>
          <th className="pr-6 font-normal">Runner</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-border">
        {runs.map((r) => {
          const row = (
            <>
              <td className="py-2.5 pl-6 whitespace-nowrap">
                <RunStatusMark status={r.status} extra={r.statusExtra} />
              </td>
              <td className="max-w-[300px] py-2.5 pr-3">
                <div className="flex items-center gap-1.5 font-medium">
                  <VoiceDot voice={harnesses[r.harness].voice} />
                  {r.agent}
                  <span className="font-mono text-[10.5px] font-normal text-muted-foreground">{r.id}</span>
                </div>
                <div className="truncate pl-3.5 text-muted-foreground">{r.task}</div>
              </td>
              <td className="max-w-[220px] pr-3">
                <TriggerLabel kind={r.trigger} name={r.triggerName} />
              </td>
              <td className="pr-3 whitespace-nowrap tabular-nums">{r.start}</td>
              <td className="pr-3 whitespace-nowrap tabular-nums">{r.duration}</td>
              <td className="pr-3 whitespace-nowrap">{r.usage}</td>
              <td className="pr-6 whitespace-nowrap text-muted-foreground">{r.runner}</td>
            </>
          )
          return (
            <tr key={r.id} className={cn(r.status === 'paused_approval' && 'bg-signal-soft/60', highlight === r.id && 'bg-accent', r.status === 'skipped' && 'text-muted-foreground')}>
              {row}
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

export function RunsScreen({ state }: { state: string }) {
  if (state === 'empty')
    return (
      <AutomationShell tab="runs">
        <F id={['ASY-001', 'ASY-005']} className="concrete-grain m-6 flex flex-1 flex-col items-center justify-center gap-3 rounded-md border border-dashed border-border p-10 text-center">
          <p className="type-wide text-xl font-[700]">Noch keine Hintergrund-Läufe</p>
          <p className="max-w-lg text-[13px] text-muted-foreground">
            Lass Agents nach Zeitplan laufen (z. B. nachts die CI prüfen), per Timer später weitermachen oder über einen Webhook starten. Sie laufen auf diesem
            Rechner mit deinen Abos; Freigaben landen in der Inbox.
          </p>
          <div className="flex gap-2">
            <Btn variant="primary">
              <Plus className="size-3.5" /> Schedule anlegen
            </Btn>
            <Btn variant="outline">Webhook-Trigger anlegen</Btn>
          </div>
          <p className="text-[11px] text-muted-foreground">Läufe starten nur, solange der Rechner wach ist und beton läuft.</p>
        </F>
      </AutomationShell>
    )

  const sleep = state === 'after-sleep'
  const runs = sleep ? [...SLEEP, ...RUNS.filter((r) => r.status !== 'paused_approval' && r.id !== 'run_99z')] : RUNS

  return (
    <AutomationShell
      tab="runs"
      actions={
        <Btn variant="outline">
          <Plus className="size-3.5" /> Schedule anlegen
        </Btn>
      }
    >
      <div className="flex items-center gap-3 border-b border-border px-6 py-2">
        <Segmented items={['Alle', 'Aktiv', 'Wartet auf dich', 'Fehlgeschlagen']} active={state === 'approval' ? 'Wartet auf dich' : 'Alle'} />
        <F id="ASY-012" as="span" className="ml-auto text-[12px] text-muted-foreground" badge="top-right">
          Gleichzeitig: {sleep ? '2' : '3'} von 4 auf diesem Rechner · pr-fixer {sleep ? '1' : '2'} von 2{!sleep && ' · 1 wartet'}
        </F>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {sleep && (
          <F id={['ASY-005', 'ASY-004']} className="px-6 pt-4">
            <Note
              title={
                <span className="inline-flex items-center gap-2">
                  <Moon className="size-4" /> Dein Rechner war von Mi. 23:10 bis heute 07:31 im Ruhezustand
                </span>
              }
            >
              In der Zeit waren 4 Termine fällig. <Code>nightly-ci</Code> hat 3 verpasste Termine zu einem Lauf zusammengefasst (catch_up: run_once),{' '}
              <Code>weekly-report</Code> wurde übersprungen (catch_up: skip). beton weckt den Rechner nicht auf; mit „Wach halten während Läufen“ bleibt er bei
              laufenden Agents an.
            </Note>
          </F>
        )}
        {state === 'approval' && (
          <F id={['ASY-008', 'ASY-009']} className="px-6 pt-4">
            <div className="chamfer flex items-start gap-3 border-l-4 border-signal bg-signal-soft p-3 text-[13px]">
              <div className="flex-1">
                <p className="font-semibold">pr-fixer wartet seit 03:14 auf deine Freigabe</p>
                <p className="mt-0.5">
                  <Code>git push -u origin beton/pr-fixer-9a1/develop-ci</Code> · Ohne Antwort wird um 11:14 abgelehnt; der Lauf geht dann ohne Push weiter.
                </p>
              </div>
              <div className="flex shrink-0 gap-2">
                <Btn variant="primary" className="chamfer-sm rounded-none">
                  Erlauben
                </Btn>
                <Btn variant="outline">Ablehnen</Btn>
                <Btn variant="ghost">Lauf öffnen</Btn>
              </div>
            </div>
          </F>
        )}
        <F id={['ASY-001', 'ASY-011', 'ASY-010', 'ASY-009']} className="pt-2">
          <RunTable runs={state === 'approval' ? runs.filter((r) => r.status === 'paused_approval') : runs} />
        </F>
        <p className="px-6 py-3 text-[11px] text-muted-foreground">
          Läufe mit Abo kosten nichts extra; angezeigt wird der Anteil am Kontingent. Ältere Läufe: beton run-history.
        </p>
      </div>
    </AutomationShell>
  )
}
