import type { ScreenGroup } from '@/proto/types'
import { AgentDetail } from './detail'
import { DuettoRun } from './duetto'
import { AgentLibrary } from './library'
import { MaestraRun } from './maestra'
import { AgentStart } from './start'
import { AgentCli } from './terminal'
import { SubAgentTree } from './tree'

/**
 * Agents (Design-Paket D-3): YAML-Agents, die auf Harnesses laufen. Herzstück ist der maestra-Lauf
 * als Partitur: Stimmen = Sub-Agents auf verschiedenen Harnesses, Fermaten = Freigaben.
 */
export const group: ScreenGroup = {
  id: 'agents',
  title: 'Agents',
  order: 60,
  screens: [
    {
      id: 'agent-library',
      title: 'Agent-Bibliothek',
      description:
        'Mitgelieferte Agents (maestra, duetto) und eigene aus Projekt und Home-Verzeichnis, mit Quelle, Version und Harness. Verschattungen und ungültige Ordner werden gemeldet.',
      features: ['AGT-001', 'AGT-003', 'AGT-011', 'AGT-012'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'shadowed', title: 'Verschattung & ungültig' },
        { id: 'empty', title: 'Keine eigenen Agents' },
      ],
      component: AgentLibrary,
    },
    {
      id: 'agent-detail',
      title: 'Agent-Detail',
      description:
        'Ein Agent mit allem, was beim Start aufgelöst wird: Ausführung, Anweisungen, Parameter, Sub-Agents, Tools als MCP-Server, System-Tools, Skills, Policies und Sandbox. Die agent.yaml mit Schema-Prüfung.',
      features: [
        'AGT-001', 'AGT-002', 'AGT-004', 'AGT-005', 'AGT-006', 'AGT-007', 'AGT-008', 'AGT-009', 'AGT-010', 'AGT-014',
        'HAR-009', 'HAR-027', 'ASY-004', 'ASY-008',
      ],
      states: [
        { id: 'overview', title: 'Übersicht' },
        { id: 'yaml-errors', title: 'YAML mit Fehlern' },
        { id: 'yaml-valid', title: 'YAML gültig' },
        { id: 'tools', title: 'Tools' },
        { id: 'skills', title: 'Skills' },
        { id: 'policies', title: 'Policies & Sandbox' },
      ],
      component: AgentDetail,
    },
    {
      id: 'agent-start',
      title: 'Agent starten',
      description:
        'Start mit Aufgabe, typisierten Parametern und Vorab-Prüfung der Harnesses. Fehlt ein Vendor, weicht maestra aus; duetto startet ohne zweite Stimme nicht.',
      features: ['AGT-004', 'AGT-010', 'AGT-011', 'AGT-012'],
      states: [
        { id: 'form', title: 'pr-fixer starten' },
        { id: 'param-error', title: 'Parameter ungültig' },
        { id: 'maestra-fallback', title: 'maestra ohne Codex' },
        { id: 'duetto-missing', title: 'duetto ohne zweite Stimme' },
      ],
      component: AgentStart,
    },
    {
      id: 'maestra-score',
      title: 'maestra-Lauf als Partitur',
      description:
        'Ein Orchestrator-Lauf als Notensystem: Jede Stimme ist ein Sub-Agent auf seinem Harness, Takte sind Turns, gestrichelte Linien Übergaben, gelbe Fermaten Freigaben, auf die du antworten musst.',
      features: ['AGT-011', 'AGT-009', 'AGT-007', 'ASY-002', 'ASY-010', 'HAR-005', 'HAR-021'],
      states: [
        { id: 'running', title: 'Läuft, Freigabe offen' },
        { id: 'done', title: 'Fertig' },
        { id: 'needs-human', title: 'Nach 3 Runden an dich' },
        { id: 'same-vendor', title: 'Ohne Codex (same-vendor)' },
      ],
      component: MaestraRun,
    },
    {
      id: 'duetto-debate',
      title: 'duetto-Debatte',
      description: 'Eine Frage, zwei Stimmen auf verschiedenen Harnesses: Antworten nebeneinander, Kritikrunden, am Ende Konsens, Dissens und Urteil.',
      features: ['AGT-012', 'AGT-009'],
      states: [
        { id: 'answers', title: 'Antworten' },
        { id: 'debate', title: 'Kritikrunde 2' },
        { id: 'verdict', title: 'Urteil' },
      ],
      component: DuettoRun,
    },
    {
      id: 'subagent-tree',
      title: 'Sub-Agents über Harness-Grenzen',
      description:
        'Eine Claude-Session startet Codex-Reviews im Hintergrund. Rechts der Session-Baum mit Worktrees und Grenzen; Ergebnisse wecken die Parent-Session.',
      features: ['AGT-009', 'AGT-007', 'ASY-002'],
      states: [
        { id: 'tree', title: 'Kinder laufen' },
        { id: 'async-result', title: 'Ergebnis zugestellt' },
        { id: 'spawn-denied', title: 'Grenze erreicht' },
        { id: 'parent-cancelled', title: 'Parent abgebrochen' },
      ],
      component: SubAgentTree,
    },
    {
      id: 'agent-cli',
      title: 'Agents im Terminal',
      description: 'beton agent list, show, validate und new: dieselben Informationen und Prüfungen wie in der App.',
      features: ['AGT-013', 'AGT-001', 'AGT-002', 'AGT-003', 'AGT-004'],
      frame: 'terminal',
      states: [
        { id: 'list', title: 'list --all' },
        { id: 'show', title: 'show' },
        { id: 'validate', title: 'validate' },
        { id: 'new', title: 'new' },
      ],
      component: AgentCli,
    },
  ],
}
