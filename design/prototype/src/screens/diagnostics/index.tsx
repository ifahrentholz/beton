import { Fragment, type ReactNode } from 'react'
import { Check, ChevronRight, Copy, FileArchive, FolderOpen, RefreshCw, Search, Trash2 } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge } from '@/app/harness'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import type { ScreenGroup } from '@/proto/types'
import { cn } from '@/lib/utils'
import { Btn, NetNote, Overlay, PageHeader, ProblemBox, Row, SectionTitle, Segmented, SettingsShell } from '../settings/shell'

/* ───────────────────────── Bausteine ───────────────────────── */

type Check3 = 'ok' | 'warn' | 'fail' | 'info'

function Status({ s }: { s: Check3 }) {
  if (s === 'ok')
    return (
      <span className="inline-flex w-16 items-center gap-1 text-[12px] text-ok">
        <Check className="size-3.5" /> ok
      </span>
    )
  if (s === 'warn')
    return (
      <span className="inline-flex w-16 items-center gap-1.5 text-[12px] font-medium">
        <span className="size-2 rotate-45 border-[1.5px] border-foreground" /> Hinweis
      </span>
    )
  if (s === 'fail')
    return (
      <span className="inline-flex w-16 items-center gap-1.5 text-[12px] font-medium text-deny">
        <span className="size-2 rotate-45 bg-deny" /> Fehler
      </span>
    )
  return (
    <span className="inline-flex w-16 items-center gap-1.5 text-[12px] text-muted-foreground">
      <span className="size-2 rounded-full border border-muted-foreground" /> optional
    </span>
  )
}

function Mono({ children, className }: { children: ReactNode; className?: string }) {
  return <code className={cn('font-mono text-[12px]', className)}>{children}</code>
}

function Banner({ tone, title, children, actions }: { tone: 'neutral' | 'deny' | 'ok'; title: ReactNode; children?: ReactNode; actions?: ReactNode }) {
  return (
    <div
      className={cn(
        'mb-5 flex items-start gap-3 rounded-md border p-3 text-[13px]',
        tone === 'deny' && 'border-deny/40 bg-deny-soft',
        tone === 'ok' && 'border-ok/40 bg-ok-soft',
        tone === 'neutral' && 'border-border bg-card',
      )}
    >
      <span className="mt-1">
        {tone === 'ok' ? <Check className="size-3.5 text-ok" /> : tone === 'deny' ? <span className="block size-2 rotate-45 bg-deny" /> : <RefreshCw className="size-3.5 animate-spin" />}
      </span>
      <div className="min-w-0 flex-1">
        <div className="font-semibold">{title}</div>
        {children && <div className="mt-0.5">{children}</div>}
      </div>
      {actions && <div className="flex shrink-0 gap-2">{actions}</div>}
    </div>
  )
}

