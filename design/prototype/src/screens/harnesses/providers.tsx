import type { ReactNode } from 'react'
import { Plus } from 'lucide-react'
import { VoiceDot } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Code, CodeBlock, FakeInput, FieldLabel, Mark, Note, PageHeader, Pill, SectionTitle } from '@/app/kit/harnesses'
import { drawerClass } from './kit'
import { HarnessSettings } from './shell'

function ProviderRow({
  name,
  id,
  url,
  wire,
  keyInfo,
  models,
  extra,
  dim,
}: {
  name: string
  id: string
  url: string
  wire: string
  keyInfo: ReactNode
  models: ReactNode
  extra?: ReactNode
  dim?: boolean
}) {
  return (
    <div className={cn('px-1 py-3 text-[13px]', dim && 'text-muted-foreground')}>
      <div className="grid grid-cols-[minmax(170px,1fr)_minmax(220px,1.3fr)_minmax(220px,1.4fr)] gap-4">
        <div>
          <div className="flex items-center gap-2 font-medium">
            <VoiceDot voice="direct" />
            {name}
          </div>
          <div className="pl-4 font-mono text-[11px] text-muted-foreground">{id}</div>
        </div>
        <div className="min-w-0">
          <div className="truncate font-mono text-[12px]">{url}</div>
          <div className="text-[11px] text-muted-foreground">{wire}</div>
        </div>
        <div>{keyInfo}</div>
      </div>
      <div className="mt-2 pl-4 text-[12px]">{models}</div>
      {extra && <div className="mt-2 pl-4">{extra}</div>}
    </div>
  )
}

function ModelChips({ items, stale }: { items: [string, string][]; stale?: boolean }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {items.map(([m, d]) => (
        <span key={m} className={cn('inline-flex items-baseline gap-1.5 rounded-sm border border-border bg-card px-1.5 py-0.5', stale && 'border-dashed')}>
          <span className="font-mono text-[11.5px]">{m}</span>
          <span className="text-[11px] text-muted-foreground">{d}</span>
        </span>
      ))}
      {stale && (
        <F id="HAR-011" as="span">
          <Mark kind="skip" label={<span className="text-muted-foreground">Liste veraltet (Stand 01.10., 14:02) – /models nicht erreichbar</span>} />
        </F>
      )}
    </div>
  )
}

function AddPanel() {
  return (
    <F id={['HAR-011', 'HAR-010']} as="section" className={cn(drawerClass, 'w-[440px]')}>
      <div className="border-b border-border px-5 py-3">
        <h3 className="type-wide text-[15px] font-[700]">Anbieter hinzufügen</h3>
        <p className="text-[12px] text-muted-foreground">Alles mit OpenAI- oder Anthropic-kompatibler API.</p>
      </div>
      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4 text-[13px]">
        <div>
          <FieldLabel>Vorlage</FieldLabel>
          <div className="flex flex-wrap gap-1.5">
            {['OpenRouter', 'LiteLLM', 'vLLM', 'Ollama', 'LM Studio', 'Anthropic', 'Eigener Endpunkt'].map((p) => (
              <span key={p} className={cn('rounded-md border px-2 py-1 text-xs', p === 'OpenRouter' ? 'border-foreground bg-foreground text-background' : 'border-border')}>
                {p}
              </span>
            ))}
          </div>
        </div>
        <div className="grid grid-cols-[1fr_120px] gap-3">
          <div>
            <FieldLabel>Name</FieldLabel>
            <FakeInput value="openrouter" mono />
          </div>
          <div>
            <FieldLabel>API-Format</FieldLabel>
            <FakeInput value="openai" mono />
          </div>
        </div>
        <div>
          <FieldLabel>Adresse</FieldLabel>
          <FakeInput value="https://openrouter.ai/api/v1" mono />
        </div>
        <div>
          <FieldLabel hint="optional">API-Schlüssel</FieldLabel>
          <div className="space-y-1.5">
            {[
              ['Kein Schlüssel', 'für lokale Server wie Ollama', false],
              ['Aus Umgebungsvariable des Daemons', 'der Wert wird nie in Dateien, Events oder Logs geschrieben', true],
              ['Aus dem Schlüsselbund', 'ab M2; der Agent sieht nur einen Platzhalter', false],
            ].map(([t, d, sel]) => (
              <label key={String(t)} className={cn('flex gap-2 rounded-md border p-2', sel ? 'border-foreground/40 bg-card' : 'border-border text-muted-foreground')}>
                <span className={cn('mt-0.5 size-3.5 shrink-0 rounded-full', sel ? 'border-4 border-foreground' : 'border border-muted-foreground')} />
                <span>
                  <span className={cn('font-medium', !sel && 'text-foreground')}>{t}</span>
                  <span className="block text-[12px]">{d}</span>
                  {sel && <FakeInput className="mt-1.5" value="OPENROUTER_API_KEY" mono />}
                </span>
              </label>
            ))}
          </div>
        </div>
        <div>
          <SectionTitle aside="über GET /models">Modelle</SectionTitle>
          <ModelChips items={[['qwen/qwen3-coder', '262k · 0,40 / 1,60 $ pro Mio.'], ['deepseek/deepseek-v3.2', '128k']]} />
        </div>
        <Note>
          Netzwerkzugriff: beton spricht mit <Code>openrouter.ai</Code>, sobald eine Session diesen Anbieter nutzt. Für lokale Adressen (127.0.0.1,
          LAN) fragt beton beim ersten Mal nach.
        </Note>
        <CodeBlock
          title="~/.beton/config.yaml (Vorschau)"
          lines={[
            'providers:',
            { text: '  openrouter:', mark: 'add' },
            { text: '    kind: openai', mark: 'add' },
            { text: '    base_url: https://openrouter.ai/api/v1', mark: 'add' },
            { text: '    api_key_env: OPENROUTER_API_KEY', mark: 'add' },
          ]}
        />
      </div>
      <div className="flex gap-2 border-t border-border px-5 py-3">
        <Btn variant="primary">Anbieter speichern</Btn>
        <Btn variant="ghost" className="ml-auto">
          Abbrechen
        </Btn>
      </div>
    </F>
  )
}

