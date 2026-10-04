import { Plus } from 'lucide-react'
import { VoiceDot } from '@/app/harness'
import { Switch } from '@/components/ui/switch'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Code, CodeBlock, FakeInput, FieldLabel, Mark, Note, PageHeader, Pill, SectionTitle, Segmented } from '@/app/kit/harnesses'
import { drawerClass } from './kit'
import { HarnessSettings } from './shell'

type AcpAgent = { slug: string; name: string; command: string; source: string; preset?: boolean; active: boolean; note: string; invalid?: boolean }

const AGENTS: AcpAgent[] = [
  { slug: 'gemini', name: 'Gemini CLI', command: 'gemini --experimental-acp', source: 'Preset', preset: true, active: true, note: 'Google-Login über gemini-CLI · 2 Modelle' },
  { slug: 'goose', name: 'Goose', command: 'goose acp', source: 'Preset', preset: true, active: false, note: 'goose nicht im PATH gefunden' },
  { slug: 'qwen', name: 'Qwen Code', command: 'qwen --acp', source: 'Preset', preset: true, active: false, note: 'qwen nicht im PATH gefunden' },
  { slug: 'review-bot', name: 'review-bot', command: './tools/review-bot acp', source: '.beton/config.yaml', active: true, note: 'Eigener Agent im Projekt shop-frontend' },
]

function AgentList({ invalid }: { invalid?: boolean }) {
  const list: AcpAgent[] = invalid
    ? [...AGENTS.slice(0, 3), { slug: 'Review_Bot', name: 'Review_Bot', command: '—', source: '~/.beton/config.yaml:14', active: false, note: 'Übersprungen: ungültiger Slug', invalid: true }]
    : AGENTS
  return (
    <F id={['HAR-008', 'HAR-007']} className="divide-y divide-border border-y border-border">
      {list.map((a) => (
        <div key={a.slug} className={cn('grid grid-cols-[minmax(160px,1fr)_minmax(180px,1.2fr)_minmax(160px,1fr)_auto] items-center gap-4 px-1 py-2.5 text-[13px]', !a.active && !a.invalid && 'text-muted-foreground')}>
          <div>
            <div className="flex items-center gap-2 font-medium">
              <VoiceDot voice="acp" />
              {a.name}
            </div>
            <div className="pl-4 font-mono text-[11px] text-muted-foreground">acp:{a.slug}</div>
          </div>
          <span className="font-mono text-[12px]">{a.command}</span>
          <div>
            {a.invalid ? <Mark kind="deny" label={a.note} /> : a.active ? <Mark kind="ok" label={a.note} /> : <Mark kind="off" label={a.note} />}
          </div>
          <div className="flex items-center gap-2">
            <Pill>{a.source}</Pill>
            {a.preset && a.active && (
              <Btn variant="ghost" size="sm">
                Anpassen
              </Btn>
            )}
          </div>
        </div>
      ))}
    </F>
  )
}

