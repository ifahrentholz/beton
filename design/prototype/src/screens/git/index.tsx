import type { ScreenGroup } from '@/proto/types'
import { ConnectScreen } from './connect'
import { CreatePrScreen, PrPanelScreen, ReviewScreen } from './panel'

/**
 * GitHub & GitLab: optionale Verbindung zu Providern, PR-/MR-Panel mit Status, Checks,
 * Reviews und Diff, PR aus der Session erstellen. Alles Netzwerk ist als optional gekennzeichnet.
 */
export const group: ScreenGroup = {
  id: 'git',
  title: 'GitHub & GitLab',
  order: 40,
  screens: [
    {
      id: 'git-connect',
      title: 'Provider verbinden',
      description:
        'Optionale Verbindung zu GitHub (auch Enterprise) und GitLab (auch selbst betrieben). Tokens liegen im Schlüsselbund; Agents sehen nur Platzhalter, jede Nutzung wird protokolliert.',
      features: ['GIT-001', 'GIT-002', 'GIT-003', 'GIT-004'],
      states: [
        { id: 'connected', title: 'Verbunden' },
        { id: 'none', title: 'Nichts verbunden' },
        { id: 'add-host', title: 'Host hinzufügen' },
        { id: 'tls-error', title: 'Zertifikatsfehler' },
      ],
      component: ConnectScreen,
    },
    {
      id: 'git-pr-panel',
      title: 'PR-Panel',
      description:
        'Pull- und Merge-Requests der Session mit Herkunft, Status, Mergebarkeit, Reviews und Checks. Ein fehlgeschlagener Check geht mit einem Klick samt Log an den Agent.',
      features: ['GIT-001', 'GIT-002', 'GIT-003', 'GIT-004', 'GIT-005', 'GIT-006'],
      states: [
        { id: 'github', title: 'GitHub-PR' },
        { id: 'sent', title: 'Check an Agent' },
        { id: 'gitlab', title: 'GitLab-MR' },
        { id: 'attach', title: 'Per Adresse verknüpfen' },
        { id: 'rate-limit', title: 'Abfragelimit' },
        { id: 'no-provider', title: 'Kein Provider' },
        { id: 'connect-account', title: 'Eigenes Konto fehlt' },
      ],
      component: PrPanelScreen,
    },
    {
      id: 'git-review',
      title: 'Reviews & Diff',
      description:
        'Der Diff, wie ihn der Provider berechnet, pro Datei einklappbar, mit Review-Threads an den Zeilen. Antworten gehen unter deinem Namen zum Provider; Threads lassen sich an den Agent geben.',
      features: ['GIT-007', 'GIT-008'],
      states: [
        { id: 'diff', title: 'Diff' },
        { id: 'threads', title: 'Review-Threads' },
        { id: 'unpushed', title: 'Lokal weiter' },
      ],
      component: ReviewScreen,
    },
    {
      id: 'git-create-pr',
      title: 'PR aus der Session',
      description:
        'Aus dem Worktree-Branch einen Pull Request anlegen: Titel und Beschreibung sind vorgeschlagen, der Push läuft durch die Policies und fragt bei Bedarf nach.',
      features: ['GIT-005', 'GIT-009'],
      states: [
        { id: 'form', title: 'Formular' },
        { id: 'approval', title: 'Push-Freigabe' },
        { id: 'done', title: 'Erstellt' },
        { id: 'disabled', title: 'Keine Commits' },
        { id: 'denied', title: 'Von Policy abgelehnt' },
      ],
      component: CreatePrScreen,
    },
  ],
}
