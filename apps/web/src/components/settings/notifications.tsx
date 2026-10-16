import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { NotificationChannels, NotificationEvent, NotificationSettings } from '@beton/sdk'
import { client } from '@/lib/client'
import { cn } from '@/lib/utils'
import { Checkbox, NetNote, PageHeader, Row, SectionTitle, SettingsShell, Switch } from './kit'

/** Zeilen der Matrix wie im Entwurf `settings-notifications`. */
export const NOTIFY_TYPES: { id: NotificationEvent; label: string; hint?: string; later?: string }[] = [
  { id: 'approval_requested', label: 'Freigabe angefordert' },
  { id: 'question', label: 'Frage des Agents' },
  { id: 'turn_completed', label: 'Agent ist fertig', hint: 'Nur wenn das Fenster nicht im Vordergrund ist' },
  { id: 'async_done', label: 'Hintergrund-Agent fertig oder fehlgeschlagen', later: 'ab M5' },
  { id: 'mention', label: 'Erwähnung in geteilter Session', later: 'ab M4' },
  { id: 'runner_lost', label: 'Runner oder Host getrennt' },
  { id: 'budget_threshold', label: 'Budget-Schwelle erreicht', hint: 'Nur bei API-Key oder Gateway' },
  { id: 'rate_limit_high', label: 'Subscription-Kontingent fast aufgebraucht', hint: 'Ab 90 % eines Fensters, z. B. des 5-Stunden-Fensters' },
]

const CHANNELS: { key: keyof NotificationChannels; label: string }[] = [
  { key: 'desktop', label: 'Desktop-Mitteilung' },
  { key: 'sound', label: 'Ton' },
  { key: 'badge', label: 'Dock-Badge' },
]

const QUERY_KEY = ['settings', 'notifications']

function timeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone
  } catch {
    return ''
  }
}

/**
 * Benachrichtigungs-Einstellungen (UX-006): Matrix Ereignis × Kanal und Ruhezeiten. Gespeichert
 * wird je User auf dem Server, sofort bei jeder Änderung; sie gelten damit auf allen Geräten.
 * Web-Push (WEB-014) kommt mit M4 und ist hier nur als ausgegraute Spalte sichtbar.
 */
