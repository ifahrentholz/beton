import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, C, Premise, Section, SettingsShell } from '../hosts/settings-shell'

/** API-Referenz des laufenden Servers (lokal oder zentral) und SDK-Beispiele. */

type Ep = { m: string; p: string }
const resources: { name: string; eps: Ep[]; milestone: string; feature?: string }[] = [
  { name: 'System', milestone: 'M0', eps: [{ m: 'GET', p: '/v1/info' }, { m: 'GET', p: '/healthz' }, { m: 'GET', p: '/metrics' }, { m: 'GET', p: '/v1/openapi.json' }] },
  {
    name: 'Sessions',
    milestone: 'M0',
    eps: [
      { m: 'GET POST', p: '/v1/sessions' },
      { m: 'GET PATCH DELETE', p: '/v1/sessions/{id}' },
      { m: 'POST', p: '…/archive · unarchive · interrupt · fork · compact · resume' },
      { m: 'GET', p: '…/events?after_seq=' },
      { m: 'GET', p: '…/events/stream (SSE)' },
      { m: 'POST', p: '…/input' },
      { m: 'GET POST PATCH DELETE', p: '…/queue' },
      { m: 'GET', p: '…/export' },
    ],
  },
  { name: 'Workspace', milestone: 'M1', eps: [{ m: 'GET', p: '…/workspace/tree · search · changes · diff' }, { m: 'GET PUT', p: '…/workspace/files/{path}' }, { m: 'GET POST DELETE', p: '…/terminals' }] },
  { name: 'Approvals', milestone: 'M2', eps: [{ m: 'GET', p: '…/approvals' }, { m: 'POST', p: '…/approvals/{aid}/resolve' }] },
  { name: 'Collaboration', milestone: 'M4', eps: [{ m: '·', p: '…/shares · comments · side-chats' }, { m: 'GET', p: '/v1/inbox' }] },
  { name: 'Git', milestone: 'M3', eps: [{ m: '·', p: '…/change-requests' }, { m: 'GET POST', p: '/v1/git/connections' }] },
  { name: 'Browser', milestone: 'M3', eps: [{ m: '·', p: '…/browser/tabs' }, { m: 'POST', p: '…/browser/mode · pick' }] },
  { name: 'Projects', milestone: 'M0', eps: [{ m: 'GET POST PATCH DELETE', p: '/v1/projects' }] },
  { name: 'Imports', milestone: 'M1', eps: [{ m: 'GET', p: '/v1/imports/candidates' }, { m: 'POST', p: '/v1/imports' }] },
  { name: 'Agents & Harnesses', milestone: 'M1', eps: [{ m: 'GET', p: '/v1/agents' }, { m: 'GET', p: '/v1/harnesses' }, { m: 'GET', p: '/v1/models' }] },
  { name: 'Policies & Usage', milestone: 'M2', eps: [{ m: '·', p: '/v1/policies' }, { m: 'POST', p: '/v1/policies/evaluate' }, { m: 'GET', p: '/v1/usage' }] },
  {
    name: 'Hosts & Runner',
    milestone: 'M4',
    feature: 'RUN-019',
    eps: [{ m: 'GET', p: '/v1/hosts' }, { m: 'GET', p: '/v1/runners' }, { m: 'POST', p: '/v1/runners/{id}/stop' }, { m: 'POST', p: '/v1/runners/{id}/snapshot' }],
  },
  { name: 'Identität', milestone: 'M4', eps: [{ m: 'GET', p: '/v1/me' }, { m: '·', p: '/v1/tokens · service-accounts · devices' }] },
  { name: 'Schedules & Webhooks', milestone: 'M5', eps: [{ m: '·', p: '/v1/schedules' }, { m: 'POST', p: '/v1/triggers/{id}/fire' }] },
  { name: 'Push', milestone: 'M3', eps: [{ m: 'POST DELETE', p: '/v1/push/subscriptions' }] },
  { name: 'WebSocket', milestone: 'M0', eps: [{ m: 'GET', p: '/v1/ws (Upgrade)' }] },
]

function Methods({ m }: { m: string }) {
  if (m === '·') return <span className="text-muted-foreground">·</span>
  return (
    <span className="inline-flex gap-1">
      {m.split(' ').map((x) => (
        <span key={x} className="rounded-sm border border-border px-1 font-mono text-[10px] leading-4 font-semibold">
          {x}
        </span>
      ))}
    </span>
  )
}

