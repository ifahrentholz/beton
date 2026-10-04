import type { ScreenGroup } from '@/proto/types'
import { RunDetail } from './run-detail'
import { RunsScreen } from './runs'
import { ScheduleDetail } from './schedule-detail'
import { SchedulesScreen } from './schedules'
import { AutomationCli } from './terminal'
import { WebhookScreen } from './webhook'

/**
 * Automationen (Design-Paket D-3): Async-Läufe, Schedules, Timer und Webhook-Trigger.
 * Lokal-first: Alles läuft auf diesem Rechner, solange beton läuft; kein Aufwecken, dafür catch_up.
 * Mit Abo zählen Laufzeit, Turns und Kontingent statt Geldbetrag.
 */
export const group: ScreenGroup = {
  id: 'automation',
  title: 'Automationen',
  order: 70,
  screens: [
    {
      id: 'automation-runs',
      title: 'Läufe',
      description:
        'Alle Hintergrund-Läufe mit Status, Auslöser, Dauer und Verbrauch. Läufe, die auf eine Freigabe warten, stehen oben; Warteschlange und verpasste Termine sind erklärt.',
      features: ['ASY-001', 'ASY-004', 'ASY-005', 'ASY-008', 'ASY-009', 'ASY-010', 'ASY-011', 'ASY-012'],
      states: [
        { id: 'default', title: 'Gemischt' },
        { id: 'approval', title: 'Wartet auf dich' },
        { id: 'after-sleep', title: 'Nach dem Ruhezustand' },
        { id: 'empty', title: 'Leer' },
      ],
      component: RunsScreen,
    },
    {
      id: 'automation-schedules',
      title: 'Schedules & Timer',
      description: 'Wiederkehrende Läufe mit Cron und Zeitzone, einmalige Timer und ob der Rechner wach gehalten wird.',
      features: ['ASY-003', 'ASY-004', 'ASY-005', 'ASY-011'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'daemon-off', title: 'beton-Dienst aus' },
        { id: 'empty', title: 'Leer' },
      ],
      component: SchedulesScreen,
    },
    {
      id: 'schedule-detail',
      title: 'Schedule',
      description:
        'Ein Schedule mit Rhythmus, Zeitzone, Vorlage, Umgang mit verpassten Terminen, Wachhalten, Grenzen pro Lauf und Runner-Auswahl. Vorschau berücksichtigt die Zeitumstellung.',
      features: ['ASY-004', 'ASY-005', 'ASY-006', 'ASY-010', 'ASY-011'],
      states: [
        { id: 'default', title: 'Aktiv' },
        { id: 'edit', title: 'Bearbeiten mit Vorschau' },
        { id: 'paused', title: 'Pausiert' },
        { id: 'team', title: 'Runner per Label (Team)' },
      ],
      component: ScheduleDetail,
    },
    {
      id: 'run-detail',
      title: 'Lauf-Detail',
      description:
        'Ein einzelner Lauf: Statusverlauf, was gerade passiert, Grenzen und Auslöser. Freigaben ohne Zuschauer pausieren den Lauf und werden nach Ablauf abgelehnt.',
      features: ['ASY-001', 'ASY-006', 'ASY-008', 'ASY-009', 'ASY-010'],
      states: [
        { id: 'paused', title: 'Wartet auf Freigabe' },
        { id: 'timeout-denied', title: 'Zeitablauf: abgelehnt' },
        { id: 'running', title: 'Läuft ohne Zuschauer' },
        { id: 'completed', title: 'Fertig' },
        { id: 'budget-exceeded', title: 'Budget erreicht' },
        { id: 'no-runner', title: 'Kein Runner (Team)' },
      ],
      component: RunDetail,
    },
    {
      id: 'webhook-trigger',
      title: 'Webhook-Trigger',
      description: 'Ein Endpunkt, über den z. B. die CI einen Agent-Lauf startet: Token, Vorlage mit Payload-Werten, Grenzen und die letzten Aufrufe.',
      features: ['ASY-007', 'AGT-010', 'ASY-010'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'token-created', title: 'Neues Token' },
        { id: 'calls', title: 'Aufrufe' },
      ],
      component: WebhookScreen,
    },
    {
      id: 'automation-cli',
      title: 'Automationen im Terminal',
      description: 'beton schedule list, run-now, run-history und die Warteschlange.',
      features: ['ASY-011', 'ASY-012', 'ASY-005', 'ASY-004'],
      frame: 'terminal',
      states: [
        { id: 'list', title: 'schedule list' },
        { id: 'history', title: 'run-now & run-history' },
        { id: 'queue', title: 'list --queue' },
      ],
      component: AutomationCli,
    },
  ],
}