export function NotificationSettingsView() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: QUERY_KEY, queryFn: () => client.settings.notifications.get() })
  const save = useMutation({
    mutationFn: (next: NotificationSettings) => client.settings.notifications.put(next),
    onMutate: (next) => {
      const prev = qc.getQueryData<NotificationSettings>(QUERY_KEY)
      qc.setQueryData(QUERY_KEY, next)
      return { prev }
    },
    onError: (_e, _next, ctx) => {
      if (ctx?.prev) qc.setQueryData(QUERY_KEY, ctx.prev)
    },
    onSuccess: (saved) => qc.setQueryData(QUERY_KEY, saved),
  })
  const s = q.data
  const update = (fn: (draft: NotificationSettings) => void) => {
    if (!s) return
    const next = structuredClone(s)
    fn(next)
    save.mutate(next)
  }
  const tz = timeZone()

  return (
    <SettingsShell active="/settings/notifications">
      <PageHeader title="Benachrichtigungen">
        Standard sind lokale Mitteilungen über das Betriebssystem – nichts verlässt dafür den Rechner. Gilt für alle deine Geräte.
      </PageHeader>
      {q.isError && (
        <p className="rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]" role="alert">
          Einstellungen nicht geladen: {q.error instanceof Error ? q.error.message : 'unbekannter Fehler'}
        </p>
      )}
      {save.isError && (
        <p className="mb-3 rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]" role="alert">
          Nicht gespeichert: {save.error instanceof Error ? save.error.message : 'unbekannter Fehler'}
        </p>
      )}
      {s && (
        <>
          <SectionTitle aside={<NetNote kind="local">Inbox, Desktop, Ton und Badge bleiben lokal</NetNote>}>Was dich wie erreicht</SectionTitle>
          <div className="overflow-x-auto rounded-md border border-border">
            <table className="w-full min-w-[560px] text-[13px]">
              <thead className="bg-sunken text-[12px] text-muted-foreground">
                <tr>
                  <th className="py-2 pl-3 text-left font-medium">Ereignis</th>
                  <th className="w-[86px] px-1 py-2 text-center font-medium">Inbox</th>
                  {CHANNELS.map((c) => (
                    <th key={c.key} className="w-[86px] px-1 py-2 text-center font-medium">
                      {c.label}
                    </th>
                  ))}
                  <th className="w-[86px] border-l border-dashed border-border px-1 py-2 text-center font-medium">
                    Web-Push
                    <div className="text-[10px] font-normal">optional</div>
                  </th>
                </tr>
              </thead>
              <tbody>
                {NOTIFY_TYPES.map((n) => (
                  <tr key={n.id} className="border-t border-border">
                    <td className="py-2 pl-3">
                      <div className="flex items-baseline gap-2">
                        <span>{n.label}</span>
                        {n.later && <span className="text-[11px] text-muted-foreground">{n.later}</span>}
                      </div>
                      {n.hint && <div className="text-[11px] text-muted-foreground">{n.hint}</div>}
                    </td>
                    <td className="text-center">
                      <Checkbox checked disabled label={`${n.label}: Inbox (immer an)`} title="Die Inbox zeigt immer alles" />
                    </td>
                    {CHANNELS.map((c) => (
                      <td key={c.key} className="text-center">
                        <Checkbox
                          checked={s.events[n.id][c.key]}
                          label={`${n.label}: ${c.label}`}
                          onChange={(v) =>
                            update((d) => {
                              d.events[n.id][c.key] = v
                            })
                          }
                        />
                      </td>
                    ))}
                    <td className={cn('border-l border-dashed border-border text-center')}>
                      <Checkbox checked={false} disabled label={`${n.label}: Web-Push`} title="Web-Push kommt mit M4" />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <SectionTitle>Ruhezeiten</SectionTitle>
          <Row label="Ruhezeiten" hint="In dieser Zeit bleiben Desktop-Mitteilung und Ton stumm; die Inbox sammelt weiter.">
            <Switch
              checked={s.quiet_hours.enabled}
              label="Ruhezeiten"
              onChange={(v) =>
                update((d) => {
                  d.quiet_hours.enabled = v
                })
              }
            />
          </Row>
          {s.quiet_hours.enabled && (
            <>
              <Row label="Zeitraum" hint={tz ? `Jeden Tag, in deiner Zeitzone (${tz})` : 'Jeden Tag, in deiner Zeitzone'}>
                <input
                  type="time"
                  aria-label="Beginn der Ruhezeiten"
                  value={s.quiet_hours.start}
                  onChange={(e) =>
                    e.target.value &&
                    update((d) => {
                      d.quiet_hours.start = e.target.value
                    })
                  }
                  className="rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]"
                />
                <span className="text-muted-foreground">bis</span>
                <input
                  type="time"
                  aria-label="Ende der Ruhezeiten"
                  value={s.quiet_hours.end}
                  onChange={(e) =>
                    e.target.value &&
                    update((d) => {
                      d.quiet_hours.end = e.target.value
                    })
                  }
                  className="rounded-md border border-input bg-card px-2 py-1 font-mono text-[12px]"
                />
              </Row>
              <Row label="Freigaben trotzdem melden" hint="Ein wartender Agent soll dich auch nachts erreichen.">
                <Switch
                  checked={s.quiet_hours.allow_approvals}
                  label="Freigaben trotzdem melden"
                  onChange={(v) =>
                    update((d) => {
                      d.quiet_hours.allow_approvals = v
                    })
                  }
                />
              </Row>
            </>
          )}
        </>
      )}
    </SettingsShell>
  )
}
