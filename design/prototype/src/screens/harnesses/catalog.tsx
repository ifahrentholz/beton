import { X } from 'lucide-react'
import { VoiceDot } from '@/app/harness'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { Voice } from '@/mock/data'
import { Btn, Code, CodeBlock, Mark, Note, PageHeader, Pill, SectionTitle, Segmented, drawerClass } from './kit'
import { HarnessSettings } from './shell'

/** Zellwert der Matrix: Glyphe + Text (nie nur Farbe). */
type Cell = { v: string; level: 'full' | 'part' | 'none'; title?: string }
const full = (v: string, title?: string): Cell => ({ v, level: 'full', title })
const part = (v: string, title?: string): Cell => ({ v, level: 'part', title })
const none: Cell = { v: 'nicht möglich', level: 'none' }

const COLS = [
  { id: 'transport', label: 'Transport', feature: 'HAR-001' },
  { id: 'approval', label: 'Freigaben', feature: 'HAR-005' },
  { id: 'gate', label: 'Tool-Prüfung', feature: 'HAR-002' },
  { id: 'model', label: 'Modellwechsel', feature: 'HAR-017' },
  { id: 'resume', label: 'Fortsetzen', feature: 'HAR-020' },
  { id: 'fork', label: 'Fork-History', feature: 'HAR-019' },
  { id: 'usage', label: 'Verbrauch', feature: 'HAR-021' },
  { id: 'compact', label: 'Compaction', feature: 'HAR-022' },
] as const
type ColId = (typeof COLS)[number]['id']

type CatalogRow = {
  id: string
  name: string
  voice: Voice
  version: string
  auth: string
  cells?: Record<ColId, Cell>
  features: string[]
  off?: string
  incompatible?: string
}

const nativeRows: CatalogRow[] = [
  {
    id: 'claude',
    name: 'Claude Code',
    voice: 'claude',
    version: '2.3.1',
    auth: 'Abo über claude-CLI',
    features: ['HAR-004', 'HAR-005'],
    cells: {
      transport: full('nativ', 'stream-json über stdio'),
      approval: full('Vendor-Anfrage', 'control_request can_use_tool'),
      gate: full('jeder Call', 'ab M2 zusätzlich PreToolUse-Hook'),
      model: full('live'),
      resume: full('warm', '--resume <id>'),
      fork: full('Rebuild', '--resume --fork-session'),
      usage: full('Tokens + Kontingent'),
      compact: full('nativ'),
    },
  },
  {
    id: 'codex',
    name: 'Codex',
    voice: 'codex',
    version: '0.52.0',
    auth: 'Abo über codex-CLI',
    features: ['HAR-006'],
    cells: {
      transport: full('nativ', 'app-server, JSON-RPC 2.0'),
      approval: full('Vendor-Anfrage', 'requestApproval → PolicyGate'),
      gate: part('Shell & Schreiben', 'approval_policy: untrusted'),
      model: full('live ab nächstem Turn'),
      resume: full('warm', 'thread/resume'),
      fork: full('Rebuild'),
      usage: full('Tokens + Kontingent'),
      compact: full('nativ'),
    },
  },
  {
    id: 'acp:gemini',
    name: 'Gemini CLI',
    voice: 'acp',
    version: '0.21.0',
    auth: 'Google-Login über CLI',
    features: ['HAR-007'],
    cells: {
      transport: full('ACP'),
      approval: full('ACP-Anfrage', 'session/request_permission'),
      gate: full('jeder Call', 'fs/* und terminal/* führt beton selbst aus'),
      model: part('Neustart mit Resume'),
      resume: part('kalt', 'kein session/load'),
      fork: part('Präambel'),
      usage: part('Tokens'),
      compact: none,
    },
  },
  {
    id: 'acp:goose',
    name: 'Goose',
    voice: 'acp',
    version: '—',
    auth: '—',
    features: ['HAR-008'],
    off: 'Preset · goose nicht im PATH',
  },
  {
    id: 'direct:ollama',
    name: 'Ollama (lokal)',
    voice: 'direct',
    version: 'qwen3-coder:30b',
    auth: 'kein Schlüssel',
    features: ['HAR-010'],
    cells: {
      transport: full('Direkt-API', 'Agent-Loop in beton'),
      approval: full('durch beton'),
      gate: full('jeder Call + Modell-Request'),
      model: full('live'),
      resume: full('warm', 'aus dem Event-Log'),
      fork: full('Rebuild'),
      usage: part('Tokens · 0 €'),
      compact: full('durch beton'),
    },
  },
  {
    id: 'direct:openrouter',
    name: 'OpenRouter',
    voice: 'direct',
    version: 'qwen/qwen3-coder',
    auth: 'API-Schlüssel (optional)',
    features: ['HAR-010', 'HAR-011'],
    cells: {
      transport: full('Direkt-API'),
      approval: full('durch beton'),
      gate: full('jeder Call + Modell-Request'),
      model: full('live'),
      resume: full('warm'),
      fork: full('Rebuild'),
      usage: full('Tokens + Kosten'),
      compact: full('durch beton'),
    },
  },
]

