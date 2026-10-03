import { Coffee, Plus } from 'lucide-react'
import { VoiceDot } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { harnesses, type HarnessId } from '@/mock/data'
import { Btn, Code, Mark, Note, Pill, SectionTitle } from '@/screens/harnesses/kit'
import { AutomationShell } from './parts'

type Sched = {
  id: string
  agent: string
  harness: HarnessId
  cron: string
  human: string
  next: string[]
  catchUp: string
  awake: string
  source: string
  paused?: boolean
  missed?: string
}

const SCHEDULES: Sched[] = [
  { id: 'nightly-ci', agent: 'pr-fixer', harness: 'claude', cron: '0 3 * * *', human: 'Täglich um 03:00', next: ['So. 04.10. 03:00', 'Mo. 05.10. 03:00'], catchUp: 'einmal nachholen', awake: 'während Läufen', source: 'agent.yaml' },
  { id: 'weekly-deps', agent: 'dep-updater', harness: 'codex', cron: '30 7 * * 6', human: 'Samstags um 07:30', next: ['Sa. 10.10. 07:30'], catchUp: 'einmal nachholen', awake: 'während Läufen', source: 'beton schedule create' },
  { id: 'weekly-report', agent: 'pr-fixer', harness: 'claude', cron: '0 18 * * 5', human: 'Freitags um 18:00', next: ['Fr. 09.10. 18:00'], catchUp: 'überspringen', awake: 'nein', source: 'App' },
  { id: 'a11y-sweep', agent: 'a11y-auditor', harness: 'gemini', cron: '30 2 * * *', human: 'Täglich um 02:30', next: ['–'], catchUp: 'überspringen', awake: 'nein', source: 'App', paused: true },
]

const TIMERS = [
  { session: 'Checkout-Formular barrierefrei machen', when: 'heute 11:40 (in 2 Std.)', prompt: 'Prüfe, ob der Fix für die Fokus-Reihenfolge in Staging angekommen ist.', by: 'vom Agent gesetzt (timer_set)', state: 'wartet' },
  { session: 'Nächtliches Dependency-Update', when: 'So. 04.10. 09:00', prompt: 'Schau, ob Renovate den Lockfile-Konflikt aufgelöst hat.', by: 'von dir per CLI', state: 'wartet' },
  { session: 'Event-Log: seq lückenlos halten', when: 'heute 07:31 statt 06:00', prompt: 'CI-Lauf von Nacht auswerten.', by: 'vom Agent gesetzt', state: 'verspätet ausgelöst (+91 Min., Rechner schlief)' },
]

