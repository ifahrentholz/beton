import { Check, Play, X } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, CodeView, PageHead, PrimaryButton, Scroll, Shell, Tag, Verdict } from './kit'

type Case = { name: string; status: 'pass' | 'fail' | 'run' | 'queued'; ms?: number }

const files: { file: string; cases: Case[] }[] = [
  {
    file: 'tests/git.policytest.yaml',
    cases: [
      { name: 'Force-Push wird geblockt', status: 'pass', ms: 3 },
      { name: 'git push origin +main wird geblockt', status: 'pass', ms: 2 },
      { name: 'git -C x push -f wird geblockt', status: 'pass', ms: 2 },
      { name: 'Push auf Feature-Branch fragt nach', status: 'pass', ms: 2 },
      { name: 'Unklares Shell-Kommando mit git fragt nach', status: 'pass', ms: 4 },
    ],
  },
  {
    file: 'tests/budget.policytest.yaml',
    cases: [
      { name: 'Opus wird bei hohen Kosten umgeroutet', status: 'pass', ms: 3 },
      { name: 'spend_cap fragt bei 10 USD genau einmal', status: 'pass', ms: 5 },
      { name: 'Tagesbudget um 00:00 Europe/Berlin zurückgesetzt', status: 'pass', ms: 6 },
    ],
  },
  {
    file: 'tests/loop.policytest.yaml',
    cases: [
      { name: 'Dritter identischer Call fragt nach', status: 'pass', ms: 4 },
      { name: 'Fünf Fehler in Folge melden', status: 'pass', ms: 4 },
    ],
  },
  {
    file: 'tests/browser.policytest.yaml',
    cases: [
      { name: 'Projekt-Default deny + Agent-allow → deny', status: 'pass', ms: 2 },
      { name: 'Zusätzliches Projekt-allow für github.com → allow', status: 'pass', ms: 2 },
    ],
  },
]

const testYaml = `cases:
  - name: Push auf Feature-Branch fragt nach
    phase: tool_call
    input: { tool: { name: exec_command, kind: shell, args: { command: "git push origin feature/x" } } }
    expect: { outcome: ask, rules: [git-guard] }
`

