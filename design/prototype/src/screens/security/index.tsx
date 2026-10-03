import type { ScreenGroup } from '@/proto/types'
import { CredentialInjection, EgressLog, EgressProxy } from './proxy'
import { SandboxProbe, SandboxSettings, SandboxStages } from './sandbox'
import { AuditLog, Redaction, Secrets, ServerHardening } from './secrets'
import { SandboxUnavailable, SandboxViolation } from './stream'

/**
 * D-5 · Sandbox & Sicherheit: Sandbox je OS, zwei Stufen, Fähigkeitsprüfung, fail closed,
 * Verstöße im Verlauf, Egress-Proxy, Credential-Injection, Secrets, Audit, Redaction, Server-Härtung.
 */
export const group: ScreenGroup = {
  id: 'security',
  title: 'Sandbox & Sicherheit',
  order: 110,
  screens: [
    {
      id: 'sandbox-settings',
      title: 'Sandbox-Einstellungen',
      description:
        'Backend je Betriebssystem mit Fähigkeiten, Voreinstellungen, freigegebene Pfade, Masken, Environment und Grenzen. Ungültige Freigaben werden mit Grund abgelehnt.',
      features: ['SBX-001', 'SBX-003', 'SBX-004', 'SBX-005', 'SBX-006', 'SBX-007', 'SBX-008', 'SBX-009', 'SBX-010', 'SBX-012', 'SBX-013', 'SBX-015', 'SBX-016'],
      states: [
        { id: 'macos', title: 'macOS (Seatbelt)' },
        { id: 'linux', title: 'Linux (Landlock)' },
        { id: 'linux-bwrap', title: 'Linux (bubblewrap)' },
        { id: 'windows', title: 'Windows (Beta)' },
        { id: 'config-error', title: 'Ungültige Freigabe' },
      ],
      component: SandboxSettings,
    },
    {
      id: 'sandbox-stages',
      title: 'Zwei Stufen: Harness und Tools',
      description:
        'Rechte-Matrix Stufe 0/1/2 und der Weg eines Shell-Befehls über den Exec-Broker. Je Harness: welche Anmeldung die CLI lesen darf und welche Anbieter-Hosts als Tunnel laufen.',
      features: ['SBX-002', 'SBX-004', 'SBX-017', 'PRX-005'],
      states: [
        { id: 'matrix', title: 'Matrix' },
        { id: 'inherited', title: 'Adapter ohne Stufe 2' },
      ],
      component: SandboxStages,
    },
    {
      id: 'sandbox-probe',
      title: 'Fähigkeitsprüfung und Ausbruchstests',
      description: 'Was dieser Rechner isolieren kann (alle Backends), die aufgelöste Sandbox, ein Testbefehl und die lokale Ausbruchs-Testsuite.',
      features: ['SBX-001', 'SBX-008', 'SBX-009', 'SBX-010', 'SBX-011', 'SBX-015'],
      states: [
        { id: 'probe', title: 'Probe (Linux)' },
        { id: 'escape-suite', title: 'Ausbruchstests' },
      ],
      component: SandboxProbe,
    },
    {
      id: 'sandbox-unavailable',
      title: 'Sandbox verlangt, aber nicht verfügbar',
      description:
        'Fail closed: Ohne geforderte Isolation startet keine Session. Ohne Sandbox nur mit Bestätigung pro Session; Policies können das verbieten; YOLO nie ohne Sandbox; Proxy-Ausfall hält die Session an.',
      features: ['SBX-006', 'SBX-001'],
      states: [
        { id: 'unavailable', title: 'Nicht verfügbar' },
        { id: 'confirm-none', title: 'Ohne Sandbox bestätigen' },
        { id: 'forbidden', title: 'Per Policy verboten' },
        { id: 'yolo', title: 'YOLO ohne Sandbox' },
        { id: 'proxy-down', title: 'Proxy ausgefallen' },
      ],
      component: SandboxUnavailable,
    },
    {
      id: 'sandbox-violation',
      title: 'Sandbox-Verstoß im Verlauf',
      description: 'Blockierte Netz-, Datei- und Ressourcenzugriffe direkt an der Tool-Karte, mit Regelvorschlag, den beton erst nach Bestätigung schreibt.',
      features: ['SBX-013', 'SBX-014', 'PRX-007', 'PRX-009'],
      states: [
        { id: 'net', title: 'Netz blockiert' },
        { id: 'suggest', title: 'Regel vorschlagen' },
        { id: 'fs', title: 'Datei maskiert' },
        { id: 'private', title: 'Privates Ziel' },
        { id: 'limit', title: 'Prozessgrenze' },
      ],
      component: SandboxViolation,
    },
    {
      id: 'egress-proxy',
      title: 'Egress-Proxy und Regeln',
      description:
        'Default-Deny-Regeln „METHODEN host/pfad“ mit Prüfung beim Eingeben, Anbieter-Hosts als Tunnel, private Ziele, Toolchain-Integration, Protokolle und die beton-CA.',
      features: ['PRX-001', 'PRX-002', 'PRX-003', 'PRX-004', 'PRX-005', 'PRX-007', 'PRX-008', 'PRX-011'],
      states: [
        { id: 'rules', title: 'Regeln' },
        { id: 'rule-error', title: 'Ungültige Regel' },
      ],
      component: EgressProxy,
    },
    {
      id: 'egress-log',
      title: 'Egress-Log (live)',
      description: 'Jede Proxy-Entscheidung mit Stufe, Methode, Host, Pfad, Regel, Binding und Protokoll; Tunnel der Anbieter-Hosts ohne Inhalt.',
      features: ['PRX-004', 'PRX-005', 'PRX-007', 'PRX-009', 'PRX-011'],
      states: [
        { id: 'live', title: 'Alle' },
        { id: 'blocked', title: 'Nur blockiert' },
      ],
      component: EgressLog,
    },
    {
      id: 'credential-injection',
      title: 'Credential-Injection',
      description: 'Secret → Binding → Platzhalter bt_cred_… in der Sandbox → Proxy setzt den Wert nur für gebundene Hosts ein. Mit Fehlgebrauch und zurückgespiegeltem Wert.',
      features: ['PRX-006', 'PRX-010', 'SBX-005', 'SEC-012'],
      states: [
        { id: 'flow', title: 'Ablauf und Bindings' },
        { id: 'misuse', title: 'Fehlgebrauch' },
        { id: 'reflected', title: 'Zurückgespiegelter Wert' },
      ],
      component: CredentialInjection,
    },
    {
      id: 'secrets-store',
      title: 'Secrets',
      description:
        'Secrets nur als Metadaten: Schlüsselbund lokal, verschlüsselte Datei mit Passphrase als Ausweichlösung, Hinzufügen verdeckt, Übernahme eines gh-Logins nur nach Bestätigung, Team-Secrets mit Bindung und Auslieferung.',
      features: ['SEC-001', 'SEC-002', 'SEC-003', 'SEC-004', 'SEC-005', 'SEC-007', 'SEC-008', 'SEC-011'],
      states: [
        { id: 'keychain', title: 'Schlüsselbund' },
        { id: 'locked', title: 'Datei gesperrt' },
        { id: 'add', title: 'Hinzufügen' },
        { id: 'import-gh', title: 'gh-Login übernehmen' },
        { id: 'team', title: 'Team-Betrieb' },
      ],
      component: Secrets,
    },
    {
      id: 'security-audit-log',
      title: 'Audit-Log',
      description: 'Anhängbares Protokoll mit Hash-Kette: Secret-Nutzung, Fehlgebrauch, Policy-Änderungen, Pairing, CA-Rotation; Prüfung der Kette.',
      features: ['SEC-012', 'PRX-002', 'PRX-009'],
      states: [
        { id: 'list', title: 'Einträge' },
        { id: 'verify-failed', title: 'Kette unterbrochen' },
      ],
      component: AuditLog,
    },
    {
      id: 'secret-redaction',
      title: 'Redaction',
      description: 'Vorher/Nachher an Beispielen: bekannte Secrets in Tool-Ausgaben, personenbezogene Daten per Policy, Diagnose-Bundle.',
      features: ['SEC-013', 'POL-019'],
      states: [
        { id: 'tool-output', title: 'Tool-Ausgabe' },
        { id: 'pii', title: 'Personenbezogene Daten' },
        { id: 'diagnose', title: 'Diagnose-Bundle' },
      ],
      component: Redaction,
    },
    {
      id: 'server-hardening',
      title: 'Server-Härtung',
      description: 'Nur Team-Server: TLS, vertraute Proxys, Security-Header, Rate-Limits, erlaubte Hosts und Origins, Web-Sitzungen, Master-Keys mit Rewrap.',
      features: ['SEC-014', 'SEC-005', 'SEC-006', 'AUTH-002', 'AUTH-003', 'AUTH-007'],
      states: [
        { id: 'ok', title: 'In Ordnung' },
        { id: 'rewrap', title: 'Rewrap läuft' },
        { id: 'key-missing', title: 'Master-Key fehlt' },
      ],
      component: ServerHardening,
    },
  ],
}
