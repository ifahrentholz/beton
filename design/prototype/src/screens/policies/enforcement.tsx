import type { ReactNode } from 'react'
import { SessionHeader } from '@/app/session-chrome'
import { SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { VoiceDot } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, C, Callout, PageHead, Scroll, Shell } from './kit'

type Level = 'full' | 'approval' | 'observe' | 'mixed'

function LevelMark({ l, children }: { l: Level; children: ReactNode }) {
  return (
    <span className="inline-flex items-start gap-1.5 text-[12px] leading-snug">
      <span
        className={cn(
          'mt-[3px] size-2.5 shrink-0 rounded-full',
          l === 'full' && 'bg-foreground',
          l === 'approval' && 'border-2 border-foreground bg-[linear-gradient(90deg,var(--foreground)_50%,transparent_50%)]',
          l === 'observe' && 'border-2 border-muted-foreground',
          l === 'mixed' && 'border-2 border-foreground bg-[linear-gradient(90deg,var(--foreground)_50%,transparent_50%)]',
        )}
      />
      <span className={cn(l === 'observe' && 'text-muted-foreground')}>{children}</span>
    </span>
  )
}

const cols = [
  { id: 'claude', label: 'Claude Code', sub: 'nativ / TUI', voice: 'claude' as const },
  { id: 'codex', label: 'Codex', sub: 'nativ', voice: 'codex' as const },
  { id: 'codex-tui', label: 'Codex', sub: 'TUI', voice: 'codex' as const },
  { id: 'acp', label: 'ACP', sub: 'z. B. Gemini CLI', voice: 'acp' as const },
  { id: 'direct', label: 'Direkt-API', sub: 'z. B. Ollama', voice: 'direct' as const },
]

const rows: { phase: string; cells: [Level, string][] }[] = [
  { phase: 'session_start', cells: [['full', 'voll'], ['full', 'voll'], ['full', 'voll'], ['full', 'voll'], ['full', 'voll']] },
  {
    phase: 'model_request',
    cells: [
      ['full', 'voll, pro Turn · Modellwechsel live'],
      ['full', 'voll, pro Turn'],
      ['observe', 'nur beobachten'],
      ['full', 'voll, pro Turn · Wechsel nur mit set_model'],
      ['full', 'voll, pro Call, inkl. Schwärzen'],
    ],
  },
  {
    phase: 'tool_call',
    cells: [
      ['full', 'voll (PreToolUse-Hook, auch Argumente)'],
      ['approval', 'Shell, Edit: nur Freigabe · lesend: beobachten'],
      ['approval', 'wie Codex nativ'],
      ['mixed', 'nur Freigabe · fs/* und terminal/* voll'],
      ['full', 'voll'],
    ],
  },
  {
    phase: 'tool_result',
    cells: [
      ['mixed', 'erlauben/ablehnen/melden, kein Schwärzen'],
      ['observe', 'nur beobachten'],
      ['observe', 'nur beobachten'],
      ['mixed', 'beobachten · fs/*, terminal/* voll'],
      ['full', 'voll'],
    ],
  },
  { phase: 'MCP-Tools', cells: [['full', 'voll über den beton-MCP-Proxy'], ['full', 'voll'], ['full', 'voll'], ['full', 'voll'], ['full', 'voll']] },
  { phase: 'browser_*', cells: [['full', 'voll (beton-Browser)'], ['full', 'voll'], ['full', 'voll'], ['full', 'voll'], ['full', 'voll']] },
]