export function PolicyTests({ state }: { state: string }) {
  const failing = state === 'failing'
  const running = state === 'running'
  const data = files.map((f, fi) => ({
    ...f,
    cases: f.cases.map((c, ci): Case => {
      if (failing && fi === 0 && ci === 3) return { ...c, status: 'fail' }
      if (running && (fi > 1 || (fi === 1 && ci > 0))) return { ...c, status: fi === 1 && ci === 1 ? 'run' : 'queued', ms: undefined }
      return c
    }),
  }))
  const all = data.flatMap((f) => f.cases)
  const passed = all.filter((c) => c.status === 'pass').length
  const failed = all.filter((c) => c.status === 'fail').length

  return (
    <Shell nav="policies">
      <PageHead
        title="Policy-Tests"
        sub=".beton/policies/tests/ · deklarativ in YAML, laufen lokal und in CI"
        actions={
          <PrimaryButton>
            <Play className="size-3.5" /> Alle Tests ausführen
          </PrimaryButton>
        }
      />
      <Scroll>
        <F id="POL-026" className="px-6 py-4">
          <div className="mb-3 flex items-center gap-4 text-[13px]">
            {running ? (
              <span className="inline-flex items-center gap-2">
                <span className="size-2 animate-pulse rounded-full bg-ok" /> Läuft … {passed} von {all.length}
              </span>
            ) : failed ? (
              <span className="inline-flex items-center gap-2 font-semibold text-deny">
                <span className="size-2 rotate-45 bg-deny" /> {failed} fehlgeschlagen, {passed} bestanden
              </span>
            ) : (
              <span className="inline-flex items-center gap-2 font-semibold">
                <Check className="size-4 text-ok" /> Alle {passed} Tests bestanden
              </span>
            )}
            <span className="text-muted-foreground">Uhr simuliert ab 2026-01-01 12:00 UTC · 4 Dateien · 41 ms</span>
            <span className="ml-auto font-mono text-[11px] text-muted-foreground">beton policy test .beton/policies/tests/ --junit out.xml</span>
          </div>
          <div className="overflow-hidden rounded-md border border-border bg-card">
            {data.map((f) => (
              <div key={f.file} className="border-b border-border last:border-b-0">
                <div className="flex items-center gap-2 bg-sunken/60 px-3 py-1.5 text-[12px]">
                  <span className="font-mono">{f.file}</span>
                  <span className="ml-auto text-muted-foreground">
                    {f.cases.filter((c) => c.status === 'pass').length}/{f.cases.length}
                  </span>
                </div>
                {f.cases.map((c) => (
                  <div key={c.name}>
                    <div className={cn('flex items-center gap-2 px-3 py-1 text-[13px]', c.status === 'fail' && 'bg-deny-soft')}>
                      {c.status === 'pass' && <Check className="size-3.5 text-ok" aria-label="bestanden" />}
                      {c.status === 'fail' && <X className="size-3.5 text-deny" aria-label="fehlgeschlagen" />}
                      {c.status === 'run' && <span className="mx-[3px] size-2 animate-pulse rounded-full bg-ok" aria-label="läuft" />}
                      {c.status === 'queued' && <span className="mx-[3px] size-2 rounded-full border border-muted-foreground" aria-label="wartet" />}
                      <span className={cn(c.status === 'queued' && 'text-muted-foreground')}>{c.name}</span>
                      {c.status === 'fail' && <span className="text-[12px] text-deny">fehlgeschlagen</span>}
                      <span className="ml-auto text-[11px] text-muted-foreground tabular-nums">{c.ms ? `${c.ms} ms` : ''}</span>
                    </div>
                    {c.status === 'fail' && (
                      <div className="grid grid-cols-2 gap-4 border-t border-deny/30 px-3 py-3 pl-8 text-[12px]">
                        <div className="space-y-2">
                          <div className="grid grid-cols-[90px_1fr] gap-y-1">
                            <span className="text-muted-foreground">Erwartet</span>
                            <span>
                              <Verdict v="ask" /> durch <span className="font-mono">git-guard</span>
                            </span>
                            <span className="text-muted-foreground">Tatsächlich</span>
                            <span>
                              <Verdict v="deny" /> durch <span className="font-mono">paths</span> (Projekt)
                            </span>
                          </div>
                          <CodeView code={testYaml} file="tests/git.policytest.yaml · Zeile 31" start={31} marks={[{ line: 35, tone: 'focus' }]} />
                        </div>
                        <div>
                          <div className="mb-1 font-medium">Explain-Trace</div>
                          <div className="space-y-1 rounded-md border border-border bg-background p-2 font-mono text-[11px]">
                            <div>pass2 user/defaults/git-guard → ask (push)</div>
                            <div>pass2 project/git-and-budget/confirm-destructive-shell → ask</div>
                            <div className="text-deny">pass2 project/git-and-budget/paths → deny</div>
                            <div className="pl-3 text-muted-foreground">under("/repo/.git/config", worktree) → false</div>
                            <div className="pl-3 text-muted-foreground">tool.paths enthält .git/config (aus push-Refspec-Auflösung)</div>
                            <div>result → deny</div>
                          </div>
                          <p className="mt-2 text-muted-foreground">
                            Die neue Regel <span className="font-mono">paths</span> erkennt beim Push fälschlich <span className="font-mono">.git/config</span> als
                            Schreibziel. Regel anpassen oder Testerwartung ändern.
                          </p>
                          <div className="mt-2 flex gap-2">
                            <Btn>Regel öffnen</Btn>
                            <Btn tone="quiet">Erwartung übernehmen</Btn>
                          </div>
                        </div>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            ))}
          </div>
          <div className="mt-3 flex items-center gap-2 text-[12px] text-muted-foreground">
            <Tag mono={false}>CI</Tag> Läuft auch in GitHub Actions und erzeugt JUnit-XML. Jeder eingebaute Regeltyp hat eine mitgelieferte Testdatei.
          </div>
        </F>
      </Scroll>
    </Shell>
  )
}

const shadowRows = [
  { rule: 'no-npm-publish', set: 'shadow-npm (Projekt)', days: [0, 1, 0, 3, 2, 0, 1], last: 'pnpm publish --dry-run · ses_6p9z · heute 11:20' },
  { rule: 'no-curl-pipe-sh', set: 'shadow-net (User)', days: [4, 2, 6, 1, 0, 2, 5], last: 'curl -fsSL https://sh.rustup.rs | sh · ses_6h2f · gestern' },
]

export function PolicyShadow({ state }: { state: string }) {
  const empty = state === 'empty'
  const max = 6
  return (
    <Shell nav="policies">
      <PageHead title="Shadow-Modus" sub="Neue Regeln gefahrlos ausprobieren: Sie werden ausgewertet und protokolliert, blockieren aber nichts." />
      <Scroll>
        <F id={['POL-027', 'POL-025']} className="px-6 py-4">
          {empty ? (
            <div className="concrete-grain flex flex-col items-center gap-2 rounded-md border border-dashed border-border px-6 py-12 text-center">
              <p className="type-wide text-[15px] font-semibold">Keine Regel im Shadow-Modus</p>
              <p className="max-w-md text-[13px] text-muted-foreground">
                Setze in einer Policy <span className="font-mono">mode: dry_run</span> oder an einer Regel <span className="font-mono">dry_run: true</span>. Treffer
                erscheinen dann hier als „würde ablehnen“, ohne eine Session zu stören.
              </p>
            </div>
          ) : (
            <>
              <div className="overflow-hidden rounded-md border border-border bg-card">
                <div className="grid grid-cols-[200px_160px_repeat(7,44px)_minmax(0,1fr)_150px] items-end gap-2 border-b border-border bg-sunken px-3 py-1.5 text-[11px] text-muted-foreground">
                  <span>Regel</span>
                  <span>Set</span>
                  {['Fr', 'Sa', 'So', 'Mo', 'Di', 'Mi', 'heute'].map((d) => (
                    <span key={d} className="text-center">
                      {d}
                    </span>
                  ))}
                  <span>Letzter Treffer</span>
                  <span />
                </div>
                {shadowRows.map((r) => (
                  <div key={r.rule} className="grid grid-cols-[200px_160px_repeat(7,44px)_minmax(0,1fr)_150px] items-center gap-2 border-b border-border px-3 py-2 text-[12px] last:border-b-0">
                    <span className="flex items-center gap-2 font-mono">
                      <Verdict v="would_deny" label="" />
                      {r.rule}
                    </span>
                    <span className="text-muted-foreground">{r.set}</span>
                    {r.days.map((n, i) => (
                      <span key={i} className="flex h-7 flex-col items-center justify-end gap-0.5" title={`${n} Treffer`}>
                        <span className="w-4 bg-foreground/60" style={{ height: `${(n / max) * 18}px` }} />
                        <span className="text-[10px] text-muted-foreground tabular-nums">{n}</span>
                      </span>
                    ))}
                    <span className="truncate font-mono text-[11px] text-muted-foreground">{r.last}</span>
                    <span className="flex justify-end gap-1.5">
                      <Btn>Scharf schalten</Btn>
                    </span>
                  </div>
                ))}
              </div>
              <p className="mt-3 text-[12px] text-muted-foreground">
                Shadow-Treffer stehen als <span className="font-mono">dry_run: true</span> im <span className="font-mono">policy.decision</span>-Ereignis und
                beeinflussen das Ergebnis nicht. „Scharf schalten“ ändert <span className="font-mono">mode</span> in der Datei auf
                <span className="font-mono"> enforce</span> – nach deiner Bestätigung.
              </p>
            </>
          )}
        </F>
      </Scroll>
    </Shell>
  )
}
