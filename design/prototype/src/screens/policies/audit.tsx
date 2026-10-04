import { Download } from 'lucide-react'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, CodeView, PageHead, Scroll, Shell, Tag, Verdict, type VerdictKind } from '@/app/kit/policies'

type Row = { seq: number; t: string; phase: string; subject: string; v: VerdictKind; rules: string; enf: string; us: number; dry?: boolean }

const rows: Row[] = [
  { seq: 1842, t: '14:21:07', phase: 'tool_call', subject: 'Shell · git push origin beton/rate-limiter-7f3k', v: 'ask', rules: 'confirm-destructive-shell, git-guard', enf: 'full', us: 412 },
  { seq: 1838, t: '14:20:51', phase: 'tool_call', subject: 'Shell · pnpm vitest run auth', v: 'allow', rules: '–', enf: 'full', us: 188 },
  { seq: 1835, t: '14:20:44', phase: 'tool_result', subject: 'Shell · pnpm vitest run auth', v: 'modify', rules: 'pii', enf: 'full', us: 960 },
  { seq: 1831, t: '14:20:40', phase: 'tool_call', subject: 'Bearbeiten · src/routes/auth.ts', v: 'allow', rules: '–', enf: 'full', us: 151 },
  { seq: 1828, t: '14:20:39', phase: 'tool_call', subject: 'Shell · pnpm publish --dry-run', v: 'allow', rules: 'no-npm-publish (Shadow)', enf: 'full', us: 233, dry: true },
  { seq: 1820, t: '14:20:12', phase: 'tool_call', subject: 'Lesen · .env.local', v: 'deny', rules: 'paths', enf: 'full', us: 205 },
  { seq: 1811, t: '14:19:58', phase: 'model_request', subject: 'Turn 7 · claude-opus-5-5', v: 'allow', rules: '–', enf: 'full', us: 97 },
]

const json = `{
  "type": "policy.decision",
  "decision_id": "pd_01JB7Q4M2X…",
  "phase": "tool_call",
  "subject_ref": { "seq": 1841, "tool_call_id": "tc_9fk2…" },
  "subject_hash": "sha256:0b6e…41",
  "outcome": "ask",
  "approval_id": "apr_01JB7Q4N…",
  "matched": [
    { "rule_id": "confirm-destructive-shell", "scope": "project", "set": "git-and-budget",
      "set_version": "sha256:9c1e…7a", "verdict": "ask", "dry_run": false },
    { "rule_id": "git-guard", "scope": "user", "set": "defaults",
      "set_version": "sha256:1f9b…3c", "verdict": "ask", "dry_run": false }
  ],
  "defaults_applied": [ { "scope": "project", "phase": "tool_call", "default": "allow" } ],
  "conflicts": [], "errors": [], "grants_used": [],
  "enforcement": "full",
  "policy_set_hash": "sha256:4be1…c0",
  "eval_us": 412
}`

export function PolicyAudit({ state }: { state: string }) {
  const onlyDeny = state === 'filtered'
  const list = onlyDeny ? rows.filter((r) => r.v === 'deny' || r.v === 'ask' || r.dry) : rows
  return (
    <Shell nav="policies">
      <PageHead
        title="Entscheidungen"
        sub="Jede Auswertung erzeugt genau ein policy.decision-Ereignis im Session-Log – auch wenn nichts gegriffen hat."
        actions={
          <Btn>
            <Download className="size-3.5" /> Mit Session exportieren
          </Btn>
        }
      >
        <div className="mt-2 flex items-center gap-1.5 text-[12px]">
          <span className="text-muted-foreground">Session:</span>
          <span className="font-medium">Rate-Limiter für die Login-API</span>
          <span className="ml-3 text-muted-foreground">Zeigen:</span>
          {['Alle', 'Abgelehnt, gefragt, Shadow', 'Geändert'].map((f, i) => (
            <span key={f} className={cn('rounded-md border px-2 py-0.5', (onlyDeny ? i === 1 : i === 0) ? 'border-foreground bg-foreground text-background' : 'border-border')}>
              {f}
            </span>
          ))}
        </div>
      </PageHead>
      <div className="flex min-h-0 flex-1">
        <Scroll>
          <F id="POL-025" className="px-6 py-3">
            <div className="grid grid-cols-[52px_64px_104px_minmax(0,1fr)_180px_minmax(0,200px)_44px_56px] gap-3 border-b border-border pb-1 text-[11px] text-muted-foreground">
              <span>seq</span>
              <span>Zeit</span>
              <span>Phase</span>
              <span>Bezug (aus dem verknüpften Ereignis)</span>
              <span>Ergebnis</span>
              <span>Regeln</span>
              <span>Durchs.</span>
              <span className="text-right">µs</span>
            </div>
            {list.map((r) => (
              <div
                key={r.seq}
                className={cn(
                  'grid grid-cols-[52px_64px_104px_minmax(0,1fr)_180px_minmax(0,200px)_44px_56px] items-center gap-3 border-b border-border/60 py-1.5 text-[12px]',
                  r.seq === 1842 && 'bg-accent',
                )}
              >
                <span className="font-mono text-muted-foreground">{r.seq}</span>
                <span className="text-muted-foreground tabular-nums">{r.t}</span>
                <span className="font-mono text-[11px]">{r.phase}</span>
                <span className="truncate">{r.subject}</span>
                <span className="flex items-center gap-1">
                  <Verdict v={r.v} label={r.v === 'modify' ? 'geändert' : undefined} />
                  {r.dry && <Tag mono={false}>würde ablehnen</Tag>}
                </span>
                <span className="truncate font-mono text-[11px] text-muted-foreground">{r.rules}</span>
                <span className="text-[11px] text-muted-foreground">{r.enf}</span>
                <span className="text-right text-[11px] text-muted-foreground tabular-nums">{r.us}</span>
              </div>
            ))}
          </F>
        </Scroll>
        <aside className="w-[440px] shrink-0 overflow-y-auto border-l border-border p-4">
          <F id="POL-025">
            <h3 className="text-[13px] font-semibold">seq 1842 · fragt nach</h3>
            <p className="mb-2 text-[12px] text-muted-foreground">
              Keine Tool-Argumente im Klartext: das Ereignis verweist per <span className="font-mono">subject_ref</span> auf seq 1841 und trägt nur
              einen Hash.
            </p>
            <CodeView code={json} lang="json" />
            <p className="mt-2 text-[12px] text-muted-foreground">Auch als Span-Ereignis in OpenTelemetry-Traces, wenn du den Export einschaltest.</p>
          </F>
        </aside>
      </div>
    </Shell>
  )
}
