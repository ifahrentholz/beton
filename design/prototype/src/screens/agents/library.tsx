import type { ReactNode } from 'react'
import { Play, Plus, Search } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, VoiceDot } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { harnesses } from '@/mock/data'
import { Btn, Code, Mark, Note, PageHeader, Pill, Segmented } from '@/screens/harnesses/kit'
import { agents, sourceLabel, type AgentEntry } from './data'

function AgentRow({ a, warn, selected }: { a: AgentEntry; warn?: ReactNode; selected?: boolean }) {
  return (
    <div className={cn('grid grid-cols-[minmax(180px,0.9fr)_minmax(260px,2fr)_minmax(170px,1fr)_minmax(150px,0.8fr)_auto] items-start gap-4 px-6 py-3 text-[13px]', selected && 'bg-accent')}>
      <div className="min-w-0">
        <div className="font-semibold">{a.name}</div>
        <div className="truncate font-mono text-[11px] text-muted-foreground">{a.path}</div>
      </div>
      <div className="min-w-0">
        <p className="text-muted-foreground">{a.description}</p>
        {warn}
      </div>
      <div className="space-y-0.5">
        <HarnessBadge id={a.harness} />
        {a.also && (
          <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            Sub-Agents:
            {a.also.map((h) => (
              <span key={h} className="inline-flex items-center gap-1">
                <VoiceDot voice={harnesses[h].voice} className="size-1.5" />
                {harnesses[h].name}
              </span>
            ))}
          </div>
        )}
        <div className="text-[11px] text-muted-foreground">{a.mode}</div>
      </div>
      <div className="space-y-1">
        <Pill>
          {sourceLabel[a.source]} · v{a.version}
        </Pill>
        <div className="text-[11px] text-muted-foreground">
          {a.lastRun ? `Zuletzt ${a.lastRun}` : 'Noch nie gestartet'}
          {a.schedule && ` · geplant ${a.schedule}`}
        </div>
      </div>
      <Btn variant="outline" size="sm">
        <Play className="size-3" /> Starten
      </Btn>
    </div>
  )
}

export function AgentLibrary({ state, overlay }: { state: string; overlay?: ReactNode }) {
  const empty = state === 'empty'
  const shadowed = state === 'shadowed'
  const list = empty ? agents.filter((a) => a.source === 'builtin') : agents
  const builtins = list.filter((a) => a.source === 'builtin')
  const own = list.filter((a) => a.source !== 'builtin')

  return (
    <AppLayout nav="agents">
      <div className="relative flex min-h-0 flex-1 flex-col">
        <PageHeader
          title="Agents"
          sub="Eigene Agents sind YAML-Dateien, die auf einem Harness laufen – mit deinem Claude- oder ChatGPT-Abo, ohne API-Schlüssel."
          actions={
            <Btn variant="primary">
              <Plus className="size-3.5" /> Agent anlegen
            </Btn>
          }
        />
        <div className="flex items-center gap-3 border-b border-border px-6 py-2">
          <label className="flex h-7 w-64 items-center gap-2 rounded-md border border-input bg-card px-2 text-xs text-muted-foreground">
            <Search className="size-3.5" /> Agents durchsuchen
          </label>
          <Segmented items={['Alle', 'Mitgeliefert', 'Projekt', 'Für mich']} active="Alle" />
          <span className="ml-auto text-[11px] text-muted-foreground">Suchpfad: Projekt → ~/.beton/agents → mitgeliefert</span>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {shadowed && (
            <F id="AGT-003" className="px-6 pt-4">
              <Note
                tone="warn"
                title="Dein Projekt verschattet den mitgelieferten Agent maestra"
                action={
                  <Btn variant="ghost" size="sm">
                    Mitgelieferte Version starten
                  </Btn>
                }
              >
                <Code>.beton/agents/maestra</Code> wird statt des Built-ins verwendet. Mit <Code>builtin:maestra</Code> startest du gezielt die
                mitgelieferte Version.
              </Note>
            </F>
          )}
          <F id={['AGT-003', 'AGT-011', 'AGT-012']} as="section" className="pt-4">
            <div className="px-6 pb-1 text-[12px] font-medium text-muted-foreground">Mitgeliefert</div>
            <div className="divide-y divide-border border-y border-border">
              {builtins.map((a) =>
                shadowed && a.name === 'maestra' ? (
                  <div key={a.name} className="opacity-60">
                    <AgentRow a={a} warn={<Mark kind="skip" label="Verschattet durch .beton/agents/maestra" className="mt-1" />} />
                  </div>
                ) : (
                  <AgentRow key={a.name} a={a} />
                ),
              )}
            </div>
          </F>
          <F id={['AGT-003', 'AGT-001']} as="section" className="pt-5 pb-6">
            <div className="px-6 pb-1 text-[12px] font-medium text-muted-foreground">Eigene</div>
            {empty ? (
              <div className="concrete-grain mx-6 flex flex-col items-center gap-2 rounded-md border border-dashed border-border px-6 py-12 text-center">
                <p className="type-wide text-base font-[700]">Noch keine eigenen Agents</p>
                <p className="max-w-lg text-[13px] text-muted-foreground">
                  Ein Agent ist ein Ordner mit <Code>agent.yaml</Code>: Anweisungen, Tools als MCP-Server, Skills und auf welchem Harness er läuft.
                  Lege ihn im Projekt unter <Code>.beton/agents/</Code> an oder für dich unter <Code>~/.beton/agents/</Code>.
                </p>
                <div className="mt-2 flex gap-2">
                  <Btn variant="primary">Agent anlegen</Btn>
                  <Btn variant="outline">Von maestra ableiten</Btn>
                </div>
                <p className="mt-1 text-[11px] text-muted-foreground">Im Terminal: beton agent new &lt;name&gt;</p>
              </div>
            ) : (
              <div className="divide-y divide-border border-y border-border">
                {shadowed && (
                  <AgentRow
                    a={{ ...agents[0], source: 'project', path: '.beton/agents/maestra', version: '1.0.0-shop', description: 'Projektvariante: maestra mit zusätzlichem a11y-Review vor dem Merge-Vorschlag.' }}
                    warn={<Mark kind="skip" label={<span className="text-muted-foreground">Verschattet das Built-in maestra</span>} className="mt-1" />}
                  />
                )}
                {own.map((a) => (
                  <AgentRow key={a.name} a={a} />
                ))}
                {shadowed && (
                  <div className="flex items-center gap-4 px-6 py-3 text-[13px] text-muted-foreground">
                    <Mark kind="deny" label={<span className="font-medium text-foreground">old-reviewer</span>} />
                    <span className="font-mono text-[11px]">.beton/agents/old-reviewer</span>
                    <span>Ungültig: keine agent.yaml im Ordner, wird übersprungen.</span>
                  </div>
                )}
              </div>
            )}
          </F>
        </div>
        {overlay}
      </div>
    </AppLayout>
  )
}