export function ProvidersScreen({ state }: { state: string }) {
  const missing = state === 'env-missing'
  return (
    <HarnessSettings active="direct-api">
      <div className="relative flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <PageHeader
            title="Direkt-API & Gateways"
            sub="Optional: Modelle direkt per API, lokal oder über ein Gateway. beton führt dafür einen eigenen Agent-Loop aus."
            actions={
              state !== 'add' && (
                <Btn variant="primary">
                  <Plus className="size-3.5" /> Anbieter hinzufügen
                </Btn>
              )
            }
          />
          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-4">
            <F id={['HAR-010', 'HAR-015']}>
              <p className="max-w-3xl text-[13px] text-muted-foreground">
                Für Claude Code und Codex mit Abo brauchst du hier nichts einzurichten. Direkt-API eignet sich für lokale Modelle und Gateways. Weil
                beton jeden Modell-Request selbst sendet, prüfen deine Policies hier auch die Anfragen ans Modell.
              </p>
            </F>
            {missing && (
              <F id="HAR-011">
                <Note
                  tone="deny"
                  title="Session „Lokales Modell testen“ konnte nicht starten"
                  action={
                    <Btn variant="outline" size="sm">
                      Daemon neu starten
                    </Btn>
                  }
                >
                  OpenRouter liest den Schlüssel aus <Code>OPENROUTER_API_KEY</Code>, aber die Variable ist in der Umgebung des beton-Daemons nicht
                  gesetzt. Trage sie z. B. in <Code>~/.zshenv</Code> ein und starte den Daemon neu. Alternativ: Session mit Ollama (lokal) starten.
                </Note>
              </F>
            )}
            <section>
              <SectionTitle>Eingerichtet</SectionTitle>
              <div className="divide-y divide-border border-y border-border">
                <F id={['HAR-011', 'HAR-016']} badge="top-right">
                  <ProviderRow
                    name="Ollama (lokal)"
                    id="direct:ollama"
                    url="http://127.0.0.1:11434/v1"
                    wire="OpenAI-kompatibel · läuft auf diesem Rechner"
                    keyInfo={<Mark kind="ok" label="Kein Schlüssel nötig" />}
                    models={<ModelChips items={[['qwen3-coder:30b', '256k'], ['gpt-oss:20b', '128k'], ['devstral:24b', '128k']]} />}
                    extra={<Pill>Netzwerk: 127.0.0.1:11434 freigegeben</Pill>}
                  />
                </F>
                <F id="HAR-011" badge="top-right">
                  <ProviderRow
                    name="OpenRouter"
                    id="direct:openrouter"
                    url="https://openrouter.ai/api/v1"
                    wire="OpenAI-kompatibel · Gateway"
                    keyInfo={
                      missing ? (
                        <Mark kind="deny" label={<>Variable <Code>OPENROUTER_API_KEY</Code> nicht gesetzt</>} />
                      ) : (
                        <Mark kind="ok" label={<>Schlüssel aus <Code>OPENROUTER_API_KEY</Code></>} />
                      )
                    }
                    models={<ModelChips stale={state === 'stale'} items={[['qwen/qwen3-coder', '262k · 0,40 / 1,60 $'], ['deepseek/deepseek-v3.2', '128k · 0,27 / 1,10 $']]} />}
                  />
                </F>
              </div>
            </section>
            <section>
              <SectionTitle aside="nur wenn du API statt Abo abrechnen willst">Nicht eingerichtet</SectionTitle>
              <div className="divide-y divide-border border-y border-border">
                <ProviderRow
                  dim
                  name="Anthropic API"
                  id="direct:anthropic"
                  url="https://api.anthropic.com"
                  wire="Anthropic Messages"
                  keyInfo={<Mark kind="off" label="Kein Schlüssel hinterlegt" />}
                  models={<span>Prompt-Caching und Thinking werden unterstützt.</span>}
                />
              </div>
            </section>
          </div>
        </div>
        {state === 'add' && <AddPanel />}
      </div>
    </HarnessSettings>
  )
}
