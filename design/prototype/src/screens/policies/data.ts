// Beispieldaten für die Policy-Screens (D-5). Realistisch, aber erfunden.

import type { VerdictKind } from '@/app/kit/policies'

export type Scope = 'org' | 'team' | 'user' | 'project' | 'agent'

export const scopeLabel: Record<Scope, string> = {
  org: 'Org',
  team: 'Team',
  user: 'User',
  project: 'Projekt',
  agent: 'Agent',
}

export type RuleRow = {
  id: string
  type?: string
  phase: string
  verdict: VerdictKind
  summary: string
  shadow?: boolean
  hits?: number
  milestone?: string
}

export type PolicySet = {
  name: string
  file: string
  version: string
  rules: RuleRow[]
  shadow?: boolean
}

export type Layer = {
  scope: Scope
  origin: string
  server?: boolean
  sets: PolicySet[]
}

export const layers: Layer[] = [
  {
    scope: 'org',
    origin: 'Team-Server · Bundle v41, signiert',
    server: true,
    sets: [
      {
        name: 'acme-baseline',
        file: 'orgs/acme/policies/acme-baseline',
        version: 'v12 · sha256:5d0a…e1',
        rules: [
          { id: 'no-force-push', type: 'git_guard', phase: 'tool_call', verdict: 'deny', summary: 'Force-Push und Push auf main/release/* verboten', hits: 2 },
          { id: 'models', type: 'model_route', phase: 'model_request', verdict: 'modify', summary: 'Nur claude-sonnet-*, claude-haiku-*, gpt-5*; Async → Sonnet, Effort mittel' },
          { id: 'team-day', type: 'daily_budget', phase: 'model_request', verdict: 'ask', summary: 'Team-Tagesbudget 300 USD, ab 250 USD nachfragen' },
        ],
      },
    ],
  },
  {
    scope: 'team',
    origin: 'Team-Server · Team „plattform“',
    server: true,
    sets: [
      {
        name: 'plattform-team',
        file: 'teams/plattform/policies/plattform-team',
        version: 'v4 · sha256:a7c2…90',
        rules: [
          { id: 'merge-needs-human', type: 'require_approval', phase: 'tool_call', verdict: 'ask', summary: 'mcp:github/merge_pull_request immer freigeben lassen, nicht merkbar' },
          { id: 'browser', type: 'browser_guard', phase: 'browser_action', verdict: 'deny', summary: 'Passwortfelder verboten, Formular-Submit fragt nach', milestone: 'M3' },
        ],
      },
    ],
  },
  {
    scope: 'user',
    origin: '~/.beton/policies/',
    sets: [
      {
        name: 'defaults',
        file: '~/.beton/policies/defaults.yaml',
        version: 'sha256:1f9b…3c',
        rules: [
          { id: 'git-guard', type: 'git_guard', phase: 'tool_call', verdict: 'ask', summary: 'Push fragt nach, Force-Push abgelehnt, Rebase/Reset fragen', hits: 14 },
          { id: 'loop', type: 'loop_detection', phase: 'tool_call', verdict: 'ask', summary: '3 identische Calls in 10 oder 5 Fehler in Folge', hits: 3 },
          { id: 'my-day', type: 'daily_budget', phase: 'model_request', verdict: 'ask', summary: '40 USD pro Tag (Europe/Berlin), ab 30 USD nachfragen' },
          { id: 'pii', type: 'pii_redaction', phase: 'tool_result', verdict: 'modify', summary: 'E-Mail, IBAN, Kreditkarte (Luhn), API-Keys schwärzen', hits: 6 },
        ],
      },
    ],
  },
  {
    scope: 'project',
    origin: 'shop-frontend · .beton/policies/',
    sets: [
      {
        name: 'git-and-budget',
        file: '.beton/policies/git-and-budget.yaml',
        version: 'sha256:9c1e…7a',
        rules: [
          { id: 'confirm-destructive-shell', phase: 'tool_call', verdict: 'ask', summary: 'tool.kind == "shell" && tool.command.matches("rm -rf|git push")', hits: 9 },
          { id: 'route-when-expensive', phase: 'model_request', verdict: 'modify', summary: 'Ab 20 USD in der Session: Opus → Sonnet' },
          { id: 'count-pushes', phase: 'tool_call', verdict: 'notify', summary: 'Zählt Pushes (state.session.pushes) und meldet in die Inbox', hits: 4 },
          { id: 'allow-docs', phase: 'browser_navigate', verdict: 'allow', summary: '*.rust-lang.org und localhost (hebt Projekt-Default deny auf)' },
          { id: 'budget', type: 'spend_cap', phase: 'model_request', verdict: 'ask', summary: '25 USD pro Session, fragt bei 10 und 20 USD' },
          { id: 'paths', type: 'path_guard', phase: 'tool_call', verdict: 'deny', summary: 'Schreiben nur im Worktree; .env*, ~/.ssh, ~/.aws gesperrt', hits: 1 },
        ],
      },
      {
        name: 'shadow-npm',
        file: '.beton/policies/shadow-npm.yaml',
        version: 'sha256:44be…02',
        shadow: true,
        rules: [{ id: 'no-npm-publish', phase: 'tool_call', verdict: 'would_deny', summary: 'npm/pnpm publish ablehnen (Shadow-Modus)', shadow: true, hits: 7 }],
      },
    ],
  },
  {
    scope: 'agent',
    origin: 'Agent „reviewer“ · agents/reviewer/agent.yaml',
    sets: [
      {
        name: 'reviewer',
        file: 'agents/reviewer/agent.yaml#policies',
        version: 'reviewer@1.3.0',
        rules: [
          { id: 'read-only-tools', type: 'tool_allowlist', phase: 'tool_call', verdict: 'deny', summary: 'Nur file_read, search, system – Schnittmenge mit höheren Ebenen' },
          { id: 'risk-score', type: 'wasm', phase: 'tool_call', verdict: 'ask', summary: 'WASM-Modul acme/risk@1.2.0: fragt bei mehr als 3 Pipes', milestone: 'M5' },
        ],
      },
    ],
  },
]

