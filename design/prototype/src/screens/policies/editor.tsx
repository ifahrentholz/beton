import { Check, FileCode2, FlaskConical, Lock, Play, Save } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, CodeView, PrimaryButton, Shell, Tag, type CodeMark } from './kit'
import { builtinTypes, contextVars, policyYaml } from './data'

const tree: { scope: string; files: { name: string; path: string; ro?: boolean; active?: boolean }[] }[] = [
  { scope: 'Org (Team-Server)', files: [{ name: 'acme-baseline', path: 'Bundle v41', ro: true }] },
  { scope: 'Team plattform', files: [{ name: 'plattform-team', path: 'Bundle v41', ro: true }] },
  { scope: 'User', files: [{ name: 'defaults.yaml', path: '~/.beton/policies' }] },
  {
    scope: 'Projekt shop-frontend',
    files: [
      { name: 'git-and-budget.yaml', path: '.beton/policies', active: true },
      { name: 'shadow-npm.yaml', path: '.beton/policies' },
      { name: 'tests/git.policytest.yaml', path: '.beton/policies' },
    ],
  },
  { scope: 'Agent', files: [{ name: 'reviewer/agent.yaml', path: 'agents' }] },
]

const orgYaml = `# Server-Policy · Org acme · Version 12 (signiert, Bundle v41)
spec_version: 1
name: acme-baseline
rules:
  - id: no-force-push
    type: git_guard
    params:
      force_push: deny
      protected_branches: [main, "release/*"]
      protected_action: deny
      push: ask

  - id: models
    type: model_route
    params:
      allowed_models: ["claude-sonnet-*", "claude-haiku-*", "gpt-5*"]
      routes:
        - { when: "session.is_async", model: claude-sonnet-5-5, reasoning_effort: medium }

  - id: team-day
    type: daily_budget
    params: { scope: team, limit_usd: 300, ask_at_usd: [250], on_exceed: ask }
`

