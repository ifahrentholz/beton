import type { ReactNode } from 'react'
import { ChevronLeft, ExternalLink, Play } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Code, CodeBlock, Mark, Note, Pill, Row, SectionTitle, type CodeLine } from '@/app/kit/harnesses'
import { prFixerYaml } from './data'

const TABS = [
  { id: 'overview', label: 'Übersicht' },
  { id: 'yaml', label: 'agent.yaml' },
  { id: 'tools', label: 'Tools' },
  { id: 'skills', label: 'Skills' },
  { id: 'policies', label: 'Policies & Sandbox' },
  { id: 'runs', label: 'Läufe' },
]

function Header({ tab, errors }: { tab: string; errors?: number }) {
  return (
    <div className="shrink-0 border-b border-border px-6 pt-3">
      <button className="mb-1 inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground">
        <ChevronLeft className="size-3.5" /> Agents
      </button>
      <div className="flex items-start gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-3">
            <h2 className="type-wide text-lg font-[700]">pr-fixer</h2>
            <Pill>Projekt · v0.3.0</Pill>
            <span className="font-mono text-[11px] text-muted-foreground">.beton/agents/pr-fixer</span>
          </div>
          <p className="mt-0.5 text-[13px] text-muted-foreground">Behebt fehlschlagende CI-Checks auf einem Branch und öffnet einen PR.</p>
        </div>
        <div className="flex gap-2">
          <Btn variant="outline">
            <ExternalLink className="size-3.5" /> Im Editor öffnen
          </Btn>
          <Btn variant="primary">
            <Play className="size-3.5" /> Agent starten
          </Btn>
        </div>
      </div>
      <div role="tablist" className="mt-3 flex gap-1">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={t.id === tab}
            className={cn('-mb-px border-b-2 px-2 pb-2 text-[13px]', t.id === tab ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground hover:text-foreground')}
          >
            {t.label}
            {t.id === 'yaml' && !!errors && <span className="ml-1.5 rounded-sm bg-deny px-1 text-[10px] font-semibold text-background">{errors}</span>}
          </button>
        ))}
      </div>
    </div>
  )
}

function Block({ title, aside, children, id }: { title: string; aside?: ReactNode; children: ReactNode; id: string | string[] }) {
  return (
    <F id={id} as="section">
      <SectionTitle aside={aside}>{title}</SectionTitle>
      <dl className="divide-y divide-border border-y border-border">{children}</dl>
    </F>
  )
}

function Overview() {
  return (
    <div className="grid grid-cols-2 gap-x-10 gap-y-6">
      <Block title="Ausführung" aside="executor" id={['AGT-004', 'HAR-027']}>
        <Row label="Harness">
          <HarnessBadge id="claude" model="claude-sonnet-5-5" /> <span className="text-[12px] text-muted-foreground">· Abo über claude-CLI</span>
        </Row>
        <Row label="Effort">mittel</Row>
        <Row label="Modus">Dateien ohne Rückfrage ändern</Row>
        <Row label="Grenzen">200 Modell-Anfragen pro Turn · 2 Std. pro Lauf</Row>
        <Row label="Letzter Stand">
          <span className="font-mono text-[12px]">sha256:9f2c…e1</span> <span className="text-muted-foreground">· eingefroren beim Start heute 03:00</span>
        </Row>
      </Block>
      <Block title="Anweisungen" aside="instructions" id="AGT-005">
        <Row label="Datei">
          <Code>prompts/system.md</Code> + Zusatz (1 Zeile)
        </Row>
        <Row label="Projektdateien">
          <Code>CLAUDE.md</Code> liest Claude selbst; beton fügt <Code>AGENTS.md</Code> an
        </Row>
        <Row label="Auslieferung">
          <span className="text-muted-foreground">Claude: als System-Prompt-Zusatz · Codex: als Developer-Instructions · ACP: vor der ersten Nachricht</span>
        </Row>
      </Block>
      <Block title="Parameter" aside="params" id="AGT-010">
        <Row label={<span className="font-mono text-[12px] text-foreground">branch</span>}>Text · Standard main</Row>
        <Row label={<span className="font-mono text-[12px] text-foreground">max_attempts</span>}>Ganzzahl · 1 bis 10 · Standard 3</Row>
      </Block>
      <Block title="Sub-Agents" aside="agents · spawn" id="AGT-009">
        <Row label="quick-check">
          <HarnessBadge id="codex" model="gpt-5.3-codex" /> <span className="text-[12px] text-muted-foreground">· nur planen, inline definiert</span>
        </Row>
        <Row label="Grenzen">Tiefe 2 · höchstens 3 gleichzeitig · eigener Worktree · max. 50 % des Budgets je Kind</Row>
      </Block>
      <Block title="Automationen" aside="schedules · async" id={['ASY-004', 'ASY-008']}>
        <Row label="nightly-ci">täglich 03:00 (Europe/Berlin) · nächster Lauf So., 04.10.</Row>
        <Row label="Ergebnis">Weckt die aufrufende Session · Inbox und Push</Row>
        <Row label="Freigaben ohne dich">Warten 8 Std., danach abgelehnt</Row>
      </Block>
    </div>
  )
}

