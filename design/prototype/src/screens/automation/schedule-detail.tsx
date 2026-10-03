import type { ReactNode } from 'react'
import { ChevronLeft, Pause, Play } from 'lucide-react'
import { HarnessBadge } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Code, FakeInput, Mark, Note, Pill, SectionTitle, Segmented } from '@/screens/harnesses/kit'
import { AutomationShell, RunStatusMark, type RunStatus } from './parts'

function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[170px_minmax(0,1fr)] gap-4 border-b border-border py-3 text-[13px]">
      <div>
        <div className="font-medium">{label}</div>
        {hint && <div className="text-[11.5px] text-muted-foreground">{hint}</div>}
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  )
}

const HISTORY: { id: string; status: RunStatus; extra?: string; when: string; dur: string; use: string; manual?: boolean }[] = [
  { id: 'run_9a1', status: 'paused_approval', when: 'Sa. 03.10. 03:00', dur: '14 Min.', use: 'Claude Max · 3 %' },
  { id: 'run_98e', status: 'completed', extra: 'PR #417', when: 'Fr. 02.10. 14:22', dur: '21 Min.', use: 'Claude Max · 4 %', manual: true },
  { id: 'run_95d', status: 'timed_out', extra: 'nach 2 Std.', when: 'Fr. 02.10. 03:00', dur: '2 Std.', use: 'Claude Max · 11 %' },
  { id: 'run_93a', status: 'completed', extra: 'nichts zu tun', when: 'Do. 01.10. 03:00', dur: '3 Min.', use: 'Claude Max · 1 %' },
  { id: 'sk_88', status: 'skipped', extra: 'Vor-Lauf lief noch', when: 'Mi. 30.09. 03:00', dur: '–', use: '–' },
]

