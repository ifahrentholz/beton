import { RefreshCw } from 'lucide-react'
import { VoiceDot } from '@/app/harness'
import { Checkbox } from '@/components/ui/checkbox'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { Voice } from '@/mock/data'
import { Btn, Code, Mark, PageHeader, Pill, Row, SectionTitle } from './kit'
import { HarnessSettings } from './shell'

type Found = { id: string; title: string; cwd: string; updated: string; model: string; size: string; imported?: boolean }

const CLAUDE: Found[] = [
  { id: 'c1', title: 'Warum ist der Checkout-Button auf Safari grau?', cwd: '~/code/shop-frontend', updated: 'heute, 09:41', model: 'claude-opus-5-5', size: '1,2 MB' },
  { id: 'c2', title: 'Migration auf Vite 8', cwd: '~/code/shop-frontend', updated: 'gestern', model: 'claude-sonnet-5-5', size: '3,4 MB' },
  { id: 'c3', title: 'GitHub-Action für Preview-Deploys', cwd: '~/code/infra', updated: '28.09.', model: 'claude-opus-5-5', size: '640 KB', imported: true },
  { id: 'c4', title: 'Zod-Schemas aus OpenAPI generieren', cwd: '~/code/shop-frontend', updated: '24.09.', model: 'claude-sonnet-5-5', size: '890 KB' },
]
const CODEX: Found[] = [
  { id: 'x1', title: 'Flaky payment_spec stabilisieren', cwd: '~/code/shop-frontend', updated: 'heute, 08:15', model: 'gpt-5.3-codex', size: '2,1 MB' },
  { id: 'x2', title: 'Terraform-Module aufräumen', cwd: '~/code/infra', updated: '30.09.', model: 'gpt-5.3-codex', size: '1,7 MB' },
]

function SourceList({ name, voice, path, items, selected, feature }: { name: string; voice: Voice; path: string; items: Found[]; selected?: string; feature: string }) {
  return (
    <F id={feature} as="section">
      <SectionTitle aside={<span className="font-mono">{path}</span>}>
        <span className="inline-flex items-center gap-2">
          <VoiceDot voice={voice} />
          {name} · {items.length} gefunden
        </span>
      </SectionTitle>
      <div className="divide-y divide-border border-y border-border">
        {items.map((f) => (
          <label key={f.id} className={cn('flex items-center gap-3 px-1 py-2 text-[13px]', selected === f.id && 'bg-accent', f.imported && 'text-muted-foreground')}>
            <Checkbox checked={selected === f.id} disabled={f.imported} aria-label={`${f.title} auswählen`} />
            <span className="min-w-0 flex-1">
              <span className="block truncate">{f.title}</span>
              <span className="type-narrow block font-mono text-[11px] text-muted-foreground">
                {f.cwd} · {f.model} · {f.size}
              </span>
            </span>
            {f.imported ? <Pill>bereits importiert</Pill> : <span className="text-[12px] text-muted-foreground">{f.updated}</span>}
          </label>
        ))}
      </div>
    </F>
  )
}