const ERRORS: Record<number, CodeLine> = {
  10: { text: '', mark: 'warn', note: 'Warnung · claude-sonnet-5-5 kennt höchstens „high“. Wirksam ist high.' },
  14: { text: '', mark: 'error', note: 'Fehler · Unbekanntes Feld „instruction“ (Zeile 14, Spalte 1). Meintest du „instructions“?' },
  33: { text: '', mark: 'error', note: 'Fehler · Skill „ci-triage“ nicht gefunden (gesucht: skills/, .beton/skills, .claude/skills, ~/.beton/skills …)' },
}

function YamlTab({ valid }: { valid?: boolean }) {
  const lines: CodeLine[] = prFixerYaml.map((text, i) => {
    const e = ERRORS[i + 1]
    if (valid) {
      if (i + 1 === 10) return { text: '  reasoning_effort: high' }
      if (i + 1 === 14) return { text: 'instructions:' }
      if (i + 1 === 33) return { text: 'skills: [fix-ci]' }
      return { text }
    }
    return e ? { ...e, text } : { text }
  })
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_300px] gap-6">
      <F id={['AGT-001', 'AGT-002']}>
        <CodeBlock title=".beton/agents/pr-fixer/agent.yaml" lines={lines} />
      </F>
      <F id={['AGT-002', 'AGT-001']} as="section" className="space-y-4 text-[13px]">
        {valid ? (
          <Mark kind="ok" label={<span className="font-medium">Gültig · Schema agent.v1 und alle Verweise geprüft</span>} />
        ) : (
          <>
            <Mark kind="deny" label={<span className="font-medium">2 Fehler, 1 Warnung – der Agent startet so nicht</span>} />
            <ul className="divide-y divide-border border-y border-border">
              {[
                ['14:1', 'Unbekanntes Feld „instruction“', 'Meintest du „instructions“?', 'error'],
                ['33:1', 'Skill „ci-triage“ nicht gefunden', 'Lege skills/ci-triage/SKILL.md an oder entferne den Eintrag.', 'error'],
                ['10:3', 'Effort „xhigh“ wird zu „high“', 'Das Modell kennt keine höhere Stufe.', 'warn'],
              ].map(([pos, t, d, k]) => (
                <li key={pos} className="flex gap-2 py-2">
                  <Mark kind={k === 'error' ? 'deny' : 'skip'} />
                  <div>
                    <div>
                      <span className="font-mono text-[11px] text-muted-foreground">Zeile {pos}</span> {t}
                    </div>
                    <div className="text-[12px] text-muted-foreground">{d}</div>
                  </div>
                </li>
              ))}
            </ul>
          </>
        )}
        <p className="text-[12px] text-muted-foreground">
          Geprüft gegen <Code>schemas/agent.v1.json</Code> und semantisch: Harness vorhanden, Effort passt zum Modell, Dateien, Skills und Sub-Agents
          existieren, keine Zyklen. Dasselbe im Terminal: <Code>beton agent validate</Code>.
        </p>
        <p className="text-[12px] text-muted-foreground">Bearbeitet wird in deinem Editor; beton lädt die Datei bei jeder Änderung neu.</p>
      </F>
    </div>
  )
}

