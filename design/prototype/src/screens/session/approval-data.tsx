// Beispieldaten für die Freigabe-Leiste im Session-Verlauf (WEB-007). Realistisch, aber erfunden.

import type { BarItem } from '@/app/kit/approvals'
import { rateLimiterDiff } from '@/mock/data'

export const pushApproval: BarItem = {
  id: 'apr_push',
  kind: 'approval',
  tool: 'shell',
  title: 'Shell-Kommando',
  summary: 'git push -u origin beton/rate-limiter-7f3k',
  requestedBy: 'Claude Code',
  reasons: [
    {
      rule: 'git-push-fragen',
      origin: 'Projekt · shop-frontend',
      text: 'Pushes verlassen deinen Rechner. Die Projekt-Policy verlangt dafür deine Freigabe.',
    },
  ],
  args: `{
  "command": "git push -u origin beton/rate-limiter-7f3k",
  "description": "Branch zum Remote pushen",
  "timeout": 120000
}`,
  timeout: 'läuft in 28 Min. ab · dann ablehnen',
  remember: true,
}

export const editApproval: BarItem = {
  id: 'apr_edit',
  kind: 'approval',
  tool: 'file',
  title: 'Datei bearbeiten',
  summary: 'src/routes/auth.ts',
  requestedBy: 'Claude Code',
  reasons: [
    {
      rule: 'auth-code-review',
      origin: 'Projekt · shop-frontend',
      text: 'Änderungen an Auth-Routen nur nach Blick auf den Diff',
      severity: 'high',
    },
  ],
  diff: { file: 'src/routes/auth.ts', lines: rateLimiterDiff, stat: '+2 −1' },
  timeout: 'läuft in 29 Min. ab · dann ablehnen',
  remember: 'Merken ist für diese Regel abgeschaltet (remember: none).',
}

export const prApproval: BarItem = {
  id: 'apr_pr',
  kind: 'approval',
  tool: 'mcp',
  title: 'MCP-Tool github/create_pull_request',
  summary: 'beton/rate-limiter-7f3k → main · „Rate-Limiter für die Login-API“',
  requestedBy: 'Claude Code · Agent pr-fixer',
  reasons: [
    { rule: 'pr-needs-human', origin: 'User · defaults', text: 'PRs legt nur ein Mensch frei' },
    { rule: 'require-approval-mcp', origin: 'Projekt · shop-frontend', text: 'Schreibende GitHub-Tools bestätigen (require_approval)' },
  ],
  args: `{
  "owner": "ifahrentholz",
  "repo": "shop",
  "head": "beton/rate-limiter-7f3k",
  "base": "main",
  "title": "Rate-Limiter für die Login-API",
  "body": "Begrenzt /login auf 5 Versuche pro Minute und IP.\\n\\n- Middleware rate-limit.ts\\n- 4 neue Tests",
  "draft": false
}`,
  timeout: 'läuft in 4 Min. ab · dann abbrechen',
  urgent: true,
  remember: 'Merken nicht möglich: pr-needs-human erlaubt kein „für diese Session“.',
  note: (
    <p className="mt-2 text-[12px] text-muted-foreground">
      Zwei Regeln fragen nach – eine Karte. Ablauf nach der kürzesten Frist; danach gilt der strengste Wert beider Regeln: abbrechen.
    </p>
  ),
}

export const lintQuestion: BarItem = {
  id: 'ask_lint',
  kind: 'question',
  title: 'Limiter pro IP oder pro Account?',
  summary: 'Soll der Limiter pro IP oder pro Account zählen? Hinter dem Load-Balancer sehen alle Anfragen aus dem Büro wie eine IP aus.',
  requestedBy: 'Claude Code (ask_user)',
  options: ['Pro IP', 'Pro Account', 'Beides'],
  timeout: 'läuft in 23 Std. ab · dann „keine Antwort“',
}