export const builtinTypes: { type: string; desc: string; milestone?: string }[] = [
  { type: 'spend_cap', desc: 'Kostengrenze pro Session, Run oder Teilbaum' },
  { type: 'daily_budget', desc: 'Tagesbudget pro User oder Team' },
  { type: 'model_route', desc: 'Modell und Effort festlegen, Allowlist' },
  { type: 'require_approval', desc: 'Freigabe für Tools, Klassen, MCP-Server' },
  { type: 'tool_allowlist', desc: 'Nur gelistete Tools, Rest ablehnen' },
  { type: 'path_guard', desc: 'Lesen/Schreiben auf Pfaden begrenzen' },
  { type: 'git_guard', desc: 'Push, Force-Push, Merge, Rewrite schützen' },
  { type: 'loop_detection', desc: 'Festgefahrene Agents erkennen' },
  { type: 'pii_redaction', desc: 'PII und Secrets in Ergebnissen schwärzen' },
  { type: 'browser_guard', desc: 'Navigation, Formulare, Downloads im Browser', milestone: 'M3' },
]

export const policyYaml = `# .beton/policies/git-and-budget.yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/ifahrentholz/beton/main/schemas/policy.v1.json
spec_version: 1
name: git-and-budget
description: Projekt-Regeln für Git und Kosten
mode: enforce                        # enforce | dry_run (Shadow)
defaults:
  browser_navigate: deny
rules:
  - id: confirm-destructive-shell
    on: tool_call
    when: tool.kind == "shell" && tool.command.matches("rm -rf|git push")
    action: ask
    severity: high
    reason: "Destruktives oder veröffentlichendes Shell-Kommando"
    approval: { timeout: 30m, on_timeout: deny, remember: session }

  - id: route-when-expensive
    on: model_request
    when: session.cost_usd > 20.0 && request.model.startsWith("claude-opus")
    action: modify
    modify: { model: claude-sonnet-5-5 }
    on_unsupported: deny
    reason: "Session über 20 USD: auf Sonnet umrouten"

  - id: count-pushes
    on: tool_call
    when: tool.kind == "shell" && shell.has_command(tool.argv, "git", "push")
    action: notify
    notify: { channels: [inbox], message: "Push Nr. {{ state.session.pushes + 1 }}" }
    effects:
      - { op: increment, scope: session, key: pushes }

  - id: allow-docs
    on: browser_navigate
    when: glob(browser.host, "*.rust-lang.org") || browser.host == "localhost"
    action: allow

  - id: budget
    type: spend_cap
    params: { scope: session, limit_usd: 25, ask_at_usd: [10, 20], on_exceed: deny }
`

export const contextVars: { name: string; type: string; note?: string }[] = [
  { name: 'tool.kind', type: 'string', note: 'shell | file_read | file_write | file_edit | search | web_fetch | mcp | system | browser | other' },
  { name: 'tool.name / native_name', type: 'string', note: 'z. B. Bash, exec_command, mcp:github/create_pull_request' },
  { name: 'tool.command', type: 'string', note: 'Shell-String' },
  { name: 'tool.argv', type: 'list<list<string>>', note: 'Teilkommandos, an ; && || | getrennt' },
  { name: 'tool.paths', type: 'list<string>', note: 'absolut, normalisiert, Symlinks aufgelöst' },
  { name: 'tool.repeat_count', type: 'int' },
  { name: 'tool.parse_uncertain', type: 'bool', note: 'eingebaute Typen fragen dann nach' },
  { name: 'session.cost_usd', type: 'double' },
  { name: 'session.branch / worktree', type: 'string' },
  { name: 'user.daily_cost_usd', type: 'double' },
  { name: 'budget.online / lease_remaining_usd', type: 'bool / double' },
  { name: 'state.session.*', type: 'map', note: 'eigener Zustand über effects' },
]