function ToolsTab() {
  return (
    <div className="space-y-6">
      <F id={['AGT-006', 'HAR-009']} as="section">
        <SectionTitle aside="Agent-Läufe nutzen nur Agent-Server (tools.inherit: false)">MCP-Server</SectionTitle>
        <table className="type-narrow w-full text-[12.5px]">
          <thead>
            <tr className="border-b border-border text-left text-[11px] text-muted-foreground">
              <th className="py-1.5 font-normal">Name</th>
              <th className="font-normal">Ebene</th>
              <th className="font-normal">Verbindung</th>
              <th className="font-normal">Freigegebene Tools</th>
              <th className="font-normal">Zustand</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            <tr>
              <td className="py-2 font-medium">github</td>
              <td>Agent</td>
              <td className="font-mono text-[11.5px]">github-mcp-server stdio</td>
              <td>get_pull_request, create_pull_request, list_check_runs</td>
              <td>
                <Mark kind="ok" label="Token als Platzhalter" />
              </td>
            </tr>
            <tr>
              <td className="py-2 font-medium">docs</td>
              <td>Agent</td>
              <td className="font-mono text-[11.5px]">https://mcp.example.com/mcp</td>
              <td className="text-muted-foreground">alle</td>
              <td>
                <Mark kind="ok" label="erreichbar" />
              </td>
            </tr>
            <tr className="text-muted-foreground">
              <td className="py-2">playwright</td>
              <td>Projekt</td>
              <td className="font-mono text-[11.5px]">.beton/mcp.yaml</td>
              <td>–</td>
              <td>
                <Mark kind="off" label="nicht geerbt" />
              </td>
            </tr>
            <tr className="text-muted-foreground">
              <td className="py-2">linear</td>
              <td>Für mich</td>
              <td className="font-mono text-[11.5px]">~/.beton/mcp.yaml</td>
              <td>–</td>
              <td>
                <Mark kind="off" label="nicht geerbt" />
              </td>
            </tr>
          </tbody>
        </table>
        <p className="mt-2 text-[12px] text-muted-foreground">
          Tools sind immer MCP-Server. Alle laufen über den beton-Proxy, damit Policies jeden Aufruf prüfen – unabhängig vom Harness.
        </p>
      </F>
      <F id="AGT-007" as="section">
        <SectionTitle aside="MCP-Server beton, in jeden Harness eingebunden">System-Tools</SectionTitle>
        <div className="grid grid-cols-2 gap-x-10">
          {[
            ['session_spawn · session_wait · session_send', 'Sub-Agents starten und abwarten', 'ok', 'weil spawn.agents gesetzt ist'],
            ['inbox_read · ask_user · policy_query', 'Ergebnisse lesen, dich fragen, Policy prüfen', 'ok', 'immer verfügbar'],
            ['skill_load · skill_read_file', 'Skills für Harnesses ohne eigene Skill-Unterstützung', 'ok', 'weil Skills aktiv sind'],
            ['timer_set · timer_list · timer_cancel', 'Sich selbst später wieder aufnehmen', 'ok', 'ab M5'],
            ['schedule_create · schedule_list …', 'Schedules verwalten', 'off', 'nicht in tools.system'],
          ].map(([n, d, k, why]) => (
            <div key={n} className={cn('flex items-start gap-2 border-b border-border py-2 text-[13px]', k === 'off' && 'text-muted-foreground')}>
              <Mark kind={k === 'ok' ? 'ok' : 'off'} />
              <div className="min-w-0">
                <div className="font-mono text-[12px]">{n}</div>
                <div className="text-[12px] text-muted-foreground">
                  {d} · {why}
                </div>
              </div>
            </div>
          ))}
        </div>
      </F>
    </div>
  )
}

