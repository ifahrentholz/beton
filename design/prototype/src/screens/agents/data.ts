// Beispieldaten der Gruppe „Agents“. Realistisch, aber erfunden.
import type { HarnessId } from '@/mock/data'

export type AgentSource = 'builtin' | 'project' | 'user'

export type AgentEntry = {
  name: string
  description: string
  version: string
  source: AgentSource
  path: string
  harness: HarnessId
  /** Weitere Harnesses über Sub-Agents. */
  also?: HarnessId[]
  mode: string
  lastRun?: string
  schedule?: string
}

export const sourceLabel: Record<AgentSource, string> = {
  builtin: 'Mitgeliefert',
  project: 'Projekt',
  user: 'Für mich',
}

export const agents: AgentEntry[] = [
  {
    name: 'maestra',
    description: 'Zerlegt ein Ziel, delegiert an Implementierer mit eigenem Worktree und lässt jedes Ergebnis von einem anderen Vendor reviewen.',
    version: '1.0.0',
    source: 'builtin',
    path: 'builtin:maestra',
    harness: 'claude',
    also: ['codex'],
    mode: 'Orchestrator · schreibt keinen Code',
    lastRun: 'vor 25 Min.',
  },
  {
    name: 'duetto',
    description: 'Stellt eine Frage zwei Stimmen auf verschiedenen Harnesses, lässt sie sich gegenseitig kritisieren und fasst Konsens und Dissens zusammen.',
    version: '1.0.0',
    source: 'builtin',
    path: 'builtin:duetto',
    harness: 'claude',
    also: ['codex'],
    mode: 'Debatte · nur lesend',
    lastRun: 'gestern',
  },
  {
    name: 'pr-fixer',
    description: 'Behebt fehlschlagende CI-Checks auf einem Branch und öffnet einen PR.',
    version: '0.3.0',
    source: 'project',
    path: '.beton/agents/pr-fixer',
    harness: 'claude',
    also: ['codex'],
    mode: 'Edits ohne Rückfrage',
    lastRun: 'heute, 03:00',
    schedule: 'täglich 03:00',
  },
  {
    name: 'a11y-auditor',
    description: 'Prüft geänderte Komponenten auf Barrierefreiheit (WCAG 2.2 AA) und schreibt Befunde als Kommentare.',
    version: '0.1.0',
    source: 'project',
    path: '.beton/agents/a11y-auditor',
    harness: 'gemini',
    mode: 'Nur planen',
    lastRun: 'Mo.',
  },
  {
    name: 'dep-updater',
    description: 'Aktualisiert Abhängigkeiten, liest Changelogs, passt Breaking Changes an und öffnet einen PR pro Paketgruppe.',
    version: '1.2.0',
    source: 'user',
    path: '~/.beton/agents/dep-updater',
    harness: 'codex',
    mode: 'Edits ohne Rückfrage',
    lastRun: 'Sa.',
    schedule: 'Sa. 06:00',
  },
  {
    name: 'local-scout',
    description: 'Erkundet ein unbekanntes Repo mit einem lokalen Modell und schreibt eine Übersicht nach docs/overview.md.',
    version: '0.2.0',
    source: 'user',
    path: '~/.beton/agents/local-scout',
    harness: 'ollama',
    mode: 'Fragen bei Schreibzugriff',
  },
]

export const prFixerYaml = [
  '# yaml-language-server: $schema=https://raw.githubusercontent.com/ifahrentholz/beton/main/schemas/agent.v1.json',
  'spec_version: 1',
  'name: pr-fixer',
  'description: Behebt fehlschlagende CI-Checks auf einem Branch und öffnet einen PR.',
  'version: 0.3.0',
  '',
  'executor:',
  '  harness: claude',
  '  model: claude-sonnet-5-5',
  '  reasoning_effort: xhigh',
  '  permission_mode: accept_edits',
  '  timeout: 2h',
  '',
  'instruction:',
  '  file: prompts/system.md',
  '  append: |',
  '    Arbeite nur im zugewiesenen Worktree. Öffne am Ende einen PR, merge nie.',
  '',
  'params:',
  '  branch:       { type: string, default: main }',
  '  max_attempts: { type: integer, default: 3, minimum: 1, maximum: 10 }',
  '',
  'tools:',
  '  mcp:',
  '    github:',
  '      command: github-mcp-server',
  '      args: [stdio]',
  '      env: { GITHUB_PERSONAL_ACCESS_TOKEN: "${secret:github}" }',
  '      allow: [get_pull_request, create_pull_request, list_check_runs]',
  '  system: [session_spawn, session_wait, inbox_read, policy_query, timer_set]',
  '',
  'skills: [fix-ci, ci-triage]',
  '',
  'agents:',
  '  quick-check:',
  '    executor: { harness: codex, model: gpt-5.3-codex, reasoning_effort: low, permission_mode: plan }',
  '    instructions: { text: "Reviewe nur den übergebenen Diff. Maximal 10 Punkte." }',
  'spawn: { agents: [quick-check], max_depth: 2, max_concurrent: 3, worktree: new }',
]