/** Anfang eines Kommentars (`//` außerhalb von Strings oder `# ` am Zeilenanfang), sonst -1. */
function commentStart(line: string) {
  if (/^\s*# /.test(line)) return line.indexOf('#')
  let quote: string | null = null
  for (let i = 0; i < line.length; i++) {
    const ch = line[i]
    if (quote) {
      if (ch === quote) quote = null
    } else if (ch === '"' || ch === "'") quote = ch
    else if (ch === '/' && line[i + 1] === '/') return i
  }
  return -1
}

/** Sehr einfaches Highlighting: Kommentare gedämpft, Strings grün. */
function Code({ title, code }: { title: string; code: string }) {
  return (
    <div className="overflow-hidden rounded-md border border-border bg-card">
      <div className="flex items-center border-b border-border bg-sunken px-3 py-1 text-[11px] text-muted-foreground">
        <span className="font-mono">{title}</span>
        <button className="ml-auto hover:text-foreground">Kopieren</button>
      </div>
      <pre className="overflow-x-auto p-3 font-mono text-[12px] leading-[1.6]">
        {code.split('\n').map((line, i) => {
          const cut = commentStart(line)
          const body = cut >= 0 ? line.slice(0, cut) : line
          const comment = cut >= 0 ? line.slice(cut) : ''
          const parts = body.split(/('[^']*'|"[^"]*")/g)
          return (
            <div key={i} className="min-h-[1.6em]">
              {parts.map((p, j) => (
                <span key={j} className={cn(/^['"]/.test(p) && 'text-ok')}>
                  {p}
                </span>
              ))}
              {comment && <span className="text-muted-foreground">{comment}</span>}
            </div>
          )
        })}
      </pre>
    </div>
  )
}

const curl = `# Lokal: Token aus der Datei, die beton serve anlegt (0600)
curl -s -H "Authorization: Bearer $(cat ~/.beton/token)" \\
  "http://127.0.0.1:7420/v1/sessions?limit=2"`

const response = `{
  "items": [
    { "id": "ses_7f3k", "title": "Rate-Limiter für die Login-API",
      "harness": "claude", "status": "waiting", "last_seq": 412 },
    { "id": "ses_7f3m", "title": "Review: Rate-Limiter",
      "harness": "codex", "status": "running", "last_seq": 142 }
  ],
  "next_cursor": "c_6q2a"
}`

const problem = `HTTP/1.1 409 Conflict
Content-Type: application/problem+json

{
  "type": "about:blank",
  "title": "Harness unterstützt das nicht",
  "status": 409,
  "code": "capability_unsupported",
  "detail": "Gemini CLI kann keinen Fork bei seq 120 anlegen."
}`

const ts = `import { BetonClient } from '@ifahrentholz/beton-sdk'

const c = new BetonClient({
  baseUrl: 'http://127.0.0.1:7420',
  token: process.env.BETON_TOKEN,
})
const s = await c.sessions.create({ target: 'claude', cwd: '.' })
await s.send('Füge der Login-Route einen Rate-Limiter hinzu')

// WS mit Resume ab seq – Abbrüche bleiben unsichtbar
for await (const ev of s.events({ fromSeq: 0 })) {
  if (ev.type === 'message.delta') process.stdout.write(ev.text)
}

await s.interrupt()
await s.fork({ atSeq: 120, harness: 'codex' })`

const rust = `use beton_sdk::{Client, SessionCreate};
use futures::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let c = Client::local()?; // liest ~/.beton/token
    let s = c.sessions()
        .create(SessionCreate::target("fake"))
        .await?;
    s.send("sag hallo").await?;

    let mut events = s.events(0).await?;
    while let Some(ev) = events.next().await {
        if let Some(text) = ev?.as_message_delta() {
            print!("{text}");
        }
    }
    Ok(())
}`

export function ApiReference({ state }: { state: string }) {
  return (
    <SettingsShell
      active="api"
      title="API & SDKs"
      description={
        <>
          Der lokale Server bietet dieselbe REST-API wie ein zentraler. Alles, was die App kann, geht auch per API – beschrieben als OpenAPI 3.1 unter{' '}
          <C>/v1/openapi.json</C>.
        </>
      }
      actions={
        <>
          <Btn>openapi.json öffnen</Btn>
          <Btn variant="primary">Zugangstoken erstellen</Btn>
        </>
      }
    >
      <div className="mb-4 flex flex-wrap gap-x-6 gap-y-1">
        <Premise kind="local">
          Läuft auf <C>127.0.0.1:7420</C>; Zugriff mit der Token-Datei <C>~/.beton/token</C>, kein Login.
        </Premise>
        <Premise kind="local">Ein beton-Token ist kein Modell-API-Schlüssel – die Harnesses nutzen weiter die Anmeldung ihrer CLIs.</Premise>
      </div>

      <div className="mb-4 flex gap-1 border-b border-border text-[13px]">
        {[
          ['reference', 'Endpunkte'],
          ['sdk', 'SDKs'],
        ].map(([id, label]) => (
          <span
            key={id}
            className={cn('-mb-px border-b-2 px-2 pb-2', id === state ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground')}
          >
            {label}
          </span>
        ))}
      </div>

      {state === 'reference' ? (
        <div className="grid grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] gap-8">
          <F id={['API-001', 'API-002']}>
            <table className="w-full text-[12.5px]">
              <thead>
                <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
                  <th className="py-1.5 pr-3 font-normal">Ressource</th>
                  <th className="py-1.5 pr-3 font-normal">Endpunkte</th>
                  <th className="py-1.5 font-normal">ab</th>
                </tr>
              </thead>
              <tbody>
                {resources.map((r) => {
                  const row = (
                    <tr key={r.name} className="border-b border-border align-top">
                      <td className="py-1.5 pr-3 font-medium whitespace-nowrap">{r.name}</td>
                      <td className="py-1.5 pr-3">
                        {r.eps.map((e) => (
                          <div key={e.p} className="flex items-center gap-2 py-px">
                            <span className="w-28 shrink-0 text-right">
                              <Methods m={e.m} />
                            </span>
                            <code className="font-mono text-[12px]">{e.p}</code>
                          </div>
                        ))}
                      </td>
                      <td className="py-1.5 font-mono text-[11px] text-muted-foreground">{r.milestone}</td>
                    </tr>
                  )
                  return row
                })}
              </tbody>
            </table>
            <p className="mt-2 text-[12px] text-muted-foreground">
              Konventionen: Präfix <C>/v1</C>, JSON in snake_case, Cursor-Pagination <C>?cursor=&amp;limit=</C>, Fehler als{' '}
              <C>application/problem+json</C>, <C>Idempotency-Key</C> für anlegende POSTs.
            </p>
          </F>
          <div className="flex flex-col gap-4">
            <F id="API-002">
              <h3 className="mb-1.5 text-[13px] font-semibold">Beispiel: letzte Sessions</h3>
              <Code title="Terminal" code={curl} />
              <div className="mt-2">
                <Code title="200 OK · application/json" code={response} />
              </div>
            </F>
            <F id="API-001">
              <h3 className="mb-1.5 text-[13px] font-semibold">Fehler sind maschinenlesbar</h3>
              <Code title="Antwort" code={problem} />
            </F>
            <F id="RUN-019">
              <h3 className="mb-1.5 text-[13px] font-semibold">Runner stoppen</h3>
              <Code
                title="Terminal · zentraler Server mit persönlichem Token"
                code={`curl -X POST -H "Authorization: Bearer $BETON_PAT" \\\n  "https://beton.team.example/v1/runners/run_2b7x/stop"\n# 202 Accepted – Session wechselt nach dem Drain auf stopped`}
              />
            </F>
          </div>
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-8">
          <F id="API-004">
            <Section title="TypeScript" aside={<C>pnpm add @ifahrentholz/beton-sdk</C>}>
              <p className="mb-2 text-[12.5px] text-muted-foreground">
                Typen aus den Rust-Typen generiert, REST-Client aus OpenAPI, WS-Client mit Resume ab <C>seq</C>. Läuft in Node ≥ 20 und im Browser. Die Web-UI
                nutzt nur dieses SDK.
              </p>
              <Code title="stream.ts" code={ts} />
            </Section>
          </F>
          <F id="API-005">
            <Section title="Rust" aside={<C>cargo add beton-sdk</C>}>
              <p className="mb-2 text-[12.5px] text-muted-foreground">
                Async mit tokio, typisiert über <C>beton-proto</C>, dieselben Fähigkeiten wie das TS-SDK. CLI und TUI sprechen den Server nur hierüber an.
              </p>
              <Code title="examples/stream.rs" code={rust} />
            </Section>
          </F>
        </div>
      )}
    </SettingsShell>
  )
}