function SkillsTab() {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-8">
      <F id="AGT-008" as="section">
        <SectionTitle aside="skills: [fix-ci, ci-triage]">Ausgewählte Skills</SectionTitle>
        <ul className="divide-y divide-border border-y border-border text-[13px]">
          <li className="py-2">
            <div className="flex items-center gap-2">
              <Mark kind="ok" />
              <span className="font-medium">fix-ci</span>
              <Pill>Agent</Pill>
              <Pill>/fix-ci aufrufbar</Pill>
            </div>
            <div className="pl-5.5 text-[12px] text-muted-foreground">
              Reproduziert CI-Fehler lokal und behebt sie. Verschattet <Code>.beton/skills/fix-ci</Code> im Projekt.
            </div>
          </li>
          <li className="py-2">
            <div className="flex items-center gap-2">
              <Mark kind="deny" />
              <span className="font-medium">ci-triage</span>
              <span className="text-[12px] text-deny">nicht gefunden</span>
            </div>
          </li>
          <li className="py-2 text-muted-foreground">
            <div className="flex items-center gap-2">
              <Mark kind="skip" />
              <span>notes</span>
              <Pill>~/.agents/skills</Pill>
            </div>
            <div className="pl-5.5 text-[12px]">Übersprungen: SKILL.md ohne description.</div>
          </li>
        </ul>
        <p className="mt-3 text-[12px] text-muted-foreground">
          Gesucht wird in dieser Reihenfolge: Agent <Code>skills/</Code> → Projekt <Code>.beton/skills</Code>, <Code>.claude/skills</Code>,{' '}
          <Code>.agents/skills</Code> → deine Ordner unter <Code>~</Code> → mitgeliefert. Claude Code lädt Skills selbst; andere Harnesses bekommen einen
          Index und laden per <Code>skill_load</Code>.
        </p>
      </F>
      <F id="AGT-008">
        <CodeBlock
          title="skills/fix-ci/SKILL.md"
          lines={[
            '---',
            'name: fix-ci',
            'description: Reproduziert fehlschlagende CI-Jobs lokal und behebt sie.',
            'user-invocable: true',
            'allowed-tools: [Bash, Read, Edit]',
            '---',
            '',
            '1. Lies die fehlgeschlagenen Checks mit list_check_runs.',
            '2. Starte scripts/run-ci-locally.sh <job>.',
            '3. Behebe die Ursache, nicht das Symptom. Keine Tests deaktivieren.',
          ]}
        />
        <div className="mt-2 flex gap-1.5">
          <Pill>scripts/run-ci-locally.sh</Pill>
          <Pill>references/ci-matrix.md</Pill>
        </div>
      </F>
    </div>
  )
}

function PoliciesTab() {
  return (
    <div className="grid grid-cols-2 gap-10">
      <F id="AGT-014" as="section">
        <SectionTitle aside="Agent-Ebene · kann nur verschärfen">Policies</SectionTitle>
        <ul className="divide-y divide-border border-y border-border text-[13px]">
          <li className="py-2">
            <div className="font-medium">policies/strict.yaml</div>
            <div className="text-[12px] text-muted-foreground">Datei · 6 Regeln</div>
          </li>
          <li className="py-2">
            <div className="font-medium">no-force-push · git_guard</div>
            <div className="text-[12px] text-muted-foreground">Force-Push verboten, Push fragt, main geschützt</div>
          </li>
          <li className="py-2">
            <div className="font-medium">spend · spend_cap</div>
            <div className="text-[12px] text-muted-foreground">pro Lauf: bei API-Abrechnung 5 $, Rückfrage ab 3 $ · im Abo zählen Turns und Laufzeit</div>
          </li>
        </ul>
        <Note className="mt-3">Eine erlaubende Regel hier hebt kein Verbot aus Projekt, Team oder Organisation auf. Die strengere Regel gewinnt.</Note>
      </F>
      <F id="AGT-014" as="section">
        <SectionTitle aside="sandbox">Sandbox</SectionTitle>
        <dl className="divide-y divide-border border-y border-border">
          <Row label="Profil">Schreiben nur im Worktree</Row>
          <Row label="Netzwerk">
            <div className="font-mono text-[12px]">GET,POST api.github.com/**</div>
            <div className="font-mono text-[12px]">GET registry.npmjs.org/**</div>
          </Row>
          <Row label="Auf diesem Rechner">
            <Mark kind="ok" label="verfügbar (Seatbelt)" />
          </Row>
        </dl>
        <p className="mt-2 text-[12px] text-muted-foreground">Ist die geforderte Sandbox nicht verfügbar, startet der Agent nicht.</p>
      </F>
    </div>
  )
}

export function AgentDetail({ state }: { state: string }) {
  const tab = state.startsWith('yaml') ? 'yaml' : state
  return (
    <AppLayout nav="agents">
      <Header tab={tab} errors={state === 'yaml-errors' ? 3 : 0} />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
        {state === 'overview' && <Overview />}
        {state === 'yaml-errors' && <YamlTab />}
        {state === 'yaml-valid' && <YamlTab valid />}
        {state === 'tools' && <ToolsTab />}
        {state === 'skills' && <SkillsTab />}
        {state === 'policies' && <PoliciesTab />}
      </div>
    </AppLayout>
  )
}