const tuiRows: CatalogRow[] = [
  {
    id: 'claude',
    name: 'Claude Code · TUI',
    voice: 'claude',
    version: '2.3.1',
    auth: 'Abo über claude-CLI',
    features: ['HAR-012', 'HAR-013'],
    cells: {
      transport: full('PTY', 'Original-TUI im eigenen Multiplexer'),
      approval: full('Hooks', 'session-eigene --settings-Datei'),
      gate: full('jeder Call', 'PreToolUse'),
      model: part('Slash-Befehl', '/model wird eingegeben'),
      resume: full('warm'),
      fork: full('Rebuild'),
      usage: part('Tokens', 'aus dem Transcript'),
      compact: full('nativ'),
    },
  },
  {
    id: 'codex',
    name: 'Codex · TUI',
    voice: 'codex',
    version: '0.52.0',
    auth: 'Abo über codex-CLI',
    features: ['HAR-012', 'HAR-014'],
    cells: {
      transport: full('PTY'),
      approval: part('Bildschirm-Spiegelung', 'Strategie B: Dialog erkennen, per Tastendruck beantworten'),
      gate: part('nur Freigaben', 'Reads nur beobachtet; Sandbox ist der Backstop'),
      model: none,
      resume: full('warm'),
      fork: full('Rebuild'),
      usage: part('Tokens'),
      compact: full('nativ'),
    },
  },
  { id: 'acp:gemini', name: 'Gemini CLI', voice: 'acp', version: '0.21.0', auth: '—', features: ['HAR-012'], off: 'Kein TUI-Modus (nur ACP)' },
  { id: 'direct:ollama', name: 'Ollama (lokal)', voice: 'direct', version: '—', auth: '—', features: ['HAR-012'], off: 'Kein TUI-Modus' },
]

function CellView({ c }: { c: Cell }) {
  return (
    <span title={c.title} className={cn('inline-flex items-baseline gap-1.5', c.level === 'none' && 'text-muted-foreground')}>
      <span aria-hidden className="w-2.5 shrink-0 text-center text-[10px]">
        {c.level === 'full' ? '●' : c.level === 'part' ? '◐' : '–'}
      </span>
      {c.v}
    </span>
  )
}

