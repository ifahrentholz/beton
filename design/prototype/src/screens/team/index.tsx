import type { ScreenGroup } from '@/proto/types'
import { LocalAccess, OrgBootstrap, TeamLogin } from './access'
import { GitConnections, Members, SessionSharing, Tokens } from './admin'
import { DevicePairing, Devices, QrPairing, QrPairingPhone } from './devices'

/**
 * D-5 · Team & Anmeldung: lokaler Modus ohne Login, optionaler Team-Server mit OIDC,
 * Bootstrap, Device-Pairing (Code, QR), Geräte, Tokens, Rollen, Freigaben, Offboarding, Git-Verbindungen.
 */
export const group: ScreenGroup = {
  id: 'team',
  title: 'Team & Anmeldung',
  order: 140,
  screens: [
    {
      id: 'local-access',
      title: 'Lokaler Modus: kein Login',
      description:
        'Lokal gibt es kein Konto: Zugriff nur über Loopback und Token-Datei, der Browser meldet sich per Einmal-Link an. Unsichere Token-Rechte verhindern den Start.',
      features: ['AUTH-001', 'AUTH-002', 'AUTH-003', 'AUTH-004', 'AUTH-009'],
      states: [
        { id: 'open', title: 'beton open → Browser' },
        { id: 'expired', title: 'Link abgelaufen' },
        { id: 'settings', title: 'Einstellungen' },
        { id: 'insecure-token', title: 'Token-Datei unsicher' },
      ],
      component: LocalAccess,
    },
    {
      id: 'team-login',
      title: 'Anmeldung am Team-Server',
      description: 'Nur im optionalen Team-Betrieb: Anmeldung über OIDC-Anbieter mit Konto-Anlage beim ersten Login; Passkey als Abkürzung, der Anbieter bleibt maßgeblich.',
      features: ['AUTH-005', 'AUTH-007', 'AUTH-017'],
      states: [
        { id: 'providers', title: 'Anbieter wählen' },
        { id: 'denied', title: 'Domain nicht zugelassen' },
        { id: 'disabled', title: 'Konto deaktiviert' },
        { id: 'passkey', title: 'Passkey' },
        { id: 'passkey-reauth', title: 'Passkey: 30 Tage um' },
      ],
      component: TeamLogin,
    },
    {
      id: 'org-bootstrap',
      title: 'Erster Owner',
      description: 'Ein frischer Team-Server: Mit dem Einrichtungscode aus den Server-Logs wird der erste angemeldete Mensch Owner; danach ist der Weg geschlossen.',
      features: ['AUTH-006'],
      states: [
        { id: 'setup-code', title: 'Code eingeben' },
        { id: 'done', title: 'Owner geworden' },
      ],
      component: OrgBootstrap,
    },
    {
      id: 'device-pairing',
      title: 'Gerät koppeln per Code',
      description: 'Ein Host ohne Bildschirm zeigt einen Code; du bestätigst im Browser mit allen Angaben zum Gerät. Mit Ratenbegrenzung und Ablauf.',
      features: ['AUTH-008', 'AUTH-010'],
      states: [
        { id: 'confirm', title: 'Bestätigen' },
        { id: 'paired', title: 'Gekoppelt' },
        { id: 'rate-limited', title: 'Zu viele Fehlversuche' },
        { id: 'expired', title: 'Abgelaufen' },
      ],
      component: DevicePairing,
    },
    {
      id: 'qr-pairing',
      title: 'Handy koppeln per QR',
      description: 'QR-Code am Rechner, Bestätigung am Rechner. Für einen rein lokalen beton geht das nur mit TLS, z. B. über Tailscale.',
      features: ['AUTH-009', 'AUTH-001'],
      states: [
        { id: 'qr', title: 'QR-Code' },
        { id: 'confirm', title: 'Am Rechner bestätigen' },
        { id: 'local-tailscale', title: 'Lokal über Tailscale' },
      ],
      component: QrPairing,
    },
    {
      id: 'qr-pairing-phone',
      title: 'Handy koppeln (Handy-Seite)',
      description: 'Was das Handy nach dem Scannen zeigt: Gerätename, Warten auf die Bestätigung am Rechner, fertig.',
      features: ['AUTH-009'],
      frame: 'mobile',
      states: [
        { id: 'name', title: 'Name' },
        { id: 'waiting', title: 'Wartet auf Rechner' },
        { id: 'done', title: 'Gekoppelt' },
      ],
      component: QrPairingPhone,
    },
    {
      id: 'device-list',
      title: 'Geräte & Hosts',
      description: 'Gekoppelte Geräte und Hosts mit Art, Plattform, zuletzt gesehen, IP und Rechten; Hosts mit Annahme-Regel und Runner-Tokens. Widerruf trennt sofort.',
      features: ['AUTH-010', 'AUTH-011'],
      states: [
        { id: 'list', title: 'Liste' },
        { id: 'revoke', title: 'Widerrufen' },
        { id: 'revoked', title: 'Widerrufen erledigt' },
      ],
      component: Devices,
    },
    {
      id: 'access-tokens',
      title: 'Zugangstokens und Service-Accounts',
      description: 'Persönliche Tokens mit Rechten und Pflicht-Ablauf, nur einmal sichtbar; Service-Accounts als eigene Konten der Org.',
      features: ['AUTH-012', 'AUTH-013', 'AUTH-014'],
      states: [
        { id: 'pats', title: 'Tokens' },
        { id: 'create', title: 'Erstellen' },
        { id: 'created', title: 'Erstellt' },
        { id: 'service-accounts', title: 'Service-Accounts' },
      ],
      component: Tokens,
    },
    {
      id: 'org-members',
      title: 'Mitglieder & Rollen',
      description: 'Org- und Team-Rollen mit Rollenmatrix; Offboarding widerruft alles in einem Schritt; der letzte Owner lässt sich nicht entfernen.',
      features: ['AUTH-006', 'AUTH-014', 'AUTH-016'],
      states: [
        { id: 'org-members', title: 'Mitglieder' },
        { id: 'offboard', title: 'Offboarding' },
        { id: 'last-owner', title: 'Letzter Owner' },
      ],
      component: Members,
    },
    {
      id: 'session-sharing',
      title: 'Session-Freigaben durchgesetzt',
      description: 'Freigabe-Rollen Lesen, Kommentieren & freigeben, Mitsteuern; Viewer höchstens Lesen; Warnung bei Mitsteuern; gesperrte Kanäle für Lesende.',
      features: ['AUTH-015'],
      states: [
        { id: 'share', title: 'Teilen' },
        { id: 'drive', title: 'Mitsteuern (Warnung)' },
        { id: 'viewer', title: 'Als Lesende' },
      ],
      component: SessionSharing,
    },
    {
      id: 'git-connections',
      title: 'Git-Verbindungen',
      description: 'GitHub, GitHub Enterprise und GitLab per OAuth (Team) oder per PAT (lokal); Device-Flow; automatische Erneuerung und Aufforderung bei Fehlschlag.',
      features: ['SEC-009', 'SEC-010', 'SEC-011'],
      states: [
        { id: 'connected', title: 'Verbunden (Team)' },
        { id: 'device-flow', title: 'GitHub verbinden' },
        { id: 'refresh-failed', title: 'GitLab abgelaufen' },
        { id: 'local-pat', title: 'Lokal mit PAT' },
      ],
      component: GitConnections,
    },
  ],
}