function AddPanel() {
  return (
    <F id={['HAR-008', 'HAR-009', 'HAR-007']} as="section" className={cn(drawerClass, 'w-[440px]')}>
      <div className="border-b border-border px-5 py-3">
        <h3 className="type-wide text-[15px] font-[700]">ACP-Agent hinzufügen</h3>
        <p className="text-[12px] text-muted-foreground">Gleiches Ergebnis wie beton setup acp add im Terminal.</p>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
        <div>
          <FieldLabel hint="a–z, 0–9, Bindestrich">Name (Slug)</FieldLabel>
          <FakeInput value="pair-agent" mono />
          <p className="mt-1 text-[11px] text-muted-foreground">
            Erscheint als Harness <Code>acp:pair-agent</Code>.
          </p>
        </div>
        <div>
          <FieldLabel>Befehl</FieldLabel>
          <FakeInput value="./tools/pair-agent" mono />
        </div>
        <div>
          <FieldLabel>Argumente</FieldLabel>
          <FakeInput value="acp --stdio" mono />
        </div>
        <div>
          <FieldLabel hint="optional">Umgebungsvariablen durchreichen</FieldLabel>
          <FakeInput placeholder="z. B. PAIR_AGENT_HOME" mono />
          <p className="mt-1 text-[11px] text-muted-foreground">Nur genannte Variablen erreichen den Agent; alles andere filtert die Sandbox.</p>
        </div>
        <div>
          <FieldLabel hint="optional, für den Modell-Picker">Modelle</FieldLabel>
          <FakeInput placeholder="vom Agent abfragen" mono />
        </div>
        <div>
          <FieldLabel>Speichern in</FieldLabel>
          <Segmented items={['Für mich (~/.beton)', 'Für dieses Projekt']} active="Für mich (~/.beton)" />
        </div>
        <div className="flex items-start gap-3">
          <div className="flex-1 text-[13px]">
            <div className="font-medium">System-Tools von beton bereitstellen</div>
            <p className="text-[12px] text-muted-foreground">Sub-Agents, Policy-Abfrage und Inbox über den MCP-Server beton.</p>
          </div>
          <Switch defaultChecked aria-label="System-Tools bereitstellen" />
        </div>

        <div>
          <SectionTitle aside="Verbindung getestet vor 4 s">Testlauf</SectionTitle>
          <ul className="space-y-1 text-[12.5px]">
            <li>
              <Mark kind="ok" label={<>initialize · ACP-Protokoll 1</>} />
            </li>
            <li>
              <Mark kind="ok" label="Freigabe-Anfragen (session/request_permission)" />
            </li>
            <li>
              <Mark kind="ok" label="Modellwechsel (session/set_model)" />
            </li>
            <li>
              <Mark kind="off" label={<>Kein session/load: Fortsetzen nur kalt, Forks mit Präambel</>} />
            </li>
          </ul>
        </div>

        <CodeBlock
          title="~/.beton/config.yaml (Vorschau)"
          lines={[
            'harnesses:',
            '  acp:',
            '    agents:',
            { text: '      pair-agent:', mark: 'add' },
            { text: '        command: ./tools/pair-agent', mark: 'add' },
            { text: '        args: [acp, --stdio]', mark: 'add' },
            { text: '        mcp_bridge: true', mark: 'add' },
          ]}
        />
      </div>
      <div className="flex gap-2 border-t border-border px-5 py-3">
        <Btn variant="primary">Agent speichern</Btn>
        <Btn variant="outline">Erneut testen</Btn>
        <Btn variant="ghost" className="ml-auto">
          Abbrechen
        </Btn>
      </div>
    </F>
  )
}

export function AcpScreen({ state }: { state: string }) {
  return (
    <HarnessSettings active="acp">
      <div className="relative flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <PageHeader
            title="ACP-Agents"
            sub="Jeder Agent, der das Agent Client Protocol spricht, läuft in beton als Harness – mit denselben Policies, Freigaben und Sandbox."
            actions={
              state !== 'custom' && (
                <Btn variant="primary">
                  <Plus className="size-3.5" /> ACP-Agent hinzufügen
                </Btn>
              )
            }
          />
          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-4">
            {state === 'invalid' && (
              <F id="HAR-008">
                <Note tone="deny" title="Ein Eintrag in ~/.beton/config.yaml wurde übersprungen">
                  Zeile 14: Der Name „Review_Bot“ enthält unerlaubte Zeichen. Erlaubt sind a–z, 0–9 und Bindestrich, z. B. <Code>review-bot</Code>.
                  Alle anderen Harnesses laufen normal.
                </Note>
                <CodeBlock
                  className="mt-3"
                  title="~/.beton/config.yaml"
                  start={12}
                  lines={[
                    '    agents:',
                    '      gemini: { command: gemini, args: ["--experimental-acp"] }',
                    { text: '      Review_Bot:', mark: 'error', note: 'Slug „Review_Bot“: nur a–z, 0–9 und - erlaubt' },
                    '        command: ./tools/review-bot',
                    '        args: [acp]',
                  ]}
                />
              </F>
            )}
            <section>
              <SectionTitle aside="Presets werden aktiv, sobald das Programm gefunden wird">Registriert</SectionTitle>
              <AgentList invalid={state === 'invalid'} />
            </section>
            <F id="HAR-007">
              <p className="max-w-2xl text-[12px] text-muted-foreground">
                Dateizugriffe und Terminal-Befehle eines ACP-Agents führt beton selbst in der Sandbox aus. Anmelden musst du dich in der jeweiligen
                CLI (z. B. Google-Login in gemini); beton speichert dafür nichts.
              </p>
            </F>
          </div>
        </div>
        {state === 'custom' && <AddPanel />}
      </div>
    </HarnessSettings>
  )
}