function Matrix({ rows, selected }: { rows: CatalogRow[]; selected?: string }) {
  return (
    <F id={['HAR-002', 'HAR-001']} className="overflow-x-auto">
      <table className="type-narrow w-full border-collapse text-[12.5px]">
        <thead>
          <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
            <th className="py-2 pr-3 font-normal">Harness</th>
            {COLS.map((c) => (
              <th key={c.id} className="px-2 py-2 font-normal whitespace-nowrap">
                <F id={c.feature} as="span" badge="top-left">
                  {c.label}
                </F>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.id + r.name} className={cn('border-b border-border align-top', selected === r.id && 'bg-accent')}>
              <td className="py-2 pr-3 whitespace-nowrap">
                <F id={r.features} as="span" badge="bottom-left">
                  <span className="flex items-center gap-1.5 font-medium">
                    <VoiceDot voice={r.voice} />
                    {r.name}
                  </span>
                  <span className="block pl-3.5 font-mono text-[11px] text-muted-foreground">
                    {r.id} · {r.version}
                  </span>
                  {!r.off && !r.incompatible && <span className="block pl-3.5 text-[11px] text-muted-foreground">{r.auth}</span>}
                </F>
              </td>
              {r.incompatible ? (
                <td colSpan={COLS.length} className="px-2 py-2">
                  <F id={['HAR-003', 'HAR-006']} as="span">
                    <Mark kind="deny" label={<span className="font-medium">Nicht kompatibel</span>} />
                    <span className="ml-2 text-muted-foreground">{r.incompatible}</span>
                  </F>
                </td>
              ) : r.off ? (
                <td colSpan={COLS.length} className="px-2 py-2 text-muted-foreground">
                  <Mark kind="off" label={r.off} />
                </td>
              ) : (
                <>
                  {COLS.map((c) => (
                    <td key={c.id} className="px-2 py-2">
                      <CellView c={r.cells![c.id]} />
                    </td>
                  ))}
                </>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </F>
  )
}

function Detail() {
  return (
    <aside className={cn(drawerClass, 'w-[400px]')}>
      <div className="flex items-center gap-2 border-b border-border px-4 py-3">
        <VoiceDot voice="claude" />
        <span className="font-semibold">Claude Code</span>
        <Pill>nativ</Pill>
        <Pill>TUI</Pill>
        <button className="ml-auto rounded-md p-1 text-muted-foreground hover:bg-accent" aria-label="Details schließen">
          <X className="size-4" />
        </button>
      </div>
      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-4 py-4 text-[13px]">
        <F id="HAR-003">
          <SectionTitle aside="erste Quelle gewinnt">Welche claude-CLI startet beton?</SectionTitle>
          <ol className="divide-y divide-border border-y border-border">
            {[
              ['BETON_CLAUDE_PATH', 'nicht gesetzt', false],
              ['.beton/config.yaml (Projekt)', 'nicht gesetzt', false],
              ['~/.beton/config.yaml', '/opt/homebrew/bin/claude', true],
              ['PATH', '/usr/local/bin/claude (übergangen)', false],
            ].map(([src, val, used]) => (
              <li key={String(src)} className="flex items-center gap-2 py-1.5">
                <span className="w-4">{used ? <Mark kind="ok" /> : null}</span>
                <span className={cn('w-44 shrink-0', !used && 'text-muted-foreground')}>{src}</span>
                <span className={cn('min-w-0 truncate font-mono text-[11.5px]', !used && 'text-muted-foreground')}>{val}</span>
              </li>
            ))}
          </ol>
          <p className="mt-1.5 text-[12px] text-muted-foreground">Version 2.3.1, getestet ab 2.0.0 · geprüft per --version, Ergebnis zwischengespeichert</p>
        </F>

        <F id="HAR-015">
          <SectionTitle>Anmeldung</SectionTitle>
          <div className="space-y-1.5">
            <label className="flex gap-2 rounded-md border border-foreground/40 bg-card p-2.5">
              <span className="mt-0.5 size-3.5 shrink-0 rounded-full border-4 border-foreground" />
              <span>
                <span className="font-medium">Abo über die claude-CLI</span>
                <span className="block text-[12px] text-muted-foreground">
                  Claude Max · die CLI meldet sich selbst an. beton entfernt <Code>ANTHROPIC_API_KEY</Code> aus ihrer Umgebung.
                </span>
              </span>
            </label>
            <label className="flex gap-2 rounded-md border border-border p-2.5 text-muted-foreground">
              <span className="mt-0.5 size-3.5 shrink-0 rounded-full border border-muted-foreground" />
              <span>
                <span className="font-medium text-foreground">API-Schlüssel</span>
                <span className="block text-[12px]">Optional, Abrechnung über die Anthropic-API. Der Schlüssel liegt im Schlüsselbund; die CLI sieht nur einen Platzhalter.</span>
              </span>
            </label>
          </div>
        </F>

        <F id="HAR-009">
          <div className="flex items-start gap-3">
            <div className="flex-1">
              <div className="font-medium">System-Tools von beton bereitstellen</div>
              <p className="text-[12px] text-muted-foreground">
                Bindet den MCP-Server <Code>beton</Code> ein (Sub-Agents starten, Policy abfragen, Inbox lesen). Alle MCP-Server laufen über den
                beton-Proxy und werden so geprüft.
              </p>
            </div>
            <Switch defaultChecked aria-label="System-Tools bereitstellen" />
          </div>
        </F>

        <F id={['HAR-012', 'HAR-013']}>
          <SectionTitle>Original-TUI</SectionTitle>
          <p className="text-muted-foreground">
            Im TUI-Modus läuft die echte Claude-Oberfläche im Terminal-Tab. Policies greifen über Hooks in einer session-eigenen Settings-Datei;
            deine <Code>~/.claude/settings.json</Code> bleibt unverändert. Freigaben warten hier höchstens 1 Std., danach wird abgelehnt.
          </p>
        </F>

        <F id="HAR-002">
          <SectionTitle aside="GET /v1/harnesses">Fähigkeiten (Rohdaten)</SectionTitle>
          <CodeBlock
            lines={[
              'id: claude',
              'mode: native',
              'version_range: ">=2.0.0"',
              'auth_sources: [vendor_cli, api_key]',
              'approval: native_request',
              'tool_call_gate: full',
              'model_switch: live',
              'effort_switch: live',
              'resume: warm',
              'fork_history: rebuild',
              'usage_reporting: tokens_and_cost',
              'compaction: native',
              'instructions_delivery: append_system_prompt',
            ]}
          />
        </F>
      </div>
    </aside>
  )
}

export function CatalogScreen({ state }: { state: string }) {
  const tui = state === 'tui'
  let rows = tui ? tuiRows : nativeRows
  if (state === 'incompatible') {
    rows = nativeRows.map((r) =>
      r.id === 'codex'
        ? { ...r, version: '0.38.0', incompatible: 'Installiert ist 0.38.0, getestet ab 0.45.0. Codex-Sessions starten nicht. Aktualisiere mit npm install -g @openai/codex.' }
        : r,
    )
  }
  return (
    <HarnessSettings active="catalog">
      <div className="relative flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <PageHeader
            title="Harness-Katalog"
            sub="Was jeder Harness auf diesem Rechner kann. Aktionen, die ein Harness nicht unterstützt, sind in Sessions ausgegraut."
            actions={<Segmented items={['Strukturiert', 'Original-TUI']} active={tui ? 'Original-TUI' : 'Strukturiert'} />}
          />
          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-6 py-4">
            {state === 'incompatible' && (
              <F id={['HAR-002', 'HAR-003']}>
                <Note
                  tone="deny"
                  title="Codex 0.38.0 ist zu alt für beton"
                  action={
                    <>
                      <Btn variant="outline" size="sm">
                        Befehl kopieren
                      </Btn>
                    </>
                  }
                >
                  Das app-server-Protokoll dieser Version passt nicht zu beton (erwartet ab 0.45.0). Aktualisiere Codex mit{' '}
                  <Code>npm install -g @openai/codex</Code> und prüfe dann erneut.
                </Note>
              </F>
            )}
            {tui && (
              <F id={['HAR-012', 'HAR-014']}>
                <Note title="Original-TUI: gleiche Policies, anderer Weg">
                  Im TUI-Modus siehst du die echte Oberfläche von Claude Code bzw. Codex. Codex 0.52 bietet keine Hooks; beton erkennt den
                  Freigabe-Dialog im Terminal und spiegelt ihn als Karte. Lese-Zugriffe sind dabei nur beobachtbar. Verlangt eine Policy mehr,
                  startet die Session nicht.
                </Note>
              </F>
            )}
            <Matrix rows={rows} selected={state === 'detail' ? 'claude' : undefined} />
            <div className="flex flex-wrap gap-x-6 gap-y-1 text-[11px] text-muted-foreground">
              <span>● voll unterstützt</span>
              <span>◐ eingeschränkt (Tooltip nennt den Grund)</span>
              <span>– nicht möglich, Aktion ist deaktiviert</span>
              <F id="HAR-026" as="span" badge="top-right">
                <span>Der Test-Harness fake erscheint nur in Entwickler-Builds (--dev).</span>
              </F>
            </div>
          </div>
        </div>
        {state === 'detail' && <Detail />}
      </div>
    </HarnessSettings>
  )
}