function Matrix() {
  return (
    <F id={['POL-024', 'POL-022', 'POL-023']} className="space-y-3">
      <div className="overflow-hidden rounded-md border border-border bg-card">
        <div className="grid grid-cols-[130px_repeat(5,minmax(0,1fr))] border-b border-border bg-sunken text-[12px]">
          <span className="px-3 py-2 text-muted-foreground">Phase</span>
          {cols.map((c) => (
            <span key={c.id} className="border-l border-border px-3 py-2">
              <span className="flex items-center gap-1.5 font-medium">
                <VoiceDot voice={c.voice} /> {c.label}
              </span>
              <span className="text-[11px] text-muted-foreground">{c.sub}</span>
            </span>
          ))}
        </div>
        {rows.map((r) => (
          <div key={r.phase} className="grid grid-cols-[130px_repeat(5,minmax(0,1fr))] border-b border-border text-[12px] last:border-b-0">
            <span className="px-3 py-2 font-mono">{r.phase}</span>
            {r.cells.map(([l, t], i) => (
              <span key={i} className="border-l border-border px-3 py-2">
                <LevelMark l={l}>{t}</LevelMark>
              </span>
            ))}
          </div>
        ))}
      </div>
      <div className="flex flex-wrap gap-5 text-[12px]">
        <LevelMark l="full">voll: ablehnen, fragen, ändern</LevelMark>
        <LevelMark l="approval">nur Freigabe: fragen und ablehnen, nicht ändern</LevelMark>
        <LevelMark l="observe">nur beobachten: Audit und Meldung, Sandbox ist die Grenze</LevelMark>
      </div>
      <div className="grid grid-cols-3 gap-6 border-t border-border pt-3 text-[12px]">
        <div>
          <div className="font-semibold">Claude Code im TUI-Modus</div>
          <p className="mt-1 text-muted-foreground">
            beton setzt session-eigene Hooks: <C>UserPromptSubmit</C> → model_request, <C>PreToolUse</C> → tool_call, <C>PostToolUse</C> → tool_result.
            Rückfragen erscheinen als beton-Freigabe, nicht als Dialog im TUI.
          </p>
        </div>
        <div>
          <div className="font-semibold">Codex</div>
          <p className="mt-1 text-muted-foreground">Über den Approval-Mechanismus von Codex. Lesende Kommandos fragt Codex nicht – beton sieht sie nur.</p>
        </div>
        <div>
          <div className="font-semibold">ACP-Agents</div>
          <p className="mt-1 text-muted-foreground">
            <C>session/request_permission</C> ist der Haken. beton antwortet immer einmalig (<C>allow_once</C>/<C>reject_once</C>), nie „immer erlauben“ –
            so wird jede weitere Aktion neu geprüft.
          </p>
        </div>
      </div>
    </F>
  )
}

export function PolicyEnforcement({ state }: { state: string }) {
  if (state === 'matrix') {
    return (
      <Shell nav="policies">
        <PageHead
          title="Durchsetzung je Harness"
          sub="Was beton bei welchem Harness und Transport durchsetzen kann. Abgeleitet aus den Fähigkeiten der Adapter, nicht fest verdrahtet."
        />
        <Scroll className="px-6 py-4">
          <Matrix />
        </Scroll>
      </Shell>
    )
  }
  const denied = state === 'start-denied'
  return (
    <Shell nav="sessions" activeSession="ses_7f3m">
      <SessionHeader title="Review: Rate-Limiter" harness="codex" status={denied ? 'failed' : 'running'} branch="beton/rate-limiter-7f3k" />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          <UserMessage>Reviewe den Rate-Limiter-Branch im Codex-TUI und korrigiere Kleinigkeiten direkt.</UserMessage>
          {denied ? (
            <F id={['POL-024', 'POL-022']}>
              <Callout
                tone="deny"
                title="Session startet nicht: eine Pflichtregel lässt sich hier nicht durchsetzen"
                actions={
                  <>
                    <Btn>Nativ statt im TUI starten</Btn>
                    <Btn>Regel öffnen</Btn>
                    <Btn tone="quiet">Durchsetzung ansehen</Btn>
                  </>
                }
              >
                <p>
                  Die Projekt-Regel <span className="font-mono">no-edit-outside-src</span> verlangt „ablehnen“ für <span className="font-mono">file_edit</span>.
                  Codex im TUI-Modus kann Dateiänderungen nur beobachten. Damit nichts unbemerkt durchrutscht, startet beton den Harness gar nicht erst.
                </p>
                <p className="text-muted-foreground">
                  Ereignis <span className="font-mono">session.start_denied</span> · Grund <span className="font-mono">enforcement_unavailable</span> · kein Prozess
                  gestartet
                </p>
              </Callout>
            </F>
          ) : (
            <F id={['POL-024', 'POL-023']} className="flex flex-col gap-4">
              <SystemNote>Session gestartet · Codex nativ · Sandbox seatbelt</SystemNote>
              <Callout tone="neutral" title="Eingeschränkte Durchsetzung bei lesenden Aktionen">
                <p>
                  Die Regel <span className="font-mono">no-secrets-read</span> (User) betrifft <span className="font-mono">file_read</span>. Codex fragt beim Lesen
                  nicht nach; beton kann solche Zugriffe nur protokollieren. Die Sandbox maskiert <span className="font-mono">.env</span> und
                  <span className="font-mono"> ~/.ssh</span> trotzdem.
                </p>
                <p className="text-muted-foreground">
                  Ereignis <span className="font-mono">policy.enforcement_degraded</span>
                </p>
              </Callout>
              <ToolCall kind="read" name="Lesen" target="src/middleware/rate-limit.ts" policy="nur beobachtet" duration="0,1 s" />
              <ToolCall kind="shell" name="Shell" target="pnpm vitest run rate-limit" policy="Freigabe-fähig" duration="3,9 s" />
            </F>
          )}
        </div>
      </div>
    </Shell>
  )
}