export function ScheduleDetail({ state }: { state: string }) {
  const edit = state === 'edit'
  const paused = state === 'paused'
  const team = state === 'team'
  return (
    <AutomationShell tab="schedules">
      <div className="flex shrink-0 items-start gap-4 border-b border-border px-6 py-3">
        <div className="min-w-0 flex-1">
          <button className="mb-0.5 inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground">
            <ChevronLeft className="size-3.5" /> Schedules
          </button>
          <div className="flex items-center gap-3">
            <h3 className="type-wide text-[17px] font-[700]">nightly-ci</h3>
            {paused ? <Mark kind="off" label="pausiert" /> : <Mark kind="ok" label="aktiv" />}
            <HarnessBadge id="claude" model="claude-sonnet-5-5" />
            <span className="text-[12px] text-muted-foreground">Agent pr-fixer · Projekt shop-frontend</span>
          </div>
        </div>
        <div className="flex gap-2 pt-4">
          {edit ? (
            <>
              <Btn variant="primary">Änderungen speichern</Btn>
              <Btn variant="ghost">Verwerfen</Btn>
            </>
          ) : (
            <>
              <Btn variant="outline">
                <Play className="size-3.5" /> Jetzt ausführen
              </Btn>
              <Btn variant="outline">{paused ? <><Play className="size-3.5" /> Fortsetzen</> : <><Pause className="size-3.5" /> Pausieren</>}</Btn>
            </>
          )}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="grid grid-cols-[minmax(0,1fr)_340px] gap-8 px-6 py-3">
          <div className="min-w-0">
            {!edit && (
              <F id="ASY-004" className="pt-2 pb-1">
                <p className="text-[12px] text-muted-foreground">
                  Deklariert in <Code>.beton/agents/pr-fixer/agent.yaml</Code> (Zeile 97). Änderungen dort werden beim nächsten Laden übernommen; hier kannst du
                  ihn pausieren oder einmalig starten.
                </p>
              </F>
            )}
            <F id="ASY-004">
              <Field label="Rhythmus" hint="Cron, 5 Felder">
                {edit ? (
                  <div className="space-y-1.5">
                    <FakeInput value="30 2 * * *" mono />
                    <p className="text-[12px]">Täglich um 02:30</p>
                  </div>
                ) : (
                  <>
                    <div>Täglich um 03:00</div>
                    <div className="font-mono text-[11.5px] text-muted-foreground">0 3 * * *</div>
                  </>
                )}
              </Field>
              <Field label="Zeitzone">Europe/Berlin (deine Zeitzone)</Field>
              <Field label="Aufgabe" hint="Vorlage mit Parametern">
                <div className="rounded-md border border-border bg-card px-2.5 py-1.5 font-mono text-[12px]">
                  Prüfe die CI von {'{{ params.branch }}'} und behebe Fehler.
                </div>
                <div className="mt-1.5 flex gap-1.5">
                  <Pill>branch = develop</Pill>
                  <Pill>max_attempts = 3</Pill>
                </div>
              </Field>
            </F>
            <F id={['ASY-004', 'ASY-005']}>
              <Field label="Verpasste Termine" hint="Rechner aus oder im Ruhezustand">
                <Segmented items={['Einmal nachholen', 'Überspringen']} active="Einmal nachholen" />
                <p className="mt-1 text-[12px] text-muted-foreground">Mehrere verpasste Termine ergeben genau einen Lauf. Bis 5 Min. Verspätung gilt als pünktlich.</p>
              </Field>
              <Field label="Wenn der letzte noch läuft">
                <Segmented items={['Überspringen', 'Danach starten']} active="Überspringen" />
              </Field>
              <Field label="Rechner wach halten">
                <Segmented items={['Nein', 'Während Läufen', 'Immer']} active="Während Läufen" />
              </Field>
            </F>
            <F id="ASY-010">
              <Field label="Grenzen pro Lauf" hint="Abo: Laufzeit und Turns zählen">
                <div className="flex flex-wrap gap-3">
                  <span>
                    Laufzeit <strong>1 Std.</strong>
                  </span>
                  <span>
                    Turns <strong>150</strong>
                  </span>
                  <span>
                    Tool-Calls <strong>400</strong>
                  </span>
                  <span className="text-muted-foreground">Kosten (nur bei API) 5 $</span>
                </div>
                <p className="mt-1 text-[12px] text-muted-foreground">Gilt für den ganzen Lauf inklusive Sub-Agents. Bei 80 % kommt ein Hinweis, bei 100 % endet der Lauf.</p>
              </Field>
            </F>
            <F id="ASY-006">
              <Field label="Runner" hint="nur mit Team-Server">
                {team ? (
                  <div className="space-y-2">
                    <FakeInput value="os=linux,repo=beton" mono />
                    <ul className="space-y-1 text-[12.5px]">
                      <li>
                        <Mark kind="ok" label={<>build-linux-01 · online · <span className="font-mono text-[11px] text-muted-foreground">os=linux repo=beton beton.harness.claude</span></>} />
                      </li>
                      <li>
                        <Mark kind="off" label={<>build-linux-02 · offline seit 06:12</>} />
                      </li>
                      <li className="text-muted-foreground">
                        <Mark kind="skip" label="mac-ingo passt nicht (os=macos)" />
                      </li>
                    </ul>
                    <p className="text-[12px] text-muted-foreground">Ist kein passender Runner online, wartet der Lauf bis zu 10 Min. und schlägt dann mit Inbox-Hinweis fehl.</p>
                  </div>
                ) : (
                  <span className="text-muted-foreground">Dieser Mac. Mit einem Team-Server kannst du Läufe per Label an andere Rechner geben.</span>
                )}
              </Field>
            </F>
          </div>

          <div className="space-y-6 pt-2">
            <F id={['ASY-004', 'ASY-005']} as="section">
              <SectionTitle>{edit ? 'Vorschau' : 'Nächste Läufe'}</SectionTitle>
              <ul className="space-y-1 text-[13px] tabular-nums">
                {(edit
                  ? ['So. 04.10. 02:30', 'Mo. 05.10. 02:30', 'Di. 06.10. 02:30', '…', 'Sa. 24.10. 02:30', 'So. 25.10. 02:30 (nur einmal)']
                  : paused
                    ? []
                    : ['So. 04.10. 03:00', 'Mo. 05.10. 03:00', 'Di. 06.10. 03:00', 'Mi. 07.10. 03:00', 'Do. 08.10. 03:00']
                ).map((d) => (
                  <li key={d}>{d}</li>
                ))}
                {paused && <li className="text-muted-foreground">Pausiert seit gestern 17:40 – keine Läufe geplant.</li>}
              </ul>
              {edit && (
                <Note className="mt-3">
                  Am 25.10. wird die Uhr zurückgestellt: 02:30 gibt es zweimal, der Lauf startet nur beim ersten Mal. Im März fehlt 02:30; dann startet er um
                  03:00.
                </Note>
              )}
            </F>
            <F id={['ASY-011', 'ASY-004']} as="section">
              <SectionTitle aside="beton run-history nightly-ci">Letzte Läufe</SectionTitle>
              <ul className="divide-y divide-border border-y border-border text-[12.5px]">
                {HISTORY.map((h) => (
                  <li key={h.id} className={cn('py-2', h.status === 'paused_approval' && 'bg-signal-soft/60')}>
                    <div className="flex items-center gap-2">
                      <RunStatusMark status={h.status} extra={h.extra} />
                      {h.manual && <Pill>manuell</Pill>}
                    </div>
                    <div className="flex gap-3 pl-5 text-[11.5px] text-muted-foreground tabular-nums">
                      <span>{h.when}</span>
                      <span>{h.dur}</span>
                      <span>{h.use}</span>
                    </div>
                  </li>
                ))}
              </ul>
            </F>
          </div>
        </div>
      </div>
    </AutomationShell>
  )
}
