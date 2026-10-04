import { Copy, Plus } from 'lucide-react'
import { HarnessBadge } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Code, CodeBlock, Mark, Note, Pill, SectionTitle } from '@/app/kit/harnesses'
import { AutomationShell } from './parts'

const TRIGGERS = [
  { id: 'ci-failed', agent: 'pr-fixer', calls: '7 heute', on: true },
  { id: 'pr-review', agent: 'duetto', calls: '2 diese Woche', on: true },
  { id: 'nightly-e2e', agent: 'a11y-auditor', calls: 'noch nie', on: false },
]

const CALLS: { time: string; code: number; text: string; run?: string }[] = [
  { time: '07:38:04', code: 202, text: 'Lauf gestartet', run: 'run_99y' },
  { time: '07:38:09', code: 202, text: 'Gleicher Idempotency-Key – kein zweiter Lauf', run: 'run_99y' },
  { time: '07:40:51', code: 202, text: 'Eingereiht (pr-fixer: 2 von 2 aktiv)', run: 'run_99z' },
  { time: '06:12:30', code: 403, text: 'Token ohne Scope webhooks:trigger (PAT „ci-readonly“)' },
  { time: '06:02:11', code: 413, text: 'Payload 412 KiB, erlaubt sind 256 KiB' },
  { time: 'gestern 23:59', code: 429, text: '61. Aufruf in einer Stunde, Grenze 60/Std.' },
]