export function SchedulesScreen({ state }: { state: string }) {
  const daemonOff = state === 'daemon-off'
  if (state === 'empty')
    return (
      <AutomationShell tab="schedules">
        <F id={['ASY-004', 'ASY-003']} className="concrete-grain m-6 flex flex-1 flex-col items-center justify-center gap-3 rounded-md border border-dashed border-border p-10 text-center">
          <p className="type-wide text-xl font-[700]">Keine Schedules und Timer</p>
          <p className="max-w-lg text-[13px] text-muted-foreground">
            Ein Schedule startet einen Agent regelmäßig, z. B. werktags um 03:00. Timer setzen Agents selbst („in 2 Std. nachsehen“) oder du per{' '}
            <Code>beton timer set</Code>. Schedules kannst du auch direkt in der <Code>agent.yaml</Code> deklarieren.
          </p>
          <Btn variant="primary">
            <Plus className="size-3.5" /> Schedule anlegen
          </Btn>
        </F>
      </AutomationShell>
    )

  return (
    <AutomationShell
      tab="schedules"
      daemonOff={daemonOff}
      actions={
        <Btn variant="primary">
          <Plus className="size-3.5" /> Schedule anlegen
        </Btn>
      }
    >
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="grid grid-cols-[minmax(0,1fr)_300px] gap-8 px-6 py-4">
          <div className="min-w-0 space-y-6">
            {daemonOff && (
              <F id="ASY-005">
                <Note
                  tone="deny"
                  title="Schedules und Timer pausieren gerade"
                  action={
                    <>
                      <Btn variant="primary" size="sm">
                        beton jetzt starten
                      </Btn>
                      <Btn variant="outline" size="sm">
                        Mit der Anmeldung starten
                      </Btn>
                    </>
                  }
                >
                  Sie laufen nur, solange der beton-Dienst auf diesem Rechner läuft. Seit 22:40 ist er beendet; der Termin von nightly-ci um 03:00 wurde
                  deshalb verpasst und wird beim Start einmal nachgeholt.
                </Note>
              </F>
            )}
            <F id={['ASY-004', 'ASY-011']} as="section">
              <SectionTitle aside="Zeitzone Europe/Berlin">Schedules</SectionTitle>
              <table className="type-narrow w-full text-[12.5px]">
                <thead>
                  <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
                    <th className="py-1.5 font-normal">Name · Agent</th>
                    <th className="font-normal">Rhythmus</th>
                    <th className="font-normal">Nächste Läufe</th>
                    <th className="font-normal">Verpasst</th>
                    <th className="font-normal">Zustand</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {SCHEDULES.map((s) => (
                    <tr key={s.id} className={cn('align-top', s.paused && 'text-muted-foreground')}>
                      <td className="py-2.5 pr-3">
                        <div className="font-medium">{s.id}</div>
                        <div className="flex items-center gap-1.5 text-muted-foreground">
                          <VoiceDot voice={harnesses[s.harness].voice} className="size-1.5" />
                          {s.agent} · aus {s.source}
                        </div>
                      </td>
                      <td className="py-2.5 pr-3">
                        <div>{s.human}</div>
                        <div className="font-mono text-[11px] text-muted-foreground">{s.cron}</div>
                      </td>
                      <td className="py-2.5 pr-3 tabular-nums">
                        {s.next.map((n) => (
                          <div key={n}>{n}</div>
                        ))}
                      </td>
                      <td className="py-2.5 pr-3">{s.catchUp}</td>
                      <td className="py-2.5">
                        {s.paused ? <Mark kind="off" label="pausiert" /> : daemonOff ? <Mark kind="skip" label="ruht, Dienst aus" /> : <Mark kind="ok" label="aktiv" />}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </F>

            <F id="ASY-003" as="section">
              <SectionTitle aside="einmalig · überleben Neustarts">Timer</SectionTitle>
              <ul className="divide-y divide-border border-y border-border text-[13px]">
                {TIMERS.map((t) => (
                  <li key={t.session} className="grid grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_auto] gap-4 py-2.5">
                    <div className="min-w-0">
                      <div className="truncate font-medium">{t.session}</div>
                      <div className="truncate text-[12px] text-muted-foreground">„{t.prompt}“</div>
                    </div>
                    <div>
                      <div className="tabular-nums">{t.when}</div>
                      <div className="text-[12px] text-muted-foreground">{t.by}</div>
                    </div>
                    <div className="flex items-start gap-2">
                      {t.state.startsWith('verspätet') ? <Mark kind="ok" label={t.state} /> : <Mark kind="queued" label={t.state} />}
                      {!t.state.startsWith('verspätet') && (
                        <Btn variant="ghost" size="sm">
                          Löschen
                        </Btn>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            </F>
          </div>

          <F id="ASY-005" as="section" className="space-y-4 text-[13px]">
            <div>
              <SectionTitle>Dieser Rechner</SectionTitle>
              <dl className="space-y-2">
                <div className="flex items-center gap-2">
                  {daemonOff ? <Mark kind="deny" label="beton-Dienst beendet" /> : <Mark kind="ok" label="beton-Dienst läuft · startet mit der Anmeldung" />}
                </div>
                <div className="flex items-start gap-2">
                  <Coffee className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
                  <span>
                    Wach halten: <strong>während Läufen</strong>
                    <span className="block text-[12px] text-muted-foreground">Verhindert den Ruhezustand nur, solange ein Agent arbeitet.</span>
                  </span>
                </div>
              </dl>
            </div>
            <div className="border-t border-border pt-4 text-[12px] text-muted-foreground">
              beton weckt den Rechner nicht auf. Verpasste Termine werden nach dem Aufwachen je Schedule einmal nachgeholt oder übersprungen.
            </div>
            <div className="border-t border-border pt-4">
              <Pill>Alles läuft lokal, ohne Server</Pill>
            </div>
          </F>
        </div>
      </div>
    </AutomationShell>
  )
}
