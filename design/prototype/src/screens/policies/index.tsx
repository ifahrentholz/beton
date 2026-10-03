import type { ScreenGroup } from '@/proto/types'
import { PolicyApprovals } from './approvals'
import { PolicyAudit } from './audit'
import { PolicyBudgets } from './budgets'
import { PolicyEditor } from './editor'
import { PolicyEnforcement } from './enforcement'
import { PolicyExplain } from './explain'
import { PolicyOverview } from './overview'
import { PolicyShadow, PolicyTests } from './tests'

/**
 * D-5 · Policies: Ebenen (Org → Team → User → Projekt → Agent), Editor mit CEL-Prüfung,
 * Explain/Dry-Run, Shadow-Modus, Tests, Freigabe-Varianten, Durchsetzung, Audit, Budgets.
 */
export const group: ScreenGroup = {
  id: 'policies',
  title: 'Policies',
  order: 100,
  screens: [
    {
      id: 'policy-overview',
      title: 'Policies nach Ebenen',
      description:
        'Alle geltenden Regeln von Org bis Agent mit Herkunft, Typ, Phase und Aktion. Rechts die Regel „strengere gewinnt“ an einem Beispiel; Org und Team kommen nur im Team-Betrieb vom Server und sind lokal nur lesbar.',
      features: [
        'POL-001', 'POL-006', 'POL-007', 'POL-008', 'POL-011', 'POL-012', 'POL-013', 'POL-014', 'POL-015',
        'POL-016', 'POL-017', 'POL-018', 'POL-019', 'POL-020', 'POL-028', 'SYNC-006',
      ],
      states: [
        { id: 'local', title: 'Lokal (ohne Server)' },
        { id: 'team', title: 'Team-Betrieb' },
        { id: 'reload-error', title: 'Hot-Reload fehlgeschlagen' },
      ],
      component: PolicyOverview,
    },
    {
      id: 'policy-editor',
      title: 'Policy-Editor',
      description:
        'YAML mit CEL-Ausdrücken, geprüft gegen Schema und Typen der jeweiligen Phase. Gespeicherte Änderungen gelten ab der nächsten Auswertung; Fehler lassen die letzte gültige Version aktiv.',
      features: [
        'POL-001', 'POL-002', 'POL-003', 'POL-004', 'POL-005', 'POL-006', 'POL-008', 'POL-009', 'POL-010',
        'POL-011', 'POL-012', 'POL-013', 'POL-014', 'POL-015', 'POL-016', 'POL-017', 'POL-018', 'POL-019', 'POL-020', 'SYNC-006',
      ],
      states: [
        { id: 'valid', title: 'Gültig, Hot-Reload' },
        { id: 'type-error', title: 'CEL-Typfehler' },
        { id: 'phase-error', title: 'Variable in falscher Phase' },
        { id: 'duplicate-id', title: 'Doppelte Regel-ID' },
        { id: 'readonly-org', title: 'Org-Policy (nur lesend)' },
      ],
      component: PolicyEditor,
    },
    {
      id: 'policy-explain',
      title: 'Warum? – Explain und Dry-Run',
      description:
        'Aus einer Ablehnung im Verlauf heraus: die Auswertung Schritt für Schritt – Anfrage, Set-Version, Änderungen, Verdikte je Ebene, Defaults, Ergebnis. „Was wäre, wenn …“ wertet ohne Seiteneffekte aus.',
      features: ['POL-027', 'POL-025', 'POL-007', 'POL-005', 'POL-013', 'POL-017'],
      states: [
        { id: 'deny', title: 'Abgelehnt (Force-Push)' },
        { id: 'default', title: 'Default nicht aufgehoben' },
        { id: 'modify', title: 'Modell geändert, Konflikt' },
        { id: 'eval', title: 'Was wäre, wenn …' },
      ],
      component: PolicyExplain,
    },
    {
      id: 'policy-shadow',
      title: 'Shadow-Modus',
      description: 'Regeln im Schattenbetrieb: Treffer je Regel und Tag als „würde ablehnen“, ohne Sessions zu stören; Scharfschalten nach Bestätigung.',
      features: ['POL-027', 'POL-025'],
      states: [
        { id: 'active', title: 'Mit Treffern' },
        { id: 'empty', title: 'Keine Shadow-Regel' },
      ],
      component: PolicyShadow,
    },
    {
      id: 'policy-tests',
      title: 'Policy-Tests',
      description: 'Deklarative Tests in YAML mit grün/rot je Fall; bei Fehlschlag erwartete und tatsächliche Entscheidung samt Explain-Trace.',
      features: ['POL-026'],
      states: [
        { id: 'passing', title: 'Alle grün' },
        { id: 'failing', title: 'Ein Test rot' },
        { id: 'running', title: 'Läuft' },
      ],
      component: PolicyTests,
    },
    {
      id: 'policy-approvals',
      title: 'Freigaben aus Policies',
      description:
        'Varianten der Freigabe-Karte: mehrere Gründe in einer Karte, Ablauf und „für diese Session erlauben“, nicht merkbare Regeln, abgelaufene Freigaben, nur Lesen, Session-Start, Schleife und Kostenschwelle.',
      features: ['POL-003', 'POL-006', 'POL-009', 'POL-010', 'POL-011', 'POL-013', 'POL-014', 'POL-018', 'AUTH-015'],
      states: [
        { id: 'two-reasons', title: 'Zwei Gründe, Timeout' },
        { id: 'granted', title: 'Für Session erlaubt' },
        { id: 'no-remember', title: 'Nicht merkbar' },
        { id: 'expired', title: 'Abgelaufen' },
        { id: 'view-only', title: 'Nur Leserecht' },
        { id: 'session-start', title: 'Session-Start' },
        { id: 'loop', title: 'Schleife erkannt' },
        { id: 'spend', title: 'Kostenschwelle' },
      ],
      component: PolicyApprovals,
    },
    {
      id: 'policy-enforcement',
      title: 'Durchsetzung je Harness',
      description:
        'Enforcement-Matrix je Harness, Transport und Phase. Ist eine Pflichtregel nicht durchsetzbar, startet die Session nicht (fail closed); bei lesenden Klassen gibt es eine Warnung.',
      features: ['POL-022', 'POL-023', 'POL-024'],
      states: [
        { id: 'matrix', title: 'Matrix' },
        { id: 'start-denied', title: 'Start verweigert' },
        { id: 'degraded', title: 'Eingeschränkt (Warnung)' },
      ],
      component: PolicyEnforcement,
    },
    {
      id: 'policy-audit',
      title: 'Entscheidungsprotokoll',
      description: 'policy.decision-Ereignisse einer Session: Phase, Ergebnis, Regeln, Durchsetzung, Laufzeit; Detail als JSON ohne Tool-Argumente im Klartext.',
      features: ['POL-025'],
      states: [
        { id: 'all', title: 'Alle' },
        { id: 'filtered', title: 'Nur Eingriffe' },
      ],
      component: PolicyAudit,
    },
    {
      id: 'policy-budgets',
      title: 'Budgets und Offline-Reserve',
      description:
        'Kostengrenzen pro Session und Tag. Im Team-Betrieb hält der Rechner eine Budget-Lease, die offline weiter gilt; ist sie aufgebraucht, fragt beton nach.',
      features: ['POL-011', 'POL-012', 'POL-021', 'SYNC-007'],
      states: [
        { id: 'local', title: 'Lokal-only' },
        { id: 'online', title: 'Team, online' },
        { id: 'offline', title: 'Offline mit Lease' },
        { id: 'exhausted', title: 'Lease aufgebraucht' },
      ],
      component: PolicyBudgets,
    },
  ],
}