function ParseResult({ codex }: { codex?: boolean }) {
  return (
    <F id={codex ? ['HAR-024'] : ['HAR-023', 'HAR-019']} as="section" className="flex w-[400px] shrink-0 flex-col border-l border-border">
      <div className="border-b border-border px-5 py-3">
        <div className="text-[11px] text-muted-foreground">Vorschau vor dem Import</div>
        <h3 className="text-[15px] font-semibold">{codex ? 'Flaky payment_spec stabilisieren' : 'Warum ist der Checkout-Button auf Safari grau?'}</h3>
      </div>
      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4 text-[13px]">
        <dl className="divide-y divide-border">
          <Row label="Datei">
            <span className="font-mono text-[11.5px] break-all">
              {codex ? '~/.codex/sessions/2026/10/03/rollout-2026-10-03T08-15-22-a41f.jsonl' : '~/.claude/projects/-Users-ingo-code-shop-frontend/7c9e…b2.jsonl'}
            </span>
          </Row>
          <Row label="Projekt">
            <span className="font-mono text-[12px]">~/code/shop-frontend</span> → shop-frontend
          </Row>
          <Row label="Modell">
            <span className="font-mono text-[12px]">{codex ? 'gpt-5.3-codex' : 'claude-opus-5-5'}</span>
          </Row>
          <Row label="Inhalt">{codex ? '18 Turns · 96 Tool-Calls · 11 Patches' : '41 Turns · 212 Tool-Calls · 3 Sub-Agent-Läufe'}</Row>
          {codex ? (
            <Row label="Änderungen">7 Dateien aus apply_patch, als Diffs sichtbar</Row>
          ) : (
            <Row label="Sub-Agents">3 Task-Läufe der CLI, verschachtelt unter dem auslösenden Tool-Call</Row>
          )}
        </dl>

        <div>
          <SectionTitle>Hinweise des Parsers</SectionTitle>
          <ul className="space-y-1.5">
            {codex ? (
              <>
                <li>
                  <Mark kind="ok" label="Format erkannt (Codex 0.48 – 0.52)" />
                </li>
                <li>
                  <Mark kind="ok" label={<>Gesucht in <Code>~/.codex</Code> (CODEX_HOME nicht gesetzt)</>} />
                </li>
                <li>
                  <Mark kind="skip" label="2 Token-Meldungen ohne bekanntes Feld, als Rohdaten gespeichert" />
                </li>
              </>
            ) : (
              <>
                <li>
                  <Mark kind="ok" label="Format erkannt (claude-CLI 2.2 – 2.3)" />
                </li>
                <li>
                  <Mark kind="skip" label="Zeile 412 beschädigt und übersprungen; der Rest ist vollständig" />
                </li>
                <li>
                  <Mark kind="skip" label="2 verworfene Zweige (Rewind) nicht übernommen, nur der aktive Verlauf" />
                </li>
                <li>
                  <Mark kind="skip" label="3 unbekannte Einträge als Rohdaten gespeichert" />
                </li>
              </>
            )}
          </ul>
        </div>

        {!codex && (
          <div>
            <SectionTitle>Danach weiterarbeiten</SectionTitle>
            <p className="text-muted-foreground">
              Claude Code kann die importierte Session mit vollem Verlauf fortsetzen. Wechselst du per Fork auf einen anderen Harness, bekommt er ein
              Übergabe-Dokument.
            </p>
          </div>
        )}
        <p className="text-[12px] text-muted-foreground">Gelesen wurden nur Session-Dateien. Anmeldedaten der CLIs fasst beton nicht an.</p>
      </div>
      <div className="flex gap-2 border-t border-border px-5 py-3">
        <Btn variant="primary">1 Session importieren</Btn>
        <Btn variant="ghost">Rohdaten ansehen</Btn>
      </div>
    </F>
  )
}

export function ImportScreen({ state }: { state: string }) {
  const empty = state === 'empty'
  return (
    <HarnessSettings active="import">
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <PageHeader
            title="Transcripts importieren"
            sub="Sessions, die du direkt in Claude Code oder Codex geführt hast, nach beton übernehmen – mit Verlauf, Tool-Calls und Änderungen."
            actions={
              <Btn variant="outline">
                <RefreshCw className="size-3.5" /> Erneut suchen
              </Btn>
            }
          />
          <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-6 py-4">
            {empty ? (
              <F id={['HAR-023', 'HAR-024']} className="concrete-grain flex flex-col items-center gap-2 rounded-md border border-dashed border-border px-6 py-16 text-center">
                <p className="type-wide text-base font-[700]">Keine fremden Sessions gefunden</p>
                <p className="max-w-md text-[13px] text-muted-foreground">
                  Gesucht in <Code>~/.claude/projects</Code> und <Code>~/.codex/sessions</Code>. Wenn Codex woanders speichert, setze{' '}
                  <Code>CODEX_HOME</Code> und suche erneut.
                </p>
              </F>
            ) : (
              <>
                <SourceList name="Claude Code" voice="claude" path="~/.claude/projects" items={CLAUDE} selected={state === 'parsed' ? 'c1' : undefined} feature="HAR-023" />
                <SourceList name="Codex" voice="codex" path="~/.codex/sessions" items={CODEX} selected={state === 'codex' ? 'x1' : undefined} feature="HAR-024" />
              </>
            )}
          </div>
        </div>
        {state === 'parsed' && <ParseResult />}
        {state === 'codex' && <ParseResult codex />}
      </div>
    </HarnessSettings>
  )
}