export function WebhookScreen({ state }: { state: string }) {
  const created = state === 'token-created'
  const calls = state === 'calls'
  return (
    <AutomationShell
      tab="triggers"
      actions={
        <Btn variant="primary">
          <Plus className="size-3.5" /> Trigger anlegen
        </Btn>
      }
    >
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Trigger" className="w-56 shrink-0 border-r border-border py-2">
          {TRIGGERS.map((t) => (
            <div key={t.id} className={cn('border-l-2 px-4 py-2 text-[13px]', t.id === 'ci-failed' ? 'border-foreground bg-accent' : 'border-transparent')}>
              <div className="flex items-center gap-2 font-medium">
                {t.id}
                {!t.on && <Pill>aus</Pill>}
              </div>
              <div className="text-[11.5px] text-muted-foreground">
                {t.agent} · {t.calls}
              </div>
            </div>
          ))}
        </nav>
        <div className="min-w-0 flex-1 overflow-y-auto px-6 py-4">
          <div className="max-w-3xl space-y-6">
            <div className="flex items-center gap-3">
              <h3 className="type-wide text-[17px] font-[700]">ci-failed</h3>
              <Mark kind="ok" label="aktiv" />
              <span className="text-[12px] text-muted-foreground">startet</span>
              <HarnessBadge id="claude" model="claude-sonnet-5-5" />
              <span className="text-[12px] text-muted-foreground">Agent pr-fixer</span>
            </div>

            <F id="ASY-007" as="section">
              <SectionTitle>Endpunkt</SectionTitle>
              <div className="flex items-center gap-2 rounded-md border border-border bg-card px-3 py-2 font-mono text-[12px]">
                <span className="font-semibold">POST</span>
                <span className="min-w-0 flex-1 truncate">http://127.0.0.1:7420/v1/triggers/trg_ci_failed/fire</span>
                <button aria-label="Adresse kopieren" className="text-muted-foreground hover:text-foreground">
                  <Copy className="size-3.5" />
                </button>
              </div>
              <p className="mt-1.5 text-[12px] text-muted-foreground">
                Nur von diesem Rechner erreichbar. Damit eine CI von außen auslösen kann, brauchst du einen Team-Server oder ein freigegebenes Netz (z. B.
                Tailscale). beton öffnet dafür keine Ports von selbst.
              </p>
            </F>

            <F id="ASY-007" as="section">
              <SectionTitle aside="alternativ: persönliches Token mit Scope webhooks:trigger">Token</SectionTitle>
              {created ? (
                <div className="chamfer border-l-4 border-signal bg-signal-soft p-3 text-[13px]">
                  <p className="font-semibold">Kopiere das neue Token jetzt – es wird nicht wieder angezeigt</p>
                  <div className="mt-2 flex items-center gap-2 rounded-sm bg-card px-2 py-1.5 font-mono text-[12px]">
                    <span className="flex-1">bt_trg_7Qm2vX9cL4eR8tYk1NpA6sJd3HwF0uGz</span>
                    <Btn variant="primary" size="sm" className="chamfer-sm rounded-none">
                      <Copy className="size-3" /> Kopieren
                    </Btn>
                  </div>
                  <p className="mt-1.5 text-muted-foreground">Das alte Token (…a91c) ist ab sofort ungültig. Hinterlege das neue als Secret in deiner CI.</p>
                </div>
              ) : (
                <div className="flex items-center gap-3 border-y border-border py-2 text-[13px]">
                  <span className="font-mono text-[12px]">bt_trg_…a91c</span>
                  <span className="text-muted-foreground">trigger-eigenes Token · erstellt 28.09. · zuletzt benutzt 07:40</span>
                  <Btn variant="outline" size="sm" className="ml-auto">
                    Neues Token erstellen
                  </Btn>
                </div>
              )}
            </F>

            <F id={['ASY-007', 'AGT-010', 'ASY-010']} as="section">
              <SectionTitle>Was beim Aufruf passiert</SectionTitle>
              <dl className="divide-y divide-border border-y border-border text-[13px]">
                <div className="grid grid-cols-[170px_1fr] gap-4 py-2">
                  <dt className="text-muted-foreground">Aufgabe</dt>
                  <dd className="font-mono text-[12px]">CI auf {'{{ trigger.payload.branch }}'} ist rot (PR {'{{ trigger.payload.pr }}'}). Bitte beheben.</dd>
                </div>
                <div className="grid grid-cols-[170px_1fr] gap-4 py-2">
                  <dt className="text-muted-foreground">Parameter aus Payload</dt>
                  <dd>
                    <Code>branch</Code> ← <Code>$.branch</Code>
                  </dd>
                </div>
                <div className="grid grid-cols-[170px_1fr] gap-4 py-2">
                  <dt className="text-muted-foreground">Grenzen pro Lauf</dt>
                  <dd>30 Min. · 80 Turns · bei API höchstens 2 $</dd>
                </div>
                <div className="grid grid-cols-[170px_1fr] gap-4 py-2">
                  <dt className="text-muted-foreground">Schutz</dt>
                  <dd>höchstens 60 Aufrufe pro Stunde · Payload bis 256 KiB · gleicher Idempotency-Key innerhalb 24 Std. = derselbe Lauf</dd>
                </div>
              </dl>
              <p className="mt-1.5 text-[12px] text-muted-foreground">Die Payload landet nur in der Aufgabe, nie in den Anweisungen des Agents.</p>
            </F>

            {calls ? (
              <F id="ASY-007" as="section">
                <SectionTitle aside="letzte 24 Std.">Aufrufe</SectionTitle>
                <ul className="divide-y divide-border border-y border-border text-[12.5px]">
                  {CALLS.map((c, i) => (
                    <li key={i} className="grid grid-cols-[110px_60px_1fr_auto] items-center gap-3 py-2">
                      <span className="text-muted-foreground tabular-nums">{c.time}</span>
                      <span className={cn('font-mono', c.code === 202 ? 'text-ok' : 'text-deny')}>
                        {c.code === 202 ? '✓' : '✕'} {c.code}
                      </span>
                      <span>{c.text}</span>
                      {c.run ? <span className="font-mono text-[11px] underline">{c.run}</span> : <span />}
                    </li>
                  ))}
                </ul>
              </F>
            ) : (
              <F id="ASY-007" as="section">
                <SectionTitle>Beispiel</SectionTitle>
                <CodeBlock
                  lines={[
                    'curl -X POST http://127.0.0.1:7420/v1/triggers/trg_ci_failed/fire \\',
                    '  -H "Authorization: Bearer $BETON_TRIGGER_TOKEN" \\',
                    '  -H "Idempotency-Key: $GITHUB_RUN_ID" \\',
                    '  -H "Content-Type: application/json" \\',
                    `  -d '{"payload": {"pr": 418, "branch": "fix/checkout-timeout"}}'`,
                    '',
                    '→ 202 { "run_id": "run_99y", "session_id": "ses_7r2c" }',
                  ]}
                />
              </F>
            )}

            {!calls && !created && (
              <Note>Vendor-Webhooks (z. B. GitHub-Events) nimmt beton nicht direkt an. Lass sie von deiner CI an diesen Endpunkt weiterreichen.</Note>
            )}
          </div>
        </div>
      </div>
    </AutomationShell>
  )
}