export function PolicyEditor({ state }: { state: string }) {
  const ro = state === 'readonly-org'
  let code = policyYaml
  let marks: CodeMark[] = []
  let problems: { line: number; text: string }[] = []

  if (state === 'type-error') {
    code = policyYaml.replace('session.cost_usd > 20.0', 'session.cost_usd > "5"')
    marks = [
      {
        line: 20,
        tone: 'deny',
        col: 10,
        len: 22,
        note: (
          <>
            <b>Typfehler:</b> <code className="font-mono">double &gt; string</code> ist nicht definiert. Meintest du{' '}
            <code className="font-mono">session.cost_usd &gt; 5.0</code>?
          </>
        ),
      },
    ]
    problems = [{ line: 20, text: 'Typfehler in when (Spalte 11): double > string' }]
  }
  if (state === 'phase-error') {
    code = policyYaml.replace('tool.kind == "shell" && tool.command.matches("rm -rf|git push")', 'tool.kind == "shell" && result.text.contains("rm -rf")')
    marks = [
      {
        line: 12,
        tone: 'deny',
        col: 34,
        len: 11,
        note: (
          <>
            <b>Variable in dieser Phase nicht verfügbar:</b> <code className="font-mono">result.*</code> gibt es nur in{' '}
            <code className="font-mono">on: tool_result</code>. Diese Regel läuft in <code className="font-mono">tool_call</code>, also
            bevor das Tool ausgeführt wird.
          </>
        ),
      },
    ]
    problems = [{ line: 12, text: 'result.text ist nur in Phase tool_result verfügbar' }]
  }
  if (state === 'duplicate-id') {
    code = policyYaml.replace('- id: count-pushes', '- id: confirm-destructive-shell')
    marks = [
      {
        line: 26,
        tone: 'deny',
        col: 8,
        len: 25,
        note: <>Regel-ID „confirm-destructive-shell“ gibt es in diesem Set schon (Zeile 10). IDs müssen pro Set eindeutig sein.</>,
      },
    ]
    problems = [{ line: 26, text: 'Doppelte Regel-ID confirm-destructive-shell' }]
  }
  if (state === 'valid') {
    marks = [{ line: 20, tone: 'changed' }]
  }
  if (ro) code = orgYaml

  const hasError = problems.length > 0

  return (
    <Shell nav="policies" connection={ro ? 'server' : 'local'}>
      <div className="flex min-h-0 flex-1">
        {/* Dateibaum nach Ebenen */}
        <F id="POL-001" className="w-60 shrink-0 overflow-y-auto border-r border-border bg-sidebar py-2">
          {tree.map((g) => (
            <div key={g.scope} className="mb-2">
              <div className="px-3 pb-0.5 text-[11px] text-muted-foreground">{g.scope}</div>
              {g.files.map((f) => {
                const active = ro ? f.name === 'acme-baseline' : f.active
                return (
                  <div
                    key={f.name}
                    className={cn('flex items-center gap-1.5 border-l-2 px-3 py-1 text-[12px]', active ? 'border-foreground bg-accent font-medium' : 'border-transparent')}
                  >
                    {f.ro ? <Lock className="size-3 text-muted-foreground" /> : <FileCode2 className="size-3 text-muted-foreground" />}
                    <span className="truncate">{f.name}</span>
                  </div>
                )
              })}
            </div>
          ))}
        </F>

        {/* Editor */}
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-11 shrink-0 items-center gap-3 border-b border-border px-4">
            <span className="font-mono text-[13px]">{ro ? 'acme-baseline (Org)' : '.beton/policies/git-and-budget.yaml'}</span>
            {ro ? (
              <F id={['SYNC-006', 'POL-008']} as="span">
                <Tag mono={false}>
                  <Lock className="mr-1 size-3" /> Vom Team-Server · nur lesend
                </Tag>
              </F>
            ) : (
              <F id={['POL-002', 'POL-001']} as="span" className="flex items-center gap-1.5 text-[12px]">
                {hasError ? (
                  <span className="inline-flex items-center gap-1.5 text-deny">
                    <span className="size-2 rotate-45 bg-deny" /> {problems.length} Fehler – nicht gespeichert
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                    <Check className="size-3.5 text-ok" /> Geprüft · gilt sofort in 3 laufenden Sessions
                  </span>
                )}
              </F>
            )}
            <div className="ml-auto flex items-center gap-2">
              {!ro && (
                <>
                  <Btn>
                    <FlaskConical className="size-3.5" /> Tests ausführen
                  </Btn>
                  <Btn>
                    <Play className="size-3.5" /> Prüfen
                  </Btn>
                  <PrimaryButton>
                    <Save className="size-3.5" /> Speichern
                  </PrimaryButton>
                </>
              )}
              {ro && <Btn>Verlauf und Diff ansehen</Btn>}
            </div>
          </div>
          <div className="min-h-0 flex-1 overflow-auto p-4">
            <F id={['POL-001', 'POL-002', 'POL-003', 'POL-006', 'POL-009', 'POL-010', 'POL-011']}>
              <CodeView code={code} marks={marks} />
            </F>
            {ro && (
              <p className="mt-3 text-[12px] text-muted-foreground">
                Org- und Team-Policies verwaltet der Team-Server; Änderungen sind dort versioniert und auditiert (wer, wann, Diff). Lokal
                kannst du sie nur durch eigene, strengere Regeln ergänzen.
              </p>
            )}
          </div>
          {/* Probleme */}
          {!ro && (
            <F id={['POL-002', 'POL-003', 'POL-004']} className="shrink-0 border-t border-border bg-sunken/60">
              <div className="flex items-center gap-3 px-4 py-1.5 text-[11px] text-muted-foreground">
                <span className="font-medium text-foreground">Probleme</span>
                <span>{problems.length}</span>
                <span className="ml-auto">CEL: max. 4 KiB je Ausdruck · Kostenbudget 10 ms je Auswertung</span>
              </div>
              <div className="max-h-24 overflow-auto px-4 pb-2 font-mono text-[12px]">
                {problems.length === 0 ? (
                  <div className="font-sans text-[12px] text-muted-foreground">
                    Keine Probleme. Gespeicherte Änderungen lädt beton sofort; bei einem Fehler bleibt die letzte gültige Version aktiv.
                  </div>
                ) : (
                  problems.map((p) => (
                    <div key={p.line} className="flex gap-3">
                      <span className="text-deny">Fehler</span>
                      <span className="text-muted-foreground">Zeile {p.line}</span>
                      <span className="font-sans">{p.text}</span>
                    </div>
                  ))
                )}
              </div>
            </F>
          )}
        </div>

        {/* Hilfen */}
        <aside className="w-72 shrink-0 overflow-y-auto border-l border-border">
          <F id={['POL-011', 'POL-012', 'POL-013', 'POL-014', 'POL-015', 'POL-016', 'POL-017', 'POL-018', 'POL-019', 'POL-020']} className="p-3">
            <h3 className="type-wide mb-1.5 text-[13px] font-semibold">Regeltyp einfügen</h3>
            <div className="divide-y divide-border/70">
              {builtinTypes.map((t) => (
                <button key={t.type} className="flex w-full items-baseline gap-2 py-1.5 text-left hover:bg-accent/60" disabled={ro}>
                  <span className="w-36 shrink-0 font-mono text-[12px]">{t.type}</span>
                  <span className="text-[12px] text-muted-foreground">
                    {t.desc}
                    {t.milestone && <span className="ml-1 text-[10px]">· ab {t.milestone}</span>}
                  </span>
                </button>
              ))}
            </div>
          </F>
          <F id={['POL-004', 'POL-005']} className="border-t border-border p-3">
            <h3 className="type-wide text-[13px] font-semibold">Kontext in tool_call</h3>
            <p className="mb-1.5 text-[12px] text-muted-foreground">Nur lesend. Tool-Calls sind über alle Harnesses normalisiert.</p>
            <div className="space-y-1">
              {contextVars.map((v) => (
                <div key={v.name} className="text-[12px]">
                  <span className="font-mono">{v.name}</span> <span className="text-[11px] text-muted-foreground">{v.type}</span>
                  {v.note && <div className="text-[11px] leading-snug text-muted-foreground">{v.note}</div>}
                </div>
              ))}
            </div>
            <p className="mt-2 text-[11px] text-muted-foreground">
              Funktionen: glob(), under(), shell.has_command(), shell.has_flag(), url.host(), domain_matches(), bytes_human()
            </p>
          </F>
        </aside>
      </div>
    </Shell>
  )
}
