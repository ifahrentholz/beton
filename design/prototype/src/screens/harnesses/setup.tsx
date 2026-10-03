import type { ReactNode } from 'react'
import { KeyRound, RefreshCw, SquareTerminal } from 'lucide-react'
import { VoiceDot } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { Voice } from '@/mock/data'
import { Btn, Code, Mark, Note, PageHeader, Pill, SectionTitle } from './kit'
import { HarnessSettings } from './shell'

type SetupRow = {
  name: string
  voice: Voice
  kind: string
  path?: string
  version?: string
  status: ReactNode
  action?: ReactNode
  detail?: ReactNode
  features: string[]
  dim?: boolean
}

function SetupTable({ rows }: { rows: SetupRow[] }) {
  return (
    <div className="divide-y divide-border border-y border-border">
      {rows.map((r) => (
        <F key={r.name} id={r.features} badge="top-right">
          <div className={cn('grid grid-cols-[minmax(170px,1.1fr)_minmax(200px,1.4fr)_minmax(220px,1.6fr)_auto] items-center gap-4 px-1 py-2.5', r.dim && 'text-muted-foreground')}>
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-[13px] font-medium">
                <VoiceDot voice={r.voice} />
                {r.name}
              </div>
              <div className="pl-4 text-[11px] text-muted-foreground">{r.kind}</div>
            </div>
            <div className="type-narrow min-w-0 font-mono text-[12px]">
              {r.path ? (
                <>
                  <div className="truncate">{r.path}</div>
                  <div className="text-muted-foreground">{r.version}</div>
                </>
              ) : (
                <span className="text-muted-foreground">{r.version ?? 'nicht gefunden'}</span>
              )}
            </div>
            <div className="min-w-0 text-[13px]">{r.status}</div>
            <div className="flex justify-end gap-2">{r.action}</div>
          </div>
          {r.detail && <div className="px-1 pb-3">{r.detail}</div>}
        </F>
      ))}
    </div>
  )
}

function InstallOffer() {
  return (
    <div className="ml-4 rounded-md border border-border bg-card p-3 text-[13px]">
      <p>
        beton führt den offiziellen Installationsbefehl von OpenAI aus, <strong>erst nachdem du bestätigst</strong>. Dafür lädt npm das
        Paket aus dem Internet.
      </p>
      <pre className="mt-2 rounded-sm bg-sunken px-2 py-1.5 font-mono text-[12px]">npm install -g @openai/codex</pre>
      <p className="mt-2 text-muted-foreground">
        Danach meldest du dich in der Codex-CLI mit deinem ChatGPT-Konto an (<Code>codex login</Code>). beton sieht dein Konto nicht.
      </p>
      <div className="mt-3 flex items-center gap-2">
        <Btn variant="signal">Codex installieren</Btn>
        <Btn variant="outline">Befehl kopieren und selbst ausführen</Btn>
        <Btn variant="ghost">Abbrechen</Btn>
      </div>
    </div>
  )
}

