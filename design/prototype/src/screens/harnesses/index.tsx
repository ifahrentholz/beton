import type { ScreenGroup } from '@/proto/types'
import { AcpScreen } from './acp'
import { CatalogScreen } from './catalog'
import { ImportScreen } from './import'
import { NoticesScreen } from './notices'
import { ProvidersScreen } from './providers'
import { SetupScreen } from './setup'
import { SwitchingScreen } from './switching'
import { DevTerminal, SetupTerminal } from './terminal'

/**
 * Einrichtung & Harnesses (Design-Paket D-3).
 * Prämisse überall sichtbar: Abos laufen über die offiziellen CLIs, beton speichert keine Tokens;
 * API-Schlüssel und Gateways sind eine Option, nie Voraussetzung.
 */
export const group: ScreenGroup = {
  id: 'harnesses',
  title: 'Einrichtung & Harnesses',
  order: 120,
  screens: [
    {
      id: 'harness-setup',
      title: 'Einrichtung',
      description:
        'Ergebnis von beton setup in der App: erkannte CLIs mit Pfad und Version, Login-Status aus der CLI selbst, lokale Modell-Server. Installationen nur auf Klick.',
      features: ['HAR-015', 'HAR-016', 'HAR-003', 'HAR-008', 'HAR-011'],
      states: [
        { id: 'first-run', title: 'Erste Einrichtung' },
        { id: 'install', title: 'Codex installieren' },
        { id: 'ready', title: 'Alles eingerichtet' },
        { id: 'auth-expired', title: 'Login abgelaufen' },
      ],
      component: SetupScreen,
    },
    {
      id: 'harness-setup-cli',
      title: 'beton setup & doctor im Terminal',
      description: 'Dieselbe Erkennung auf der Kommandozeile, inklusive Installationsangebot mit Rückfrage und maschinenlesbarem Status.',
      features: ['HAR-016', 'HAR-015', 'HAR-003', 'HAR-014'],
      frame: 'terminal',
      states: [
        { id: 'setup', title: 'beton setup' },
        { id: 'doctor', title: 'beton doctor' },
        { id: 'json', title: '--check --json' },
      ],
      component: SetupTerminal,
    },
    {
      id: 'harness-catalog',
      title: 'Harness-Katalog',
      description:
        'Fähigkeiten aller Harnesses als Matrix: Transport, Freigaben, Modellwechsel, Fortsetzen, Fork-History, Verbrauch. Daraus leitet die UI ab, was in einer Session möglich ist.',
      features: [
        'HAR-001', 'HAR-002', 'HAR-003', 'HAR-004', 'HAR-005', 'HAR-006', 'HAR-007', 'HAR-008', 'HAR-009', 'HAR-010', 'HAR-011',
        'HAR-012', 'HAR-013', 'HAR-014', 'HAR-015', 'HAR-017', 'HAR-019', 'HAR-020', 'HAR-021', 'HAR-022', 'HAR-026',
      ],
      states: [
        { id: 'native', title: 'Strukturiert' },
        { id: 'tui', title: 'Original-TUI' },
        { id: 'detail', title: 'Details Claude Code' },
        { id: 'incompatible', title: 'Version inkompatibel' },
      ],
      component: CatalogScreen,
    },
    {
      id: 'harness-acp',
      title: 'ACP-Agents registrieren',
      description: 'Presets für Gemini CLI, Goose und Qwen werden aktiv, sobald das Programm gefunden wird. Eigene ACP-Agents mit Testlauf hinzufügen.',
      features: ['HAR-007', 'HAR-008', 'HAR-009'],
      states: [
        { id: 'presets', title: 'Übersicht' },
        { id: 'custom', title: 'Eigenen Agent hinzufügen' },
        { id: 'invalid', title: 'Ungültiger Eintrag' },
      ],
      component: AcpScreen,
    },
    {
      id: 'harness-providers',
      title: 'Direkt-API & Gateways',
      description:
        'Optionale Modell-Anbieter für den eingebauten Agent-Loop: Ollama lokal ohne Schlüssel, Gateways wie OpenRouter mit Schlüssel aus einer Umgebungsvariable.',
      features: ['HAR-010', 'HAR-011', 'HAR-015', 'HAR-016'],
      states: [
        { id: 'list', title: 'Übersicht' },
        { id: 'add', title: 'Anbieter hinzufügen' },
        { id: 'env-missing', title: 'Variable fehlt' },
        { id: 'stale', title: 'Modell-Liste veraltet' },
      ],
      component: ProvidersScreen,
    },
    {
      id: 'harness-switching',
      title: 'Modell, Effort & Modus wechseln',
      description:
        'Wechsel während der Session: live, wenn der Harness es kann, sonst per Neustart mit Fortsetzen. Nicht unterstützte Effort-Stufen werden gemeldet; YOLO nur mit Sandbox.',
      features: ['HAR-017', 'HAR-027'],
      states: [
        { id: 'model-picker', title: 'Modell wählen' },
        { id: 'switched', title: 'Live gewechselt' },
        { id: 'restart', title: 'Wechsel per Neustart (ACP)' },
        { id: 'effort-mapped', title: 'Effort angepasst' },
        { id: 'permission-mode', title: 'Modus wählen' },
        { id: 'yolo-blocked', title: 'YOLO ohne Sandbox' },
      ],
      component: SwitchingScreen,
    },
    {
      id: 'harness-notices',
      title: 'Harness-Ereignisse im Verlauf',
      description:
        'Was der Harness über sich meldet: Login abgelaufen, Runner-Neustart mit Fortsetzen, Übergabe beim Fork, MCP-Server-Fehler, unbekannte Meldungen, Compaction.',
      features: ['HAR-001', 'HAR-004', 'HAR-009', 'HAR-015', 'HAR-018', 'HAR-019', 'HAR-020', 'HAR-022', 'AGT-006'],
      states: [
        { id: 'auth-required', title: 'Login abgelaufen' },
        { id: 'runner-restart', title: 'Fortgesetzt (warm)' },
        { id: 'cold-resume', title: 'Fortgesetzt (Übergabe)' },
        { id: 'fork-preamble', title: 'Fork auf anderen Harness' },
        { id: 'fork-rebuild', title: 'Fork, gleicher Harness' },
        { id: 'mcp-failed', title: 'MCP-Server fehlt' },
        { id: 'unmapped', title: 'Unbekannte Meldung' },
        { id: 'compaction', title: 'Compaction' },
      ],
      component: NoticesScreen,
    },
    {
      id: 'harness-import',
      title: 'Transcripts importieren',
      description: 'Lokale Sessions aus Claude Code und Codex finden, das Parser-Ergebnis vor dem Import prüfen und danach in beton weiterarbeiten.',
      features: ['HAR-023', 'HAR-024', 'HAR-019'],
      states: [
        { id: 'discover', title: 'Gefunden' },
        { id: 'parsed', title: 'Claude-Session geprüft' },
        { id: 'codex', title: 'Codex-Session geprüft' },
        { id: 'empty', title: 'Nichts gefunden' },
      ],
      component: ImportScreen,
    },
    {
      id: 'harness-dev',
      title: 'Entwickler: Golden-Transcripts & Fake-Harness',
      description:
        'Kein Teil der App für Nutzer: Werkzeuge im Repo, mit denen Adapter gegen echte Aufnahmen getestet und Szenarien ohne Abo abgespielt werden.',
      features: ['HAR-025', 'HAR-026'],
      frame: 'terminal',
      states: [
        { id: 'golden', title: 'Golden-Test mit Abweichung' },
        { id: 'record-abort', title: 'Aufnahme abgebrochen' },
        { id: 'fake', title: 'Fake-Harness' },
      ],
      component: DevTerminal,
    },
  ],
}