function KV({ rows }: { rows: [ReactNode, ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[200px_1fr] gap-x-4 text-[13px]">
      {rows.map(([k, v], i) => (
        <Fragment key={i}>
          <dt className="border-b border-border py-1.5 text-muted-foreground">{k}</dt>
          <dd className="border-b border-border py-1.5">{v}</dd>
        </Fragment>
      ))}
    </dl>
  )
}

/* ───────────────────────── Umgebung prüfen (OBS-005, UX-007) ───────────────────────── */

type DoctorCheck = { id: string; label: string; s: Check3; msg: string; hint?: string; f?: string }

function doctorChecks(state: string): { group: string; items: DoctorCheck[] }[] {
  const ok = state === 'ok'
  const fail = state === 'fail'
  return [
    {
      group: 'Grundlagen',
      items: [
        { id: 'version', label: 'Version & Update-Kanal', s: 'ok', msg: 'beton 0.9.2 · stable' },
        { id: 'daemon', label: 'Daemon', s: 'ok', msg: 'läuft seit 3 Std. · 127.0.0.1:7420' },
        { id: 'config', label: 'Konfiguration', s: 'ok', msg: '~/.beton/config.yaml gültig' },
        fail
          ? { id: 'perms', label: 'Rechte der Token-Datei', s: 'fail', msg: '~/.beton/token hat 0644 – andere Benutzer können sie lesen', hint: 'chmod 600 ~/.beton/token' }
          : { id: 'perms', label: 'Rechte von ~/.beton', s: 'ok', msg: '0700, Token-Datei 0600' },
        { id: 'db', label: 'Datenbank', s: 'ok', msg: 'quick_check ok · Schema 14' },
      ],
    },
    {
      group: 'Agent-CLIs',
      items: [
        { id: 'harness.claude', label: 'Claude Code', s: 'ok', msg: 'claude 2.4.1 · angemeldet (Claude Max)' },
        ok
          ? { id: 'harness.codex', label: 'Codex', s: 'ok', msg: 'codex 0.61.0 · angemeldet (ChatGPT Pro)' }
          : { id: 'harness.codex', label: 'Codex', s: 'warn', msg: 'codex 0.48.2 ist älter als unterstützt (0.55 bis 0.62)', hint: 'npm i -g @openai/codex@latest' },
        { id: 'harness.gemini', label: 'Gemini CLI (ACP)', s: 'ok', msg: 'gemini 0.19.2 · angemeldet' },
      ],
    },
    {
      group: 'Sandbox & Sicherheit',
      items: [
        { id: 'sandbox.macos', label: 'Sandbox', s: 'ok', msg: 'macOS sandbox-exec verfügbar' },
        { id: 'proxy.ca', label: 'Egress-Proxy', s: 'ok', msg: 'lokales Zertifikat gültig bis 2027-04-11' },
        { id: 'keychain', label: 'Schlüsselbund', s: 'ok', msg: 'Zugriff erlaubt · 4 Secrets' },
        { id: 'policies', label: 'Policies', s: 'ok', msg: '3 Policy-Sets geladen, keine Fehler' },
      ],
    },
    {
      group: 'Optionale Funktionen',
      items: [
        { id: 'browser.chrome', label: 'Eingebauter Browser', s: ok ? 'ok' : 'info', msg: ok ? 'Chrome for Testing 141 installiert' : 'Chrome for Testing nicht installiert – nur für den eingebauten Browser nötig', hint: ok ? undefined : 'beton browser install' },
        { id: 'voice.whisper', label: 'Spracheingabe', s: 'info', msg: 'kein Whisper-Modell geladen – Spracheingabe nicht eingerichtet' },
        ok
          ? { id: 'features', label: 'Feature-Flags', s: 'ok', msg: 'voice, browser aktiv', f: 'UX-007' }
          : { id: 'features', label: 'Feature-Flags', s: 'warn', msg: 'BETON_FEATURES enthält unbekanntes Flag „brwoser“ – ignoriert', hint: 'Meintest du „browser“?', f: 'UX-007' },
        { id: 'telemetry', label: 'Telemetrie', s: 'ok', msg: 'aus (nie zugestimmt)' },
      ],
    },
  ]
}

function DiagDoctor({ state }: { state: string }) {
  const groups = doctorChecks(state)
  const all = groups.flatMap((g) => g.items)
  const warns = all.filter((c) => c.s === 'warn').length
  const fails = all.filter((c) => c.s === 'fail').length
  const running = state === 'running'
  return (
    <SettingsShell section="doctor">
      <PageHeader
        title="Umgebung prüfen"
        actions={
          <>
            <Btn>
              <Copy className="size-3.5" /> Als JSON kopieren
            </Btn>
            <Btn disabled={running}>
              <RefreshCw className={cn('size-3.5', running && 'animate-spin')} /> Erneut prüfen
            </Btn>
          </>
        }
      >
        Dieselben Prüfungen wie <Mono>beton doctor</Mono>. Prüfen ändert nichts an deinem System.
      </PageHeader>
      <F id="OBS-005">
        <div className="mb-4 flex items-center gap-3 text-[13px]">
          {running ? (
            <span className="text-muted-foreground">Prüfe … 9 von 16</span>
          ) : fails ? (
            <span className="font-semibold text-deny">
              {fails} Fehler, {warns} {warns === 1 ? 'Hinweis' : 'Hinweise'} – beton funktioniert so nicht sicher.
            </span>
          ) : warns ? (
            <span className="font-semibold">{warns} Hinweise, keine Fehler – beton läuft.</span>
          ) : (
            <span className="font-semibold text-ok">Alles in Ordnung.</span>
          )}
          <span className="ml-auto text-[12px] text-muted-foreground">zuletzt geprüft 14:03 · im Terminal Exit-Code {fails ? 2 : warns ? 1 : 0}</span>
        </div>
        {groups.map((g, gi) => (
          <div key={g.group} className="mb-4">
            <div className="mb-1 text-[12px] font-medium text-muted-foreground">{g.group}</div>
            <div className="divide-y divide-border rounded-md border border-border">
              {g.items.map((c, ci) => {
                const pending = running && gi * 5 + ci >= 9
                const row = (
                  <div className={cn('grid grid-cols-[180px_72px_1fr] items-start gap-3 px-3 py-2 text-[13px]', c.s === 'fail' && !pending && 'bg-deny-soft/60')}>
                    <span>{c.label}</span>
                    {pending ? <span className="text-[12px] text-muted-foreground">…</span> : <Status s={c.s} />}
                    <span className={cn(pending && 'text-muted-foreground')}>
                      {pending ? 'wartet' : c.msg}
                      {!pending && c.hint && (
                        <span className="mt-0.5 flex items-center gap-2 text-[12px] text-muted-foreground">
                          Behebung: <Mono className="rounded-sm bg-muted px-1 text-foreground">{c.hint}</Mono>
                        </span>
                      )}
                    </span>
                  </div>
                )
                return c.f ? (
                  <F key={c.id} id={c.f}>
                    {row}
                  </F>
                ) : (
                  <Fragment key={c.id}>{row}</Fragment>
                )
              })}
            </div>
          </div>
        ))}
        <p className="text-[12px] text-muted-foreground">
          Für Support: <a className="underline underline-offset-2">Diagnose-Bundle erstellen</a> – enthält dieses Ergebnis, aber keine Secrets.
        </p>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Diagnose-Bundle (OBS-006, OBS-002) ───────────────────────── */

const BUNDLE = [
  ['doctor.json', 'Ergebnis von „Umgebung prüfen“', '3 KB'],
  ['system.txt', 'Version, Betriebssystem, CPU, Speicher', '1 KB'],
  ['config.effective.yaml', 'Wirksame Konfiguration, Secrets ersetzt', '6 KB'],
  ['logs/daemon.log', 'Letzte 2 000 Zeilen', '388 KB'],
  ['logs/host.log', 'Letzte 2 000 Zeilen', '204 KB'],
  ['logs/runner.log', 'Letzte 2 000 Zeilen', '96 KB'],
  ['logs/cli.log', 'Letzte 412 Zeilen', '31 KB'],
  ['db-schema.txt', 'Schema-Version 14, Migrationsverlauf', '1 KB'],
  ['plugins.txt', 'Installierte Plugins mit Version', '1 KB'],
  ['event-counts.json', 'Anzahl Ereignisse je Typ – ohne Inhalte', '2 KB'],
]

function DiagBundle({ state }: { state: string }) {
  const anon = state !== 'preview'
  return (
    <SettingsShell
      section="bundle"
      overlay={
        state === 'include-session' ? (
          <Overlay
            title="Session-Inhalt ins Bundle aufnehmen?"
            footer={
              <>
                <Btn>Ohne Session-Inhalt</Btn>
                <Btn>Session aufnehmen</Btn>
              </>
            }
          >
            <p>
              „Rate-Limiter für die Login-API“ enthält Nachrichten, Befehle und Code aus <Mono>shop-frontend</Mono>. Inhalte werden redigiert, bekannte Secrets
              ersetzt – trotzdem kann Code aus deinem Repo darin stehen.
            </p>
            <p className="mt-2 text-muted-foreground">Terminals mit Anmeldungen (z. B. <Mono>codex login</Mono>) sind nie enthalten.</p>
          </Overlay>
        ) : null
      }
    >
      <PageHeader title="Diagnose-Bundle">
        Ein Archiv für die Fehlersuche, z. B. für ein GitHub-Issue. Es enthält keine Secrets und wird nicht hochgeladen – du entscheidest, wem du die Datei gibst.
      </PageHeader>
      <F id="OBS-006">
        {state === 'written' && (
          <Banner
            tone="ok"
            title="Bundle gespeichert"
            actions={
              <Btn>
                <FolderOpen className="size-3.5" /> Im Finder zeigen
              </Btn>
            }
          >
            <Mono>~/Desktop/beton-diagnose-2026-10-03-1405.tar.gz</Mono> · 236 KB · anonymisiert
          </Banner>
        )}
        <div className="grid grid-cols-[1fr_340px] gap-6">
          <div>
            <SectionTitle aside="10 Dateien · 733 KB vor Kompression">Inhalt</SectionTitle>
            <div className="divide-y divide-border rounded-md border border-border">
              {BUNDLE.map(([f, d, s], i) => (
                <div key={f} className={cn('grid grid-cols-[180px_1fr_60px] gap-3 px-3 py-1.5 text-[13px]', i === 2 && 'bg-accent/60')}>
                  <Mono>{f}</Mono>
                  <span className="text-muted-foreground">{d}</span>
                  <span className="text-right text-[12px] text-muted-foreground tabular-nums">{s}</span>
                </div>
              ))}
              {state === 'include-session' && (
                <div className="grid grid-cols-[180px_1fr_60px] gap-3 px-3 py-1.5 text-[13px]">
                  <Mono>session/ses_01J9X7F3K.jsonl</Mono>
                  <span className="text-muted-foreground">Verlauf, redigiert</span>
                  <span className="text-right text-[12px] text-muted-foreground">1,2 MB</span>
                </div>
              )}
            </div>
            <Row label="Hostnamen, Benutzernamen und Pfade ersetzen" hint="Aus „ingos-macbook“ wird „host-1“, aus „/Users/ingo“ wird „~“." className="mt-2">
              <Switch checked={anon} aria-label="Anonymisieren" />
            </Row>
            <Row label="Eine Session mitnehmen" hint="Nur wenn der Fehler in einer bestimmten Session auftritt. Standard: keine Inhalte.">
              <Btn>Session wählen …</Btn>
            </Row>
            <div className="mt-4 flex items-center gap-2">
              <Btn primary>
                <FileArchive className="size-3.5" /> Bundle speichern …
              </Btn>
              <NetNote kind="local">Wird nur lokal gespeichert</NetNote>
            </div>
          </div>
          <F id="OBS-002">
            <SectionTitle aside="Vorschau">config.effective.yaml</SectionTitle>
            <pre className="overflow-x-auto rounded-md border border-border bg-card p-3 font-mono text-[11.5px] leading-relaxed">
              {`daemon:
  listen: 127.0.0.1:7420
  data_dir: ${anon ? '~/.beton' : '/Users/ingo/.beton'}
harnesses:
  claude: { path: /opt/homebrew/bin/claude }
  codex:  { path: ${anon ? '~' : '/Users/ingo'}/.local/bin/codex }
providers:
  litellm-intern:
    url: https://llm.intern.example
    api_key: `}
              <mark className="bg-muted px-0.5 text-foreground">[REDACTED:secret_ref]</mark>
              {`
mcp:
  github:
    env:
      GITHUB_PERSONAL_ACCESS_TOKEN: `}
              <mark className="bg-muted px-0.5 text-foreground">[REDACTED:ghp]</mark>
              {`
  sentry:
    headers:
      Authorization: `}
              <mark className="bg-muted px-0.5 text-foreground">[REDACTED:bearer]</mark>
              {`
telemetry: { enabled: false }`}
            </pre>
            <p className="mt-2 text-[12px] text-muted-foreground">
              Ersetzt werden bekannte Muster (Tokens, Bearer-Header, private Schlüssel) und jeder Wert aus deinem Schlüsselbund.
            </p>
          </F>
        </div>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Logs (OBS-001, OBS-002) ───────────────────────── */

type LogLine = { ts: string; level: 'ERROR' | 'WARN' | 'INFO' | 'DEBUG'; comp: string; target: string; msg: ReactNode; fields?: Record<string, string> }

const LOGS: LogLine[] = [
  { ts: '14:05:12.418', level: 'INFO', comp: 'daemon', target: 'beton_server::ws', msg: 'client attached', fields: { session_id: 'ses_01J9X7F3K', from_seq: '412', request_id: 'req_01J9Y2A' } },
  { ts: '14:05:12.402', level: 'INFO', comp: 'daemon', target: 'beton_server::ws', msg: 'handshake ok, protocol 1.4', fields: { client: 'web/0.9.2' } },
  { ts: '14:05:09.977', level: 'WARN', comp: 'daemon', target: 'beton_server::ws', msg: 'heartbeat timeout, closing 4408', fields: { conn: 'c_7' } },
  { ts: '14:04:58.231', level: 'INFO', comp: 'runner', target: 'beton_runner::tool', msg: 'tool.call.completed status=ok duration_ms=4812', fields: { session_id: 'ses_01J9X7F3K', seq: '431' } },
  { ts: '14:04:53.410', level: 'INFO', comp: 'host', target: 'beton_policy::eval', msg: 'decision ask (rule git-push-fragen)', fields: { session_id: 'ses_01J9X7F3K', seq: '428' } },
  {
    ts: '14:04:41.006',
    level: 'ERROR',
    comp: 'host',
    target: 'beton_mcp::client',
    msg: (
      <>
        mcp server sentry failed: 401 Unauthorized (Authorization: Bearer <mark className="bg-muted px-0.5">[REDACTED:bearer]</mark>)
      </>
    ),
    fields: { session_id: 'ses_01J9X7F3K', mcp_server: 'sentry' },
  },
  { ts: '14:04:40.120', level: 'INFO', comp: 'runner', target: 'beton_runner::sandbox', msg: 'sandbox started backend=sandbox-exec stage=tools', fields: { runner_id: 'run_local' } },
  { ts: '14:03:02.551', level: 'INFO', comp: 'daemon', target: 'beton_store::events', msg: 'appended 64 events in batch', fields: { session_id: 'ses_01J9W6P2Z' } },
  { ts: '14:02:47.090', level: 'WARN', comp: 'daemon', target: 'beton_core::features', msg: 'unknown feature flag "brwoser" ignored' },
  { ts: '14:02:47.004', level: 'INFO', comp: 'daemon', target: 'beton_server', msg: 'daemon started, version 0.9.2' },
]

function LevelTag({ l }: { l: LogLine['level'] }) {
  return (
    <span className={cn('inline-flex w-12 items-center gap-1 text-[11px] font-medium', l === 'ERROR' && 'text-deny', l === 'WARN' && 'text-foreground', (l === 'INFO' || l === 'DEBUG') && 'text-muted-foreground')}>
      {l === 'ERROR' && <span className="size-1.5 rotate-45 bg-deny" />}
      {l === 'WARN' && <span className="size-1.5 rotate-45 border border-foreground" />}
      {l.toLowerCase()}
    </span>
  )
}

function DiagLogs({ state }: { state: string }) {
  const filtered = state === 'filtered'
  const lines = filtered ? LOGS.filter((l) => l.fields?.session_id === 'ses_01J9X7F3K') : LOGS
  const debug = state === 'debug'
  return (
    <SettingsShell section="logs" wide>
      <PageHeader
        title="Logs"
        actions={
          <Btn>
            <FolderOpen className="size-3.5" /> Ordner öffnen
          </Btn>
        }
      >
        Strukturierte Logs aller Komponenten aus <Mono>~/.beton/logs/</Mono>. Prompts und Antworten stehen nie auf Stufe info oder höher; Secrets sind ersetzt.
      </PageHeader>
      <F id="OBS-001">
        <div className="flex flex-wrap items-center gap-2">
          <Segmented
            label="Komponente"
            value="all"
            options={[
              { id: 'all', label: 'Alle' },
              { id: 'daemon', label: 'daemon' },
              { id: 'host', label: 'host' },
              { id: 'runner', label: 'runner' },
              { id: 'cli', label: 'cli' },
            ]}
          />
          <Segmented
            label="Stufe"
            value={debug ? 'debug' : 'info'}
            options={[
              { id: 'error', label: 'Fehler' },
              { id: 'warn', label: '≥ Warnung' },
              { id: 'info', label: '≥ Info' },
              { id: 'debug', label: '≥ Debug' },
            ]}
          />
          <span className={cn('flex h-8 w-56 items-center gap-1.5 rounded-md border bg-card px-2 font-mono text-[12px]', filtered ? 'border-ring' : 'border-input text-muted-foreground')}>
            {filtered ? 'session_id = ses_01J9X7F3K' : 'session_id, request_id …'}
          </span>
          <span className="flex h-8 flex-1 items-center gap-1.5 rounded-md border border-input bg-card px-2 text-[12px] text-muted-foreground">
            <Search className="size-3.5" /> Text suchen
          </span>
          <label className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
            <Switch size="sm" defaultChecked aria-label="Live mitlesen" /> live
          </label>
        </div>
        {debug && (
          <div className="mt-3 rounded-md border border-border bg-card p-2.5 text-[12px]">
            Debug-Logs sind für <Mono>beton_policy</Mono> eingeschaltet (<Mono>BETON_LOG=beton_policy=debug</Mono>). Auf Stufe debug können Inhalte stehen – nicht
            dauerhaft anlassen.
          </div>
        )}
        <div className="mt-3 overflow-hidden rounded-md border border-border font-mono text-[12px]">
          <div className="grid grid-cols-[96px_56px_64px_180px_1fr] gap-3 bg-sunken px-3 py-1.5 font-sans text-[11px] text-muted-foreground">
            <span>Zeit</span>
            <span>Stufe</span>
            <span>Komponente</span>
            <span>Ziel</span>
            <span>Meldung</span>
          </div>
          {(debug
            ? [
                { ts: '14:04:53.409', level: 'DEBUG' as const, comp: 'host', target: 'beton_policy::eval', msg: 'matched rule git-push-fragen (scope=project, prio=40)', fields: { seq: '428' } },
                { ts: '14:04:53.408', level: 'DEBUG' as const, comp: 'host', target: 'beton_policy::eval', msg: 'evaluate phase=pre_tool tool=Bash eval_us=212' },
                ...lines,
              ]
            : lines
          ).map((l, i) => {
            const open = state === 'raw' && i === 5
            return (
              <div key={i} className={cn('border-t border-border', l.level === 'ERROR' && 'bg-deny-soft/50')}>
                <div className="grid grid-cols-[96px_56px_64px_180px_1fr] gap-3 px-3 py-1 leading-relaxed">
                  <span className="text-muted-foreground tabular-nums">{l.ts}</span>
                  <LevelTag l={l.level} />
                  <span>{l.comp}</span>
                  <span className="truncate text-muted-foreground">{l.target}</span>
                  <span className="min-w-0">
                    <ChevronRight className={cn('mr-1 inline size-3 text-muted-foreground', open && 'rotate-90')} />
                    {l.msg}
                    {l.fields && (
                      <span className="ml-2 text-[11px] text-muted-foreground">
                        {Object.entries(l.fields).map(([k, v]) => (
                          <span key={k} className={cn('mr-2', filtered && k === 'session_id' && 'rounded-sm bg-accent px-1 text-foreground')}>
                            {k}={v}
                          </span>
                        ))}
                      </span>
                    )}
                  </span>
                </div>
                {open && (
                  <pre className="mx-3 mb-2 overflow-x-auto rounded-sm bg-card p-2 text-[11px] leading-relaxed">
                    {`{"ts":"2026-10-03T12:04:41.006Z","level":"ERROR","target":"beton_mcp::client","component":"host",
 "version":"0.9.2","session_id":"ses_01J9X7F3K","request_id":"req_01J9Y1Q",
 "message":"mcp server sentry failed: 401 Unauthorized (Authorization: Bearer [REDACTED:bearer])"}`}
                  </pre>
                )}
              </div>
            )
          })}
        </div>
        <p className="mt-2 text-[12px] text-muted-foreground">Dateien werden täglich gewechselt und nach 7 Tagen oder 100 MB gelöscht.</p>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Verbindung & Protokoll (PROTO-004…009, 015) ───────────────────────── */

function DiagConnection({ state }: { state: string }) {
  const banner: Record<string, ReactNode> = {
    reconnecting: (
      <F id="PROTO-009">
        <Banner tone="neutral" title="Verbindung verloren – verbinde neu …" actions={<Btn>Jetzt versuchen</Btn>}>
          Seit 60 s kein Lebenszeichen vom Daemon (Code 4408). Versuch 4, nächster in 3,2 s; die Wartezeit wächst bis 30 s. Laufende Sessions arbeiten weiter.
        </Banner>
      </F>
    ),
    resumed: (
      <F id={['PROTO-005', 'PROTO-009']}>
        <Banner tone="ok" title="Wieder verbunden">
          Nach 41 s fortgesetzt ab Ereignis 418 der Session „Rate-Limiter für die Login-API“: 23 Ereignisse nachgeladen, keine Lücke, keine Doppelten. Laufende Antwort aus dem
          Zwischenspeicher ergänzt.
        </Banner>
      </F>
    ),
    shutdown: (
      <F id="PROTO-009">
        <Banner tone="neutral" title="beton startet neu (Update auf 0.9.3)">
          Der Daemon hat sich ordentlich abgemeldet (Code 4503). Die Verbindung wird in wenigen Sekunden automatisch wiederhergestellt.
        </Banner>
      </F>
    ),
    overflow: (
      <F id="PROTO-008">
        <Banner tone="neutral" title="Ansicht kam nicht hinterher – lädt neu">
          „Nächtliches Dependency-Update“ erzeugt gerade sehr viele Ereignisse. Über 4 MiB waren ausstehend, deshalb lädt diese Ansicht ab Ereignis 9 812 neu. Andere Fenster
          sind nicht betroffen.
        </Banner>
      </F>
    ),
    incompatible: (
      <F id={['PROTO-004', 'PROTO-011']}>
        <div className="mb-5">
          <ProblemBox
            title="Diese App und der Daemon sprechen verschiedene Protokollversionen"
            detail="Der Daemon unterstützt 1.2–1.3, diese App braucht mindestens 1.4. Wahrscheinlich läuft noch ein alter Daemon aus einer früheren Installation."
            next={
              <>
                Beende ihn mit <Mono>beton daemon restart</Mono> oder aktualisiere beton.
              </>
            }
            problem={{ type: 'urn:beton:problem:protocol_unsupported', status: 400, code: 'protocol_unsupported', trace_id: '0af7651916cd43dd8448eb211c80319c' }}
          />
        </div>
      </F>
    ),
    tunnel: (
      <F id="PROTO-015">
        <Banner tone="neutral" title="Runner „shop-ci (Container)“ getrennt – 214 Ereignisse gepuffert">
          Der Runner verbindet sich neu und reicht die Ereignisse in Reihenfolge nach. Bei 64 MiB Puffer pausiert er den Agent, bis die Verbindung steht.
        </Banner>
      </F>
    ),
  }
  const down = state === 'reconnecting' || state === 'incompatible' || state === 'shutdown'
  return (
    <SettingsShell section="connection" wide connection={down ? 'offline' : 'local'}>
      <PageHeader title="Verbindung">Wie diese App mit dem beton-Daemon auf deinem Rechner spricht. Für die Fehlersuche – im Alltag musst du hier nichts tun.</PageHeader>
      {banner[state]}
      <div className={cn('grid grid-cols-2 gap-x-8', down && 'opacity-60')}>
        <F id="PROTO-004">
          <SectionTitle>WebSocket</SectionTitle>
          <KV
            rows={[
              ['Adresse', <Mono>ws://127.0.0.1:7420/v1/ws</Mono>],
              ['Protokoll', state === 'incompatible' ? <span className="text-deny">App 1.4 · Daemon 1.2–1.3</span> : <span>1.4 ausgehandelt <span className="text-muted-foreground">(App 1.4, Daemon 1.4)</span></span>],
              ['Lebenszeichen', state === 'reconnecting' ? <span className="text-deny">seit 60 s keins</span> : 'vor 6 s · alle 20 s'],
              ['Verbunden seit', state === 'resumed' ? 'vor 3 s (Neuversuch 4)' : '14:02:47'],
            ]}
          />
        </F>
        <F id="PROTO-008">
          <SectionTitle>Durchsatz</SectionTitle>
          <KV
            rows={[
              ['Ausgehender Puffer', state === 'overflow' ? <span className="text-deny">4,1 MiB von 4 MiB</span> : '12 KiB von 4 MiB'],
              ['Bündelung', 'alle 16 ms oder 64 Ereignisse'],
              ['Befehle', '2 / s · Grenze 100 / s'],
              ['Zusammengefasst', state === 'overflow' ? '18 210 Text-Deltas' : '–'],
            ]}
          />
        </F>
      </div>

      <F id="PROTO-005" className={cn(down && 'opacity-60')}>
        <SectionTitle aside="Ereignisse kommen lückenlos in Reihenfolge, auch nach Neuverbindung">Angehängte Sessions</SectionTitle>
        <div className="overflow-hidden rounded-md border border-border">
          <table className="w-full text-[13px]">
            <thead className="bg-sunken text-[12px] text-muted-foreground">
              <tr>
                <th className="py-1.5 pl-3 text-left font-medium">Session</th>
                <th className="text-right font-medium">Gesehen bis</th>
                <th className="text-right font-medium">Neuestes</th>
                <th className="pr-3 pl-6 text-left font-medium">Modus</th>
              </tr>
            </thead>
            <tbody className="type-narrow tabular-nums">
              {[
                { t: 'Rate-Limiter für die Login-API', h: 'claude' as const, seen: state === 'resumed' ? 441 : 431, head: 441, mode: state === 'resumed' ? 'live (nachgeladen 419–441)' : 'live' },
                { t: 'Review: Rate-Limiter', h: 'codex' as const, seen: 212, head: 212, mode: 'live' },
                { t: 'Nächtliches Dependency-Update', h: 'claude' as const, seen: state === 'overflow' ? 9812 : 10_244, head: 10_244, mode: state === 'overflow' ? 'lädt neu ab 9 812' : 'nur letzte 200 (Liste)' },
              ].map((r) => (
                <tr key={r.t} className="border-t border-border">
                  <td className="py-1.5 pl-3 font-sans">
                    {r.t} <HarnessBadge id={r.h} className="ml-2 text-[11px] text-muted-foreground" />
                  </td>
                  <td className="text-right">{r.seen.toLocaleString('de-DE')}</td>
                  <td className="text-right">{r.head.toLocaleString('de-DE')}</td>
                  <td className="pr-3 pl-6 font-sans text-[12px] text-muted-foreground">{r.mode}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </F>

      <div className={cn('grid grid-cols-2 gap-x-8', down && 'opacity-60')}>
        <F id="PROTO-007">
          <SectionTitle>Datenkanäle</SectionTitle>
          <div className="divide-y divide-border rounded-md border border-border text-[13px]">
            {[
              ['Terminal (Claude Code)', 'nur lesen', '256 KiB Kredit frei'],
              ['Terminal (deins)', 'lesen & tippen', '1 MiB Kredit frei'],
              ['Browser localhost:5173', 'ansehen', '1 von 2 Bildern ausstehend'],
            ].map(([n, r, c]) => (
              <div key={n} className="grid grid-cols-[1fr_100px_150px] gap-2 px-3 py-1.5">
                <span>{n}</span>
                <span className="text-[12px] text-muted-foreground">{r}</span>
                <span className="text-right text-[12px] text-muted-foreground tabular-nums">{c}</span>
              </div>
            ))}
          </div>
        </F>
        <F id="PROTO-006">
          <SectionTitle>Letzte Befehle</SectionTitle>
          <div className="divide-y divide-border rounded-md border border-border font-mono text-[12px]">
            {[
              ['input.submit', 'ok', 'inp_01J9Y3 · Schlüssel 7c1e…'],
              ['input.submit', 'ok', 'Wiederholung erkannt → gleiches Ergebnis'],
              ['approval.resolve', 'ok', 'apr_01J9Y2 allow'],
              ['session.pin', 'abgelehnt', 'unknown_command'],
            ].map(([n, s, d], i) => (
              <div key={i} className="grid grid-cols-[130px_74px_1fr] gap-2 px-3 py-1.5">
                <span>{n}</span>
                <span className={cn('font-sans', s === 'ok' ? 'text-ok' : 'text-deny')}>{s}</span>
                <span className="truncate text-muted-foreground">{d}</span>
              </div>
            ))}
          </div>
        </F>
      </div>

      <F id="PROTO-015" className={cn(down && 'opacity-60')}>
        <SectionTitle aside="Runner verbinden sich nur ausgehend; kein offener Port">Runner</SectionTitle>
        <div className="divide-y divide-border rounded-md border border-border text-[13px]">
          {[
            { n: 'Lokaler Runner', via: 'Unix-Socket', ep: 'Epoche 3', buf: '0 unbestätigt', ok: true },
            { n: 'shop-ci (Container)', via: 'Unix-Socket', ep: 'Epoche 1', buf: state === 'tunnel' ? '214 gepuffert · 1,8 MiB von 64 MiB' : '0 unbestätigt', ok: state !== 'tunnel' },
          ].map((r) => (
            <div key={r.n} className="grid grid-cols-[16px_1fr_120px_90px_230px] items-center gap-2 px-3 py-1.5">
              {r.ok ? <span className="size-2 rounded-full bg-ok" /> : <RefreshCw className="size-3 animate-spin" />}
              <span>{r.n}</span>
              <span className="text-[12px] text-muted-foreground">{r.via}</span>
              <span className="text-[12px] text-muted-foreground">{r.ep}</span>
              <span className="text-right text-[12px] text-muted-foreground tabular-nums">{r.buf}</span>
            </div>
          ))}
        </div>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Event-Log einer Session ───────────────────────── */

type Ev = { seq?: number; tseq?: number; ts: string; actor: string; type: string; summary: ReactNode; unknown?: boolean; f?: string }

const EVENTS: Ev[] = [
  { seq: 431, ts: '12:04:58.231', actor: 'harness', type: 'tool.call.completed', summary: 'call_7Kd · ok · 4 812 ms · Ergebnis als Anhang (71 KB)' },
  { tseq: 18822, ts: '12:04:57.904', actor: 'harness', type: 'tool.call.output.delta', summary: 'stdout: „✓ middleware/rate-limit.spec.ts (4)“' },
  { tseq: 18821, ts: '12:04:57.880', actor: 'harness', type: 'tool.call.output.delta', summary: 'stdout: „✓ auth/login.spec.ts (6)“' },
  { seq: 430, ts: '12:04:53.414', actor: 'system', type: 'terminal.snapshot', summary: 'term_2 · 120×32 · Bildschirmstand als Anhang', f: 'DATA-011' },
  { seq: 429, ts: '12:04:53.412', actor: 'harness', type: 'tool.call.started', summary: 'call_7Kd · Sandbox: tools' },
  { seq: 428, ts: '12:04:53.410', actor: 'system', type: 'policy.decision', summary: 'pre_tool · allow · Regel shell-tests-erlauben · 0,2 ms' },
  { seq: 427, ts: '12:04:53.398', actor: 'harness', type: 'tool.call.requested', summary: 'Bash · pnpm vitest run auth' },
  { seq: 426, ts: '12:04:52.120', actor: 'harness', type: 'harness.unmapped', summary: 'Claude-Ereignis „system/hook_progress“ ohne Entsprechung – roh gespeichert' },
  { seq: 425, ts: '12:04:51.002', actor: 'system', type: 'workspace.indexed', summary: 'Unbekannter Typ (aus neuerer Version) – angezeigt, nicht ausgewertet', unknown: true, f: 'PROTO-014' },
  { seq: 424, ts: '12:04:50.871', actor: 'harness', type: 'context.usage', summary: '84 200 von 200 000 Tokens · Quelle: Harness' },
  { seq: 423, ts: '12:04:50.870', actor: 'harness', type: 'cost.delta', summary: 'claude-opus-5-5 · 18 210 ein / 1 104 aus · Subscription' },
  { seq: 422, ts: '12:04:50.869', actor: 'harness', type: 'usage.subscription', summary: 'claude · 5h · 38 % · Reset 14:19 UTC' },
  { seq: 421, ts: '12:04:50.701', actor: 'harness', type: 'message.completed', summary: 'assistant · „Die Login-Route ist jetzt auf 5 Versuche …“' },
  { seq: 420, ts: '12:04:31.044', actor: 'user:Ingo', type: 'turn.started', summary: 'turn_0F · Eingabe inp_01J9Y3' },
]

function DiagEvents({ state }: { state: string }) {
  const showT = state === 'transient'
  const redacted = state === 'redacted'
  const rows = EVENTS.filter((e) => showT || e.seq !== undefined)
  const sel = state === 'detail' || state === 'redact' ? 431 : redacted ? 421 : undefined
  return (
    <AppLayout sessionList={false} activeSession="ses_7f3k">
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
          <span className="text-[13px] text-muted-foreground">Rate-Limiter für die Login-API ›</span>
          <h2 className="text-[15px] font-semibold">Event-Log</h2>
          <F id="DATA-002" as="span" className="ml-2 inline-flex items-center gap-1.5 text-[12px] text-muted-foreground" badge="bottom-left">
            <Check className="size-3.5 text-ok" /> 441 Ereignisse, lückenlos (1–441)
          </F>
          <span className="ml-auto font-mono text-[11px] text-muted-foreground">ses_01J9X7F3K</span>
        </div>
        <div className="flex items-center gap-2 border-b border-border px-4 py-2">
          <Segmented
            label="Typ"
            value="all"
            options={[
              { id: 'all', label: 'Alle' },
              { id: 'msg', label: 'Nachrichten' },
              { id: 'tool', label: 'Tools' },
              { id: 'ctl', label: 'Freigaben & Policy' },
              { id: 'cost', label: 'Verbrauch' },
              { id: 'sys', label: 'System' },
            ]}
          />
          <F id="PROTO-003" as="span" className="ml-2 flex items-center gap-2 text-[12px]">
            <Switch size="sm" checked={showT} aria-label="Flüchtige Ereignisse zeigen" />
            Flüchtige zeigen <span className="text-muted-foreground">(nur live, nicht gespeichert)</span>
          </F>
          <span className="ml-auto text-[12px] text-muted-foreground">seq 420–431 von 441</span>
        </div>
        <div className="flex min-h-0 flex-1">
          <F id={['PROTO-001', 'PROTO-002']} className="min-h-0 flex-1 overflow-y-auto">
            <table className="w-full text-[12.5px]">
              <thead className="sticky top-0 bg-sunken text-[11px] text-muted-foreground">
                <tr>
                  <th className="w-16 py-1.5 pr-2 text-right font-medium">seq</th>
                  <th className="w-28 text-left font-medium">Zeit (UTC)</th>
                  <th className="w-24 text-left font-medium">Akteur</th>
                  <th className="w-48 text-left font-medium">Typ</th>
                  <th className="text-left font-medium">Inhalt</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((e, i) => {
                  const transient = e.seq === undefined
                  const isRedacted = redacted && e.seq === 421
                  const row = (
                    <tr
                      key={i}
                      className={cn(
                        'border-t border-border align-top',
                        transient && 'text-muted-foreground italic',
                        e.seq === sel && 'bg-accent',
                        e.unknown && 'text-muted-foreground',
                      )}
                    >
                      <td className="py-1.5 pr-2 text-right font-mono tabular-nums">{transient ? <span title={`tseq ${e.tseq}`}>–</span> : e.seq}</td>
                      <td className="py-1.5 font-mono text-[11.5px] tabular-nums">{e.ts}</td>
                      <td className="py-1.5 text-[12px]">{e.actor}</td>
                      <td className="py-1.5 font-mono text-[12px]">
                        {e.f ? <F id={e.f} as="span">{e.type}</F> : e.type}
                        {transient && <span className="ml-1 rounded-sm border border-dashed border-border px-1 text-[10px] not-italic">flüchtig</span>}
                      </td>
                      <td className="py-1.5 pr-3">
                        {isRedacted ? (
                          <span>
                            <span className="font-mono">[REDACTED:manual]</span> <span className="text-muted-foreground">– von Ingo geschwärzt, 14:09</span>
                          </span>
                        ) : (
                          e.summary
                        )}
                      </td>
                    </tr>
                  )
                  return row
                })}
                {redacted && (
                  <tr className="border-t border-border bg-accent/50 align-top">
                    <td className="py-1.5 pr-2 text-right font-mono">442</td>
                    <td className="py-1.5 font-mono text-[11.5px]">12:09:14.002</td>
                    <td className="py-1.5 text-[12px]">user:Ingo</td>
                    <td className="py-1.5 font-mono text-[12px]">
                      <F id="DATA-012" as="span">event.redacted</F>
                    </td>
                    <td className="py-1.5 pr-3">Ereignis 421 geschwärzt · Grund: „GitHub-Token im Text“</td>
                  </tr>
                )}
              </tbody>
            </table>
            <button className="mx-auto my-3 block text-[12px] text-muted-foreground underline underline-offset-2">Ältere laden (419 bis 1)</button>
          </F>

          {(state === 'detail' || state === 'redact') && (
            <div className="flex w-[400px] shrink-0 flex-col border-l border-border">
              <div className="flex items-center gap-2 border-b border-border px-3 py-2 text-[13px]">
                <span className="font-semibold">Ereignis 431</span>
                <span className="text-muted-foreground">dauerhaft</span>
                <button className="ml-auto inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground">
                  <Copy className="size-3" /> JSON
                </button>
              </div>
              <pre className="min-h-0 flex-1 overflow-auto p-3 font-mono text-[11.5px] leading-relaxed">
                {`{
  "v": 1,
  "id": "evt_01J9Y4M2QK7V0W3T8R",
  "session_id": "ses_01J9X7F3K",
  "seq": 431,
  "ts": "2026-10-03T12:04:58.231Z",
  "actor": { "kind": "harness", "id": "claude" },
  "type": "tool.call.completed",
  "turn_id": "turn_0F",
  "causation_id": "evt_01J9Y4KZ81",
  "payload": {
    "call_id": "call_7Kd",
    "status": "ok",
    "duration_ms": 4812,
    "result_ref": "sha256:9f2c…e41a"
  }
}`}
              </pre>
              <div className="border-t border-border p-3 text-[12px] text-muted-foreground">
                Das Ergebnis ist größer als 64 KiB und liegt als Anhang im Speicher. <a className="underline underline-offset-2">Anhang öffnen (71 KB)</a>
              </div>
              <div className="flex gap-2 border-t border-border p-3">
                <Btn>Roh-Ereignis des Harness</Btn>
                <Btn danger>Inhalt schwärzen …</Btn>
              </div>
            </div>
          )}
        </div>

        <F id="PROTO-012" className="flex items-center gap-2 border-t border-border px-4 py-2 text-[12px]">
          <span className="text-muted-foreground">Für Skripte (nur lesen):</span>
          <code className="min-w-0 flex-1 truncate rounded-sm bg-muted px-2 py-0.5 font-mono text-[11.5px]">
            curl -N -H "Authorization: Bearer $BETON_TOKEN" http://127.0.0.1:7420/v1/sessions/ses_01J9X7F3K/events/stream?from_seq=431
          </code>
          <button className="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground">
            <Copy className="size-3" /> Kopieren
          </button>
        </F>

        {state === 'redact' && (
          <Overlay
            title="Inhalt von Ereignis 431 schwärzen?"
            footer={
              <>
                <Btn danger>Schwärzen</Btn>
                <Btn>Abbrechen</Btn>
              </>
            }
          >
            <F id="DATA-012">
              <p>
                Nutze das, wenn versehentlich ein Secret im Verlauf steht. Der Inhalt wird durch <Mono>[REDACTED:manual]</Mono> ersetzt – auf allen Geräten, in Exporten und
                für alle, mit denen die Session geteilt ist.
              </p>
              <div className="mt-3 space-y-1.5">
                <label className="flex items-center gap-2">
                  <span className="size-3.5 rounded-full border-[5px] border-foreground" /> Ganzen Inhalt
                </label>
                <label className="flex items-center gap-2 text-muted-foreground">
                  <span className="size-3.5 rounded-full border border-foreground" /> Nur Felder: <Mono>/payload/result_ref</Mono>
                </label>
              </div>
              <p className="mt-3 text-muted-foreground">Nummer, Typ, Zeit und Akteur bleiben. Roh-Ausgabe und Anhang werden gelöscht. Nicht rückgängig zu machen.</p>
            </F>
          </Overlay>
        )}
      </div>
    </AppLayout>
  )
}

/* ───────────────────────── Speicher (DATA-003, 005, 006, 011) ───────────────────────── */

const STORAGE = [
  { l: 'Datenbank', v: 182, cls: 'bg-foreground' },
  { l: 'Snapshots', v: 620, cls: 'bg-foreground/70' },
  { l: 'Tool-Ergebnisse', v: 410, cls: 'bg-foreground/50' },
  { l: 'Anhänge', v: 290, cls: 'bg-foreground/35' },
  { l: 'Exporte', v: 80, cls: 'bg-foreground/20' },
  { l: 'Logs', v: 64, cls: 'bg-muted-foreground/40' },
]

function DiagStorage({ state }: { state: string }) {
  const total = STORAGE.reduce((s, x) => s + x.v, 0)
  return (
    <SettingsShell section="storage">
      <PageHeader title="Speicher">
        Alles liegt in <Mono>~/.beton/</Mono> auf diesem Rechner – zusammen {(total / 1000).toLocaleString('de-DE', { maximumFractionDigits: 2 })} GB.
      </PageHeader>

      {state === 'migrating' && (
        <F id="DATA-003">
          <Banner tone="neutral" title="Datenbank wird aktualisiert (Schema 13 → 14)">
            Vorher gesichert als <Mono>beton.db.bak-13</Mono>. Dauert etwa 10 Sekunden; Sessions starten danach automatisch weiter.
          </Banner>
        </F>
      )}
      {state === 'too-new' && (
        <F id={['DATA-003', 'PROTO-011']}>
          <div className="mb-5">
            <ProblemBox
              title="Die Datenbank ist neuer als dieses beton"
              detail="Die Daten wurden zuletzt mit beton 0.10.0 (Schema 15) geöffnet. beton 0.9.2 kennt nur Schema 14 und startet nicht, um nichts zu beschädigen."
              next="Installiere wieder beton 0.10 oder neuer. Zurückspielen geht nur mit der Sicherung beton.db.bak-14."
              problem={{ type: 'urn:beton:problem:schema_too_new', status: 503, code: 'schema_too_new', trace_id: '5b8aa5a2d2c872e8321cf37308d69df2' }}
            />
          </div>
        </F>
      )}

      <div role="img" aria-label="Speicherbelegung" className="flex h-3 overflow-hidden rounded-sm">
        {STORAGE.map((x) => (
          <span key={x.l} className={cn('h-full border-r-2 border-background last:border-r-0', x.cls)} style={{ width: `${(x.v / total) * 100}%` }} title={`${x.l}: ${x.v} MB`} />
        ))}
      </div>
      <div className="mt-2 flex flex-wrap gap-x-5 gap-y-1 text-[12px]">
        {STORAGE.map((x) => (
          <span key={x.l} className="inline-flex items-center gap-1.5">
            <span className={cn('size-2.5 rounded-[2px]', x.cls)} />
            {x.l} <span className="text-muted-foreground tabular-nums">{x.v.toLocaleString('de-DE')} MB</span>
          </span>
        ))}
      </div>

      <F id="DATA-003">
        <SectionTitle>Datenbank</SectionTitle>
        <KV
          rows={[
            ['Datei', <Mono>~/.beton/beton.db · SQLite, WAL</Mono>],
            ['Schema', state === 'migrating' ? '13 → 14 (läuft)' : state === 'too-new' ? <span className="text-deny">15 – zu neu für beton 0.9.2</span> : '14 · aktuell'],
            ['Sicherungen', <span>beton.db.bak-13 (vom 28.09.) <span className="text-muted-foreground">· vor jeder Aktualisierung automatisch</span></span>],
            ['Integrität', <span className="inline-flex items-center gap-1 text-ok"><Check className="size-3.5" /> geprüft heute 14:03</span>],
          ]}
        />
      </F>

      <F id="DATA-005">
        <SectionTitle aside="aus dem Verlauf abgeleitet">Ansichten</SectionTitle>
        <Row label="Session-Liste, Inbox, Verbrauch" hint="Werden bei jedem Ereignis mitgeschrieben. Bei Unstimmigkeiten aus dem Verlauf neu aufbauen – es geht nichts verloren.">
          <Btn>
            <RefreshCw className="size-3.5" /> Neu aufbauen
          </Btn>
        </Row>
      </F>

      <F id="DATA-006">
        <SectionTitle aside={state === 'gc' ? 'Aufräumen läuft …' : 'Nächstes Aufräumen heute 20:00'}>Anhänge und große Inhalte</SectionTitle>
        <KV
          rows={[
            ['Ablage', <Mono>~/.beton/blobs/sha256/ · 3 812 Dateien</Mono>],
            ['Gleiche Inhalte', 'nur einmal gespeichert'],
            ['Nicht mehr verwendet', state === 'gc' ? <span>214 MB werden entfernt … <span className="text-muted-foreground">1 204 von 1 690</span></span> : '214 MB · werden 24 Std. nach dem Löschen der Session entfernt'],
          ]}
        />
        <div className="mt-2 flex gap-2">
          <Btn disabled={state === 'gc'}>
            <Trash2 className="size-3.5" /> Jetzt aufräumen
          </Btn>
        </div>
        {state === 'corrupt' && (
          <F id="PROTO-011" className="mt-3">
            <ProblemBox
              open
              title="Ein Anhang ist beschädigt"
              detail={
                <>
                  Der Screenshot aus „Checkout-Formular barrierefrei machen“ (Ereignis 1 207) passt nicht mehr zu seiner Prüfsumme. Der Verlauf ist intakt, nur dieses Bild fehlt.
                </>
              }
              next="Prüfe die Festplatte. Ist eine Sicherung von ~/.beton/blobs vorhanden, stelle die Datei von dort wieder her."
              problem={{
                type: 'urn:beton:problem:blob_corrupt',
                status: 500,
                code: 'blob_corrupt',
                instance: '/v1/sessions/ses_01J8Q6Q2A/blobs/sha256:4be1…',
                trace_id: '1f3a0c9e77b84d5c9a1e2b6d0c4f8a21',
              }}
            />
          </F>
        )}
      </F>

      <F id="DATA-011">
        <SectionTitle>Snapshots</SectionTitle>
        <p className="text-[13px] text-muted-foreground">
          Von Terminals und dem eingebauten Browser speichert beton nur Bildschirmstände und Screenshots – höchstens alle 10 Sekunden. Die laufende Terminal-Ausgabe, Browser-Bilder
          und Sprachaufnahmen werden nie dauerhaft gespeichert.
        </p>
      </F>
    </SettingsShell>
  )
}

/* ───────────────────────── Metriken & Traces (OBS-003, OBS-004, OBS-009) ───────────────────────── */

const SPANS: { name: string; start: number; dur: number; depth: number; detail: string; wait?: boolean }[] = [
  { name: 'turn', start: 0, dur: 100, depth: 0, detail: 'turn_0F · 1 Min. 12 s' },
  { name: 'model.request', start: 1, dur: 22, depth: 1, detail: 'claude-opus-5-5 · 18 210 ein / 1 104 aus' },
  { name: 'policy.evaluate', start: 24, dur: 1, depth: 1, detail: 'pre_tool · allow' },
  { name: 'tool.call', start: 25, dur: 9, depth: 1, detail: 'Bash · ok · 4,8 s' },
  { name: 'model.request', start: 35, dur: 14, depth: 1, detail: 'claude-opus-5-5' },
  { name: 'policy.evaluate', start: 50, dur: 1, depth: 1, detail: 'pre_tool · ask' },
  { name: 'approval.wait', start: 51, dur: 41, depth: 1, detail: 'auf dich gewartet · 29 s', wait: true },
  { name: 'tool.call', start: 92, dur: 6, depth: 1, detail: 'git push · ok' },
]

function DiagMetrics({ state }: { state: string }) {
  const operator = state === 'operator' || state === 'not-ready'
  const notReady = state === 'not-ready'
  return (
    <SettingsShell section="metrics" connection={operator ? 'server' : 'local'}>
      <PageHeader title="Metriken & Traces">
        Für Betreiber: Daten bleiben bei dir und gehen nur an Systeme, die du selbst einträgst. Unabhängig von der anonymen Nutzungsstatistik.
      </PageHeader>

      <F id="OBS-009">
        <SectionTitle aside={operator ? 'team.example.com' : 'dieser Rechner'}>Zustand</SectionTitle>
        <div className="divide-y divide-border rounded-md border border-border font-mono text-[12px]">
          {[
            ['GET /healthz', '200', 'Prozess lebt', true],
            ['GET /readyz', notReady ? '503' : '200', notReady ? 'nicht bereit: Datenbank nicht erreichbar' : 'Datenbank, Migrationen, Anhänge ok', !notReady],
            ['GET /v1/info', '200', 'version 0.9.2 · protocol 1.4 · features voice, browser', true],
          ].map(([p, c, d, ok]) => (
            <div key={p as string} className="grid grid-cols-[150px_48px_1fr] gap-3 px-3 py-1.5">
              <span>{p}</span>
              <span className={cn('inline-flex items-center gap-1', ok ? 'text-ok' : 'text-deny')}>
                {ok ? <Check className="size-3" /> : <span className="size-1.5 rotate-45 bg-deny" />}
                {c}
              </span>
              <span className="font-sans text-[12.5px] text-muted-foreground">{d}</span>
            </div>
          ))}
        </div>
        {notReady && <p className="mt-1.5 text-[12px] text-muted-foreground">Die Antworten nennen bewusst keine Hostnamen oder Fehlertexte der Datenbank; Details stehen in den Logs.</p>}
      </F>

      <F id="OBS-004">
        <SectionTitle aside={operator ? <NetNote kind="network">abrufbar unter 127.0.0.1:9464/metrics (mit Token)</NetNote> : <NetNote kind="local">aus</NetNote>}>Prometheus-Metriken</SectionTitle>
        {operator ? (
          <pre className="overflow-x-auto rounded-md border border-border bg-card p-3 font-mono text-[11.5px] leading-relaxed">
            {`# TYPE beton_sessions_active gauge
beton_sessions_active{harness="claude"} 4
beton_sessions_active{harness="codex"} 2
# TYPE beton_policy_evaluations_total counter
beton_policy_evaluations_total{decision="allow"} 18412
beton_policy_evaluations_total{decision="ask"} 311
beton_policy_evaluations_total{decision="deny"} 57
# TYPE beton_tokens_total counter
beton_tokens_total{harness="claude",model="claude-opus-5-5",kind="input"} 8.42e+07`}
          </pre>
        ) : (
          <Row label="Endpunkt /metrics anbieten" hint="Für ein eigenes Prometheus. Labels enthalten nie Session-, Personen- oder Pfadangaben.">
            <Switch aria-label="Metriken anbieten" />
          </Row>
        )}
      </F>

      <F id="OBS-003">
        <SectionTitle aside={operator ? <NetNote kind="network">Export an http://otel-collector:4317</NetNote> : <NetNote kind="local">kein Export eingetragen</NetNote>}>Traces (OpenTelemetry)</SectionTitle>
        {!operator && (
          <Row label="OTLP-Endpunkt" hint="Ohne Eintrag wird nichts exportiert. Prompt-Inhalte nur mit ausdrücklicher Einstellung.">
            <span className="flex h-8 w-64 items-center rounded-md border border-input bg-card px-2 font-mono text-[12px] text-muted-foreground">z. B. http://localhost:4317</span>
          </Row>
        )}
        <div className="mt-2 rounded-md border border-border p-3">
          <div className="mb-2 flex items-baseline gap-2 text-[12px]">
            <span className="font-medium">Ein Turn als Trace</span>
            <span className="text-muted-foreground">Vorschau · jede Session hat eine eigene Trace-ID</span>
          </div>
          {SPANS.map((s, i) => (
            <div key={i} className="grid grid-cols-[150px_1fr] items-center gap-2 py-0.5 text-[12px]">
              <span className={cn('truncate font-mono text-[11.5px]', s.depth && 'pl-3')}>{s.name}</span>
              <div className="relative h-4">
                <span
                  className={cn('absolute top-0.5 h-3 rounded-[2px]', s.wait ? 'border border-dashed border-foreground/60 bg-transparent' : s.depth ? 'bg-foreground/55' : 'bg-foreground/20')}
                  style={{ left: `${s.start}%`, width: `${s.dur}%` }}
                />
                <span className={cn('absolute top-0 text-[10.5px] whitespace-nowrap', s.dur >= 90 ? 'text-foreground' : 'text-muted-foreground')} style={
                    s.dur >= 90
                      ? { left: `calc(${s.start}% + 6px)` }
                      : s.start + s.dur > 70
                        ? { right: `calc(${100 - s.start}% + 6px)` }
                        : { left: `calc(${s.start + s.dur}% + 6px)` }
                  }>
                  {s.detail}
                </span>
              </div>
            </div>
          ))}
        </div>
      </F>
    </SettingsShell>
  )
}

export const group: ScreenGroup = {
  id: 'diagnostics',
  title: 'Diagnose',
  order: 230,
  screens: [
    {
      id: 'diag-doctor',
      title: 'Umgebung prüfen',
      description: '„beton doctor“ in der App: Prüfungen nach Bereich mit ok / Hinweis / Fehler, Ursache und Befehl zur Behebung.',
      features: ['OBS-005', 'UX-007'],
      states: [
        { id: 'warn', title: 'Mit Hinweisen' },
        { id: 'ok', title: 'Alles ok' },
        { id: 'fail', title: 'Mit Fehler' },
        { id: 'running', title: 'Prüft …' },
      ],
      component: DiagDoctor,
    },
    {
      id: 'diag-bundle',
      title: 'Diagnose-Bundle',
      description: '„beton diagnose“: Inhaltsverzeichnis vor dem Speichern, Vorschau mit ersetzten Secrets, optional anonymisiert; nie automatischer Upload.',
      features: ['OBS-006', 'OBS-002'],
      states: [
        { id: 'preview', title: 'Vorschau' },
        { id: 'include-session', title: 'Session aufnehmen?' },
        { id: 'written', title: 'Gespeichert' },
      ],
      component: DiagBundle,
    },
    {
      id: 'diag-logs',
      title: 'Logs',
      description: 'JSON-Lines-Logs aller Komponenten, gefiltert nach Komponente, Stufe und session_id; Secrets sind in den Zeilen ersetzt.',
      features: ['OBS-001', 'OBS-002'],
      states: [
        { id: 'default', title: 'Alle' },
        { id: 'filtered', title: 'Nach session_id' },
        { id: 'raw', title: 'Zeile aufgeklappt' },
        { id: 'debug', title: 'Debug für ein Modul' },
      ],
      component: DiagLogs,
    },
    {
      id: 'diag-connection',
      title: 'Verbindung',
      description:
        'Zustand der Verbindung zum Daemon: Versionsaushandlung, angehängte Sessions mit seq, Datenkanäle mit Kredit, Befehle mit Ergebnis, Runner-Tunnel. Zustände zeigen die Meldungen, die Nutzer sehen.',
      features: ['PROTO-004', 'PROTO-005', 'PROTO-006', 'PROTO-007', 'PROTO-008', 'PROTO-009', 'PROTO-011', 'PROTO-015'],
      states: [
        { id: 'ok', title: 'Verbunden' },
        { id: 'reconnecting', title: 'Heartbeat verloren' },
        { id: 'resumed', title: 'Fortgesetzt ab seq' },
        { id: 'overflow', title: 'Backpressure' },
        { id: 'shutdown', title: 'Daemon startet neu' },
        { id: 'incompatible', title: 'Version inkompatibel' },
        { id: 'tunnel', title: 'Runner getrennt' },
      ],
      component: DiagConnection,
    },
    {
      id: 'diag-events',
      title: 'Event-Log einer Session',
      description:
        'Alle Ereignisse einer Session mit seq, Zeit, Akteur und Typ; flüchtige Ereignisse optional; Envelope im Detail, Schwärzen eines Inhalts und SSE-Befehl für Skripte.',
      features: ['PROTO-001', 'PROTO-002', 'PROTO-003', 'PROTO-012', 'PROTO-014', 'DATA-002', 'DATA-011', 'DATA-012'],
      states: [
        { id: 'default', title: 'Dauerhafte Ereignisse' },
        { id: 'transient', title: 'Mit flüchtigen' },
        { id: 'detail', title: 'Envelope' },
        { id: 'redact', title: 'Schwärzen' },
        { id: 'redacted', title: 'Geschwärzt' },
      ],
      component: DiagEvents,
    },
    {
      id: 'diag-storage',
      title: 'Speicher',
      description: 'Belegung von ~/.beton: Datenbank mit Schema und Sicherungen, abgeleitete Ansichten, Anhänge mit Aufräumen, Snapshots.',
      features: ['DATA-003', 'DATA-005', 'DATA-006', 'DATA-011', 'PROTO-011'],
      states: [
        { id: 'default', title: 'Übersicht' },
        { id: 'gc', title: 'Aufräumen läuft' },
        { id: 'migrating', title: 'Datenbank-Update' },
        { id: 'too-new', title: 'Datenbank zu neu' },
        { id: 'corrupt', title: 'Anhang beschädigt' },
      ],
      component: DiagStorage,
    },
    {
      id: 'diag-metrics',
      title: 'Metriken & Traces',
      description: 'Betreiber-Ansicht: Health-Endpunkte, Prometheus-Metriken und OpenTelemetry-Traces. Lokal ist alles aus; Export nur an selbst eingetragene Ziele.',
      features: ['OBS-003', 'OBS-004', 'OBS-009'],
      states: [
        { id: 'local', title: 'Lokal (nichts exportiert)' },
        { id: 'operator', title: 'Team-Server mit Export' },
        { id: 'not-ready', title: 'Nicht bereit' },
      ],
      component: DiagMetrics,
    },
  ],
  noUi: {
    'DATA-001': { reason: 'Relationales Datenmodell und Mandantentrennung sind Interna des Stores; sichtbar wird es nur indirekt über Sessions, Verbrauch und Inbox.', visibleIn: 'diag-storage' },
    'DATA-004': { reason: 'Postgres ist Backend des Team-Servers (ab M4); die Oberfläche ist identisch. Der Betreiber sieht es in Zustand und Readiness.', visibleIn: 'diag-metrics' },
    'DATA-007': { reason: 'S3-kompatibler Blob-Speicher für den Team-Server; für Nutzer unsichtbar, Anhänge verhalten sich gleich.', visibleIn: 'diag-storage' },
    'PROTO-010': { reason: 'REST-Konventionen (Cursor-Pagination, Idempotency-Key, ETag) betreffen API-Clients; in der UI sichtbar als „Ältere laden“ und Konfliktmeldungen.', visibleIn: 'diag-events' },
    'PROTO-013': { reason: 'Schema- und Typ-Generierung aus Rust ist Build-/CI-Infrastruktur ohne Oberfläche.' },
  },
}