export function SetupScreen({ state }: { state: string }) {
  const ready = state === 'ready'
  const expired = state === 'auth-expired'

  const claude: SetupRow = {
    name: 'Claude Code',
    voice: 'claude',
    kind: 'nativ · stream-json',
    path: '/opt/homebrew/bin/claude',
    version: '2.3.1 · kompatibel',
    features: ['HAR-016', 'HAR-015', 'HAR-003'],
    status: expired ? (
      <Mark kind="deny" label={<span>Abgemeldet – Login in der claude-CLI abgelaufen</span>} />
    ) : (
      <Mark kind="ok" label={<span>Über claude-CLI angemeldet · Claude Max</span>} />
    ),
    action: expired ? (
      <Btn variant="signal">
        <SquareTerminal className="size-3.5" /> In der claude-CLI anmelden
      </Btn>
    ) : (
      <Btn variant="ghost" size="sm">
        Status erneut abfragen
      </Btn>
    ),
    detail: expired ? (
      <div className="ml-4">
        <Note tone="deny" title="Claude Code kann keine Sessions starten">
          Die claude-CLI meldet einen abgelaufenen Login. beton öffnet ein Terminal mit <Code>claude auth login</Code>; die CLI übernimmt die
          Anmeldung vollständig im Browser. Laufende Sessions warten, bis du angemeldet bist.
        </Note>
      </div>
    ) : undefined,
  }

  const codex: SetupRow = ready
    ? {
        name: 'Codex',
        voice: 'codex',
        kind: 'nativ · app-server',
        path: '~/.local/share/pnpm/codex',
        version: '0.52.0 · kompatibel',
        features: ['HAR-016', 'HAR-003'],
        status: <Mark kind="ok" label="Über codex-CLI angemeldet · ChatGPT Pro" />,
        action: (
          <Btn variant="ghost" size="sm">
            Status erneut abfragen
          </Btn>
        ),
      }
    : {
        name: 'Codex',
        voice: 'codex',
        kind: 'nativ · app-server',
        version: 'nicht installiert',
        features: ['HAR-016'],
        status: <Mark kind="off" label="Nicht installiert" />,
        action:
          state === 'install' ? undefined : (
            <Btn variant="outline" size="sm">
              Installation anzeigen
            </Btn>
          ),
        detail: state === 'install' ? <InstallOffer /> : undefined,
      }

  const gemini: SetupRow = {
    name: 'Gemini CLI',
    voice: 'acp',
    kind: 'ACP-Preset · acp:gemini',
    path: '/opt/homebrew/bin/gemini',
    version: '0.21.0 · kompatibel',
    features: ['HAR-016', 'HAR-008'],
    status: ready ? <Mark kind="ok" label="Über gemini-CLI angemeldet · Google-Konto" /> : <Mark kind="unknown" label="Login-Status unbekannt (CLI meldet keinen Status)" />,
    action: ready ? undefined : (
      <Btn variant="outline" size="sm">
        <SquareTerminal className="size-3.5" /> gemini öffnen
      </Btn>
    ),
  }

  const ollama: SetupRow = {
    name: 'Ollama',
    voice: 'direct',
    kind: 'Lokaler Modell-Server',
    path: 'http://127.0.0.1:11434',
    version: '3 Modelle · qwen3-coder:30b …',
    features: ['HAR-016', 'HAR-011'],
    status: ready ? <Mark kind="ok" label="Als Anbieter eingerichtet · kein Schlüssel nötig" /> : <Mark kind="ok" label="Läuft auf diesem Rechner" />,
    action: ready ? undefined : (
      <Btn variant="outline" size="sm">
        Als Anbieter hinzufügen
      </Btn>
    ),
  }

  const rest: SetupRow[] = [
    {
      name: 'LM Studio',
      voice: 'direct',
      kind: 'Lokaler Modell-Server',
      version: 'kein Server auf :1234',
      features: ['HAR-016'],
      status: <Mark kind="off" label="Nicht gefunden" />,
      dim: true,
    },
    {
      name: 'Goose',
      voice: 'acp',
      kind: 'ACP-Preset · acp:goose',
      version: 'nicht im PATH',
      features: ['HAR-008'],
      status: <Mark kind="off" label="Preset inaktiv, bis goose installiert ist" />,
      dim: true,
    },
    {
      name: 'Qwen Code',
      voice: 'acp',
      kind: 'ACP-Preset · acp:qwen',
      version: 'nicht im PATH',
      features: ['HAR-008'],
      status: <Mark kind="off" label="Preset inaktiv, bis qwen installiert ist" />,
      dim: true,
    },
  ]

  return (
    <HarnessSettings active="setup">
      <PageHeader
        title="Einrichtung"
        sub="Welche Coding-Agents auf diesem Rechner nutzbar sind. Ergebnis von beton setup, zuletzt geprüft vor 2 Min."
        actions={
          <Btn variant="outline">
            <RefreshCw className="size-3.5" /> Erneut prüfen
          </Btn>
        }
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="grid grid-cols-[minmax(0,1fr)_280px] gap-8 px-6 py-5">
          <div className="min-w-0 space-y-6">
            {!ready && !expired && (
              <F id="HAR-016">
                <p className="text-[13px]">
                  <strong>1 von 4 Harnesses ist startklar.</strong> Für den Anfang reicht Claude Code mit deinem Claude-Max-Abo. Codex und Gemini
                  kannst du später ergänzen.
                </p>
              </F>
            )}
            <section>
              <SectionTitle aside="Pfad und Version aus <bin> --version">Coding-Agents (CLIs)</SectionTitle>
              <SetupTable rows={[claude, codex, gemini]} />
            </section>
            <section>
              <SectionTitle aside="Probe auf 127.0.0.1, 500 ms">Lokale Modelle &amp; weitere ACP-Agents</SectionTitle>
              <SetupTable rows={[ollama, ...rest]} />
            </section>
            <F id={['HAR-016', 'HAR-015']}>
              <section>
                <SectionTitle aside="optional">API-Schlüssel in der Umgebung</SectionTitle>
                <div className="flex items-center gap-4 border-y border-border px-1 py-2.5 text-[13px]">
                  <KeyRound className="size-4 text-muted-foreground" />
                  <span className="font-mono text-[12px]">OPENROUTER_API_KEY</span>
                  <span className="font-mono text-[12px] text-muted-foreground">sk-or-…3f9a</span>
                  <span className="text-muted-foreground">Gefunden. Nur nötig, wenn du OpenRouter als Direkt-API nutzen willst.</span>
                  <Btn variant="ghost" size="sm" className="ml-auto">
                    Als Anbieter einrichten
                  </Btn>
                </div>
                <p className="mt-2 text-[12px] text-muted-foreground">
                  Für Claude Code und Codex mit Abo brauchst du keinen Schlüssel. Ist <Code>ANTHROPIC_API_KEY</Code> gesetzt, entfernt beton ihn aus
                  der Umgebung der claude-CLI, damit dein Abo statt API-Abrechnung genutzt wird.
                </p>
              </section>
            </F>
          </div>

          <F id="HAR-015" as="section" className="space-y-4 text-[13px]">
            <div>
              <SectionTitle>So funktioniert die Anmeldung</SectionTitle>
              <ol className="space-y-2.5">
                {[
                  ['Du meldest dich in der offiziellen CLI an', <><Code>claude auth login</Code>, <Code>codex login</Code> oder beim ersten Start von <Code>gemini</Code>.</>],
                  ['beton fragt nur den Status ab', <>über <Code>claude auth status</Code> bzw. <Code>codex login status</Code>, nie über Token-Dateien.</>],
                  ['Dein Abo bleibt bei der CLI', 'beton speichert keine Tokens und hat keinen eigenen Login. Die CLI spricht direkt mit Anthropic, OpenAI oder Google.'],
                ].map(([t, d], i) => (
                  <li key={i} className="flex gap-2.5">
                    <span className="flex size-5 shrink-0 items-center justify-center rounded-full border border-border text-[11px] tabular-nums">{i + 1}</span>
                    <div>
                      <div className="font-medium">{t}</div>
                      <div className="text-muted-foreground">{d}</div>
                    </div>
                  </li>
                ))}
              </ol>
            </div>
            <div className="border-t border-border pt-4">
              <SectionTitle>Netzwerk</SectionTitle>
              <p className="text-muted-foreground">
                beton läuft vollständig auf diesem Rechner. Ins Internet gehen nur die CLIs zu ihren Modell-Anbietern. Installationen laden erst
                nach deiner Bestätigung etwas herunter.
              </p>
            </div>
            <div className="border-t border-border pt-4">
              <Pill>Auch im Terminal: beton setup</Pill>
            </div>
          </F>
        </div>
      </div>
    </HarnessSettings>
  )
}
