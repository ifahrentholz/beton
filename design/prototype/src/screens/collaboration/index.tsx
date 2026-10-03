import type { ScreenGroup } from '@/proto/types'
import { CoDriveScreen } from './codrive'
import { CommentsScreen } from './comments'
import { ShareScreen, SharedWithMeScreen } from './share'
import { InboxScreen, SideChatScreen } from './side-inbox'

/**
 * Zusammenarbeit: Sessions teilen, gemeinsam steuern, kommentieren, Side-Chats und Benachrichtigungen.
 * Teilen mit anderen braucht einen (optionalen) Team-Server; lokal teilst du nur mit eigenen Geräten.
 */
export const group: ScreenGroup = {
  id: 'collaboration',
  title: 'Zusammenarbeit',
  order: 30,
  screens: [
    {
      id: 'collab-share',
      title: 'Session teilen',
      description:
        'Freigeben an Personen oder Teams mit drei Rollen: Ansehen, Kommentieren & freigeben, Mitsteuern. Mitsteuern heißt Code-Ausführung auf deinem Rechner und muss bestätigt werden.',
      features: ['COL-001', 'COL-002'],
      states: [
        { id: 'dialog', title: 'Teilen-Dialog' },
        { id: 'drive-warning', title: 'Mitsteuern bestätigen' },
        { id: 'view-only', title: 'Server erlaubt nur Ansehen' },
        { id: 'local-only', title: 'Ohne Team-Server' },
      ],
      component: ShareScreen,
    },
    {
      id: 'collab-shared-with-me',
      title: 'Mit mir geteilt',
      description: 'Sessions anderer mit deiner Rolle. Wer nur ansehen darf, liest live mit und kann in eine eigene Session forken. Beendete Freigaben trennen sofort.',
      features: ['COL-001', 'COL-002'],
      states: [
        { id: 'list', title: 'Liste' },
        { id: 'viewer', title: 'Als Zuschauer' },
        { id: 'revoked', title: 'Freigabe beendet' },
      ],
      component: SharedWithMeScreen,
    },
    {
      id: 'collab-codrive',
      title: 'Gemeinsam steuern',
      description:
        'Mehrere Personen steuern dieselbe Session. Nachrichten tragen Avatar und Namen, der Agent sieht im Kontext, wer schreibt. Oben siehst du, wer gerade da ist und wo.',
      features: ['COL-003', 'COL-004', 'SES-004'],
      states: [
        { id: 'live', title: 'Zu zweit' },
        { id: 'presence', title: 'Wer ist da' },
        { id: 'typing', title: 'Anna tippt' },
        { id: 'approval-decided', title: 'Freigabe von Anna' },
      ],
      component: CoDriveScreen,
    },
    {
      id: 'collab-comments',
      title: 'Kommentare',
      description:
        'Kommentare an Nachrichten, Dateien und Diff-Zeilen, als Threads mit Antworten. Mehrere Kommentare gehen gebündelt als eine Nachricht an den Agent; danach sind sie als adressiert markiert.',
      features: ['COL-005', 'COL-006', 'COL-007'],
      states: [
        { id: 'message', title: 'An einer Nachricht' },
        { id: 'diff', title: 'An Diff-Zeilen' },
        { id: 'address', title: 'An Agent geben' },
        { id: 'addressed', title: 'Adressiert' },
        { id: 'suggest', title: 'Nur vorschlagen' },
      ],
      component: CommentsScreen,
    },
    {
      id: 'collab-side-chat',
      title: 'Side-Chats',
      description: 'Nebenfragen in einem versteckten Fork, ohne die laufende Session zu stören. Side-Chats lesen nur; das Ergebnis lässt sich in die Hauptsession übernehmen.',
      features: ['COL-008'],
      states: [
        { id: 'chat', title: 'Nebenfrage' },
        { id: 'adopted', title: 'Übernommen' },
      ],
      component: SideChatScreen,
    },
    {
      id: 'collab-inbox',
      title: 'Inbox & Benachrichtigungen',
      description:
        'Freigaben, Erwähnungen, Antworten und neue Freigaben landen in der Inbox. Desktop-Meldungen kommen nur, wenn du nicht gerade hinsiehst, und werden zusammengefasst.',
      features: ['COL-009', 'COL-010'],
      states: [
        { id: 'inbox', title: 'Inbox' },
        { id: 'notification', title: 'Desktop-Meldung' },
        { id: 'settings', title: 'Einstellungen' },
      ],
      component: InboxScreen,
    },
  ],
}
