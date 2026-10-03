import type { ScreenGroup } from '@/proto/types'
import { AgentsScreen } from './agents'
import { ChangesScreen } from './changes'
import { CliScreen } from './cli'
import { ComposerScreen } from './composer'
import { FilesScreen } from './files'
import { ForkScreen } from './fork'
import { ImportExportScreen } from './import-export'
import { LifecycleScreen } from './lifecycle'
import { ProjectsScreen } from './projects'
import { ResumeScreen } from './resume'
import { TerminalsScreen } from './terminals'
import { NewSessionScreen, WorktreesScreen } from './worktrees'

/**
 * Workspace: alles rund um die Arbeit in einer Session – Dateien, Änderungen, Terminals,
 * Sub-Agents, Worktrees, Projects, Fork, Import/Export, Fortsetzen, Titel und Kontext.
 */
export const group: ScreenGroup = {
  id: 'workspace',
  title: 'Workspace',
  order: 20,
  screens: [
    {
      id: 'workspace-files',
      title: 'Dateien & Editor',
      description:
        'Dateibaum und Editor im Workspace-Rail: Dateien des Worktrees lesen, durchsuchen und bearbeiten, Zeilen an den Agent anhängen. Ändert der Agent eine Datei mit ungespeicherten Änderungen, fragt der Editor nach.',
      features: ['SES-017', 'WEB-009'],
      states: [
        { id: 'editor', title: 'Zeilen markiert' },
        { id: 'attached', title: 'An Agent angehängt' },
        { id: 'search', title: 'Inhaltssuche' },
        { id: 'preview', title: 'Markdown-Vorschau' },
        { id: 'conflict', title: 'Konflikt mit Agent' },
        { id: 'outside', title: 'Außerhalb des Workspace' },
      ],
      component: FilesScreen,
    },
    {
      id: 'workspace-composer',
      title: 'Eingabe: Anhänge, @ und /',
      description: 'Dateien per @ referenzieren, Befehle und Skills per / aufrufen, Bilder und PDFs anhängen. Limits erklärt die Eingabe direkt an der Datei.',
      features: ['WEB-006'],
      states: [
        { id: 'mention', title: '@-Dateisuche' },
        { id: 'slash', title: 'Slash-Menü' },
        { id: 'attachments', title: 'Anhänge' },
        { id: 'too-large', title: 'Datei zu groß' },
      ],
      component: ComposerScreen,
    },
    {
      id: 'workspace-changes',
      title: 'Änderungen',
      description: 'Was der Agent geändert hat, in drei Sichten: nicht committet, Branch gegenüber Base und pro Turn. Jede Diff-Zeile ist verlinkbar und lässt sich an den Agent anhängen.',
      features: ['SES-018'],
      states: [
        { id: 'uncommitted', title: 'Nicht committet' },
        { id: 'branch', title: 'Branch' },
        { id: 'turn', title: 'Pro Turn' },
        { id: 'empty', title: 'Keine Änderungen' },
        { id: 'no-git', title: 'Kein Git-Repository' },
      ],
      component: ChangesScreen,
    },
    {
      id: 'workspace-terminals',
      title: 'Terminals',
      description:
        'Benannte Terminals pro Session, in der Sandbox der Session. Vom Agent gestartete Dev-Server laufen weiter, wenn alle Fenster zu sind. Sessions im PTY-Modus zeigen hier die Original-Oberfläche des Harness.',
      features: ['SES-019', 'WEB-010', 'GIT-004'],
      states: [
        { id: 'shell', title: 'Eigene Shell' },
        { id: 'dev', title: 'Dev-Server vom Agent' },
        { id: 'reattach', title: 'Wieder verbunden' },
        { id: 'tui', title: 'Harness im PTY' },
        { id: 'readonly', title: 'Nur zusehen' },
      ],
      component: TerminalsScreen,
    },
    {
      id: 'workspace-agents',
      title: 'Sub-Agents',
      description: 'Die Session und ihre Sub-Agents als Graph mit Harness, Status und Verbrauch, darunter der Ablauf als Partitur. Klick auf einen Knoten öffnet die Sub-Session.',
      features: ['WEB-012'],
      states: [
        { id: 'live', title: 'Neuer Sub-Agent' },
        { id: 'done', title: 'Alle fertig' },
        { id: 'empty', title: 'Keine Sub-Agents' },
      ],
      component: AgentsScreen,
    },
    {
      id: 'workspace-new-session',
      title: 'Neue Session mit Worktree',
      description: 'Eine neue Session übernimmt die Vorgaben ihres Projects und bekommt einen eigenen Worktree. Ist die Base nicht auflösbar, startet sie nicht stillschweigend ohne.',
      features: ['SES-013', 'SES-015'],
      states: [
        { id: 'default', title: 'Vorgaben übernommen' },
        { id: 'branch-picker', title: 'Worktree wählen' },
        { id: 'base-error', title: 'Base fehlt' },
      ],
      component: NewSessionScreen,
    },
    {
      id: 'workspace-worktrees',
      title: 'Worktrees',
      description: 'Alle Worktrees eines Projects mit Zustand und Größe. Löschen fragt nach, bevor ungesicherte Arbeit oder ungepushte Commits verloren gehen.',
      features: ['SES-015', 'SES-016'],
      states: [
        { id: 'list', title: 'Übersicht' },
        { id: 'delete-dirty', title: 'Löschen: nicht committet' },
        { id: 'delete-unpushed', title: 'Löschen: ungepusht' },
      ],
      component: WorktreesScreen,
    },
    {
      id: 'workspace-projects',
      title: 'Projects',
      description: 'Ein Project bündelt Sessions eines Repos, liefert Vorgaben für neue Sessions und trägt die Projekt-Ebene der Policies.',
      features: ['SES-013', 'SES-014'],
      states: [
        { id: 'defaults', title: 'Vorgaben' },
        { id: 'policies', title: 'Policies' },
        { id: 'delete', title: 'Löschen' },
      ],
      component: ProjectsScreen,
    },
    {
      id: 'workspace-fork',
      title: 'Fork & „Weiter mit …“',
      description:
        'Ab einer Stelle eine neue Session abzweigen, auf demselben oder einem anderen Harness. „Weiter mit Codex“ im Harness-Picker forkt ab jetzt; die Ursprungs-Session bleibt bei ihrem Harness.',
      features: ['SES-006', 'SES-007'],
      states: [
        { id: 'dialog', title: 'Fork-Dialog' },
        { id: 'continue', title: 'Weiter mit Codex' },
        { id: 'forked', title: 'Fork läuft' },
        { id: 'incompatible', title: 'Harness passt nicht' },
      ],
      component: ForkScreen,
    },
    {
      id: 'workspace-import-export',
      title: 'Import & Export',
      description: 'Bestehende Claude-Code- und Codex-Chats übernehmen und Sessions als JSONL exportieren oder importieren. Alles liest und schreibt lokale Dateien.',
      features: ['SES-008', 'SES-009'],
      states: [
        { id: 'import', title: 'Chats übernehmen' },
        { id: 'imported', title: 'Übernommen' },
        { id: 'export', title: 'Exportieren' },
        { id: 'import-error', title: 'Datei zu neu' },
      ],
      component: ImportExportScreen,
    },
    {
      id: 'workspace-sessions',
      title: 'Session-Liste & Verwaltung',
      description: 'Alle Sessions mit Filtern, Volltextsuche, Anpinnen und Umbenennen. Archivieren stoppt und blendet aus, Löschen entfernt endgültig.',
      features: ['SES-001', 'SES-010', 'SES-012', 'SES-016'],
      states: [
        { id: 'list', title: 'Liste' },
        { id: 'menu', title: 'Aktionen' },
        { id: 'search', title: 'Suche' },
        { id: 'rename', title: 'Umbenennen' },
        { id: 'archived', title: 'Archiviert' },
        { id: 'delete', title: 'Löschen' },
      ],
      component: LifecycleScreen,
    },
    {
      id: 'workspace-resume',
      title: 'Fortsetzen, Titel & Kontext',
      description:
        'Gestoppte Sessions setzen beim nächsten Senden fort, nativ oder per Übergabe. Dazu: automatischer Titel nach dem ersten Turn, Komprimieren bei vollem Kontext und Wiederverbinden ohne Lücken.',
      features: ['SES-002', 'SES-003', 'SES-010', 'SES-011'],
      states: [
        { id: 'stopped', title: 'Gestoppt' },
        { id: 'resumed', title: 'Nativ fortgesetzt' },
        { id: 'handover', title: 'Per Übergabe fortgesetzt' },
        { id: 'title', title: 'Titel erzeugt' },
        { id: 'context-high', title: 'Kontext fast voll' },
        { id: 'compacted', title: 'Komprimiert' },
        { id: 'compact-unsupported', title: 'Komprimieren nicht möglich' },
        { id: 'reconnected', title: 'Wieder verbunden' },
      ],
      component: ResumeScreen,
    },
    {
      id: 'workspace-cli',
      title: 'Sessions im Terminal',
      description: 'Dieselben Session-Funktionen über die CLI: anlegen, live mitlesen, fortsetzen, forken, exportieren und verwaiste Worktrees finden.',
      features: ['SES-001', 'SES-002', 'SES-003', 'SES-006', 'SES-007', 'SES-009', 'SES-016'],
      frame: 'terminal',
      states: [
        { id: 'attach', title: 'run & attach' },
        { id: 'resume', title: 'resume, fork, export' },
        { id: 'doctor', title: 'doctor' },
      ],
      component: CliScreen,
    },
  ],
}
