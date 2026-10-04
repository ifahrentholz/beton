import { F } from '@/proto/feature-marker'
import { Cmd, Ln, Term } from '@/app/kit/harness-term'

export function AgentCli({ state }: { state: string }) {
  if (state === 'validate')
    return (
      <Term>
        <F id={['AGT-013', 'AGT-002', 'AGT-001']}>
          <Cmd>beton agent validate .beton/agents/pr-fixer</Cmd>
          <Ln tone="deny">✗ agent.yaml:14:1  unknown_field     Unbekanntes Feld „instruction“. Meintest du „instructions“?</Ln>
          <Ln tone="deny">✗ agent.yaml:33:1  skill_not_found   Skill „ci-triage“ nicht gefunden.</Ln>
          <Ln tone="muted">! agent.yaml:10:3  effort_mapped     „xhigh“ → „high“ (claude-sonnet-5-5 kennt keine höhere Stufe)</Ln>
          <Ln />
          <Ln>2 Fehler, 1 Warnung · Exit-Code 1 · maschinenlesbar mit --json</Ln>
          <Cmd>beton agent validate .beton/agents/loop-a</Cmd>
          <Ln tone="deny">✗ agent.yaml:22:5  agent_cycle       Sub-Agent-Zyklus: loop-a → loop-b → loop-a</Ln>
        </F>
      </Term>
    )
  if (state === 'new')
    return (
      <Term>
        <F id={['AGT-013', 'AGT-003']}>
          <Cmd>beton agent new release-notes --from builtin:maestra</Cmd>
          <Ln>Angelegt in .beton/agents/release-notes/</Ln>
          <Ln tone="muted">  agent.yaml              mit $schema-Kommentar für Editor-Vervollständigung</Ln>
          <Ln tone="muted">  prompts/system.md</Ln>
          <Ln tone="muted">  skills/example/SKILL.md</Ln>
          <Ln />
          <Ln tone="ok">✓ beton agent validate .beton/agents/release-notes: gültig</Ln>
          <Ln>Starten mit: beton run release-notes -p "…"</Ln>
        </F>
      </Term>
    )
  if (state === 'show')
    return (
      <Term>
        <F id={['AGT-013', 'AGT-004']}>
          <Cmd>beton agent show pr-fixer</Cmd>
          <Ln>pr-fixer 0.3.0 · Projekt · .beton/agents/pr-fixer · sha256:9f2c…e1</Ln>
          <Ln />
          <Ln>executor.harness          claude                 agent.yaml:8</Ln>
          <Ln>executor.model            claude-sonnet-5-5      agent.yaml:9</Ln>
          <Ln>executor.reasoning_effort high (angefragt xhigh)  agent.yaml:10</Ln>
          <Ln>executor.max_turns        200                    Standard</Ln>
          <Ln>instructions              prompts/system.md + 1 Zeile + AGENTS.md (project_files: auto)</Ln>
          <Ln>tools.mcp.github          stdio · 3 Tools        agent.yaml:25</Ln>
          <Ln>skills                    fix-ci                 skills/fix-ci</Ln>
          <Ln>policies                  3 (1 Datei, 2 inline)  Agent-Ebene</Ln>
        </F>
      </Term>
    )
  return (
    <Term>
      <F id={['AGT-013', 'AGT-003']}>
        <Cmd>beton agent list --all</Cmd>
        <Ln>Name          Quelle    Version     Harness        Pfad</Ln>
        <Ln>maestra       project   1.0.0-shop  claude         .beton/agents/maestra</Ln>
        <Ln tone="muted">              ! verschattet builtin:maestra</Ln>
        <Ln>pr-fixer      project   0.3.0       claude         .beton/agents/pr-fixer</Ln>
        <Ln>a11y-auditor  project   0.1.0       acp:gemini     .beton/agents/a11y-auditor</Ln>
        <Ln>dep-updater   user      1.2.0       codex          ~/.beton/agents/dep-updater</Ln>
        <Ln>local-scout   user      0.2.0       direct:ollama  ~/.beton/agents/local-scout</Ln>
        <Ln>maestra       builtin   1.0.0       claude         builtin:maestra</Ln>
        <Ln>duetto        builtin   1.0.0       claude         builtin:duetto</Ln>
        <Ln tone="deny">old-reviewer  ungültig  –           –              .beton/agents/old-reviewer (keine agent.yaml)</Ln>
      </F>
    </Term>
  )
}
