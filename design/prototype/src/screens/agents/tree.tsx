import { GitBranch } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, StatusMark } from '@/app/harness'
import { Composer, SessionHeader, WorkspaceRail } from '@/app/session-chrome'
import { AgentMessage, SystemNote, ToolCall, UserMessage } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { HarnessId, SessionStatus } from '@/mock/data'
import { Btn, Pill } from '@/screens/harnesses/kit'

type Node = { name: string; harness: HarnessId; model: string; status: SessionStatus; statusText: string; branch?: string; note: string; async?: boolean; children?: Node[] }

function TreeNode({ n, depth = 0, self }: { n: Node; depth?: number; self?: boolean }) {
  return (
    <li>
      <div className={cn('relative py-2 pr-3', depth > 0 ? 'pl-7' : 'pl-3', self && 'bg-accent')}>
        {depth > 0 && <span aria-hidden className="absolute top-0 left-4 h-5 w-2.5 rounded-bl-[3px] border-b border-l border-border" />}
        <div className="flex items-center gap-2 text-[13px]">
          <StatusMark status={n.status} />
          <span className="font-medium">{n.name}</span>
          {self && <span className="text-[11px] text-muted-foreground">diese Session</span>}
          {n.async && <Pill>im Hintergrund</Pill>}
          <span className="ml-auto text-[11px] text-muted-foreground">{n.statusText}</span>
        </div>
        <div className="mt-0.5 flex items-center gap-2 pl-4">
          <HarnessBadge id={n.harness} model={n.model} />
        </div>
        {n.branch && (
          <div className="flex items-center gap-1 pl-4 font-mono text-[11px] text-muted-foreground">
            <GitBranch className="size-3" /> {n.branch}
          </div>
        )}
        <div className="pl-4 text-[11.5px] text-muted-foreground">{n.note}</div>
      </div>
      {n.children && (
        <ul className={cn(depth > 0 && 'pl-4')}>
          {n.children.map((c) => (
            <TreeNode key={c.name} n={c} depth={depth + 1} />
          ))}
        </ul>
      )}
    </li>
  )
}

function TreePanel({ state }: { state: string }) {
  const cancelled = state === 'parent-cancelled'
  const done = state === 'async-result'
  const root: Node = {
    name: 'pr-fixer',
    harness: 'claude',
    model: 'claude-sonnet-5-5',
    status: cancelled ? 'stopped' : 'running',
    statusText: cancelled ? 'abgebrochen' : 'läuft',
    branch: 'fix/checkout-timeout',
    note: 'Parent · Projekt-Agent',
    children: [
      {
        name: 'quick-check #1',
        harness: 'codex',
        model: 'gpt-5.3-codex',
        status: cancelled ? 'stopped' : done ? 'idle' : 'running',
        statusText: cancelled ? 'mit Parent beendet' : done ? 'fertig · 3 Punkte' : 'läuft · 4 Min.',
        branch: 'beton/pr-fixer-k3/quick-check-1',
        note: 'Reviewt den Diff · nur planen · eigener Worktree',
        async: true,
      },
      {
        name: 'quick-check #2',
        harness: 'codex',
        model: 'gpt-5.3-codex',
        status: cancelled ? 'stopped' : 'running',
        statusText: cancelled ? 'mit Parent beendet' : 'läuft · 2 Min.',
        branch: 'beton/pr-fixer-k3/quick-check-2',
        note: 'Prüft die geänderten e2e-Tests',
        async: true,
      },
      ...(state === 'spawn-denied'
        ? [
            {
              name: 'quick-check #3',
              harness: 'codex' as const,
              model: 'gpt-5.3-codex',
              status: 'running' as const,
              statusText: 'läuft · 1 Min.',
              branch: 'beton/pr-fixer-k3/quick-check-3',
              note: 'Prüft die Fixture-Daten',
              async: true,
            },
          ]
        : []),
    ],
  }
  return (
    <F id={['AGT-009', 'ASY-002']} className="flex h-full flex-col">
      <div className="flex items-baseline gap-2 border-b border-border px-3 py-2 text-[12px]">
        <span className="font-medium">Session-Baum</span>
        <span className="text-muted-foreground">{state === 'spawn-denied' ? 4 : 3} Sessions · 2 Harnesses</span>
        <Btn variant="ghost" size="sm" className="ml-auto">
          Als Partitur
        </Btn>
      </div>
      <ul className="min-h-0 flex-1 overflow-y-auto">
        <TreeNode n={root} self />
      </ul>
      <div className="space-y-1 border-t border-border px-3 py-2.5 text-[11.5px] text-muted-foreground">
        <div>Tiefe 1 von 2 · {cancelled ? 0 : done ? 1 : state === 'spawn-denied' ? 3 : 2} von 3 gleichzeitig · je Kind höchstens 50 % des Budgets</div>
        <div>Kosten und Laufzeit der Kinder zählen zum Lauf von pr-fixer.</div>
      </div>
    </F>
  )
}

export function SubAgentTree({ state }: { state: string }) {
  const cancelled = state === 'parent-cancelled'
  return (
    <AppLayout
      activeSession="ses_7f3k"
      rail={
        <WorkspaceRail active="agents" width="w-[380px]">
          <TreePanel state={state} />
        </WorkspaceRail>
      }
    >
      <SessionHeader title="pr-fixer · CI auf fix/checkout-timeout reparieren" harness="claude" model="claude-sonnet-5-5" status={cancelled ? 'stopped' : 'running'} branch="fix/checkout-timeout" />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6">
          <UserMessage>CI auf fix/checkout-timeout ist rot (e2e: checkout.spec.ts). Bitte beheben.</UserMessage>
          <F id={['AGT-009', 'AGT-007']}>
            <div className="flex flex-col gap-1">
              <ToolCall kind="other" name="session_spawn" target="quick-check · Codex · im Hintergrund · eigener Worktree" duration="0,8 s" sandbox={false} />
              <ToolCall kind="other" name="session_spawn" target="quick-check · Codex · im Hintergrund · eigener Worktree" duration="0,7 s" sandbox={false} />
            </div>
          </F>
          <AgentMessage harness="claude">
            <p>Zwei Codex-Reviews laufen parallel. Ich reproduziere währenddessen den e2e-Fehler lokal.</p>
          </AgentMessage>
          <ToolCall kind="shell" name="Shell" target="pnpm playwright test checkout.spec.ts" duration="38 s" status="failed" />
          <ToolCall kind="edit" name="Bearbeiten" target="e2e/checkout.spec.ts" duration="0,3 s" />

          {state === 'spawn-denied' && (
            <F id="AGT-009">
              <ToolCall kind="other" name="session_spawn" target="quick-check · Codex" status="denied" sandbox={false}>
                <p className="text-[13px] text-muted-foreground">Abgelehnt: Es laufen schon 3 Sub-Agents gleichzeitig (Grenze max_concurrent: 3).</p>
              </ToolCall>
              <p className="mt-1 ml-9 text-[12px] text-muted-foreground">
                Sub-Agent abgelehnt: 3 von 3 gleichzeitig laufen bereits. Der Agent wartet mit session_wait, bis einer fertig ist.
              </p>
            </F>
          )}

          {state === 'async-result' && (
            <F id={['ASY-002', 'AGT-009']}>
              <SystemNote>Sub-Agent quick-check #1 (Codex) fertig · Ergebnis zugestellt, pr-fixer wurde geweckt</SystemNote>
              <div className="mt-4">
                <UserMessage author="beton · Sub-Agent-Ergebnis">
                  <div className="rounded-md border border-border bg-card p-3 text-[13px]">
                    <p className="font-medium">quick-check #1 (Codex, gpt-5.3-codex) · 3 Punkte, 1 blockierend</p>
                    <ol className="mt-1 list-decimal space-y-0.5 pl-5 text-muted-foreground">
                      <li>Blockierend: waitForResponse ohne Timeout kann den Test hängen lassen.</li>
                      <li>Der Selektor hängt am Button-Text; besser getByRole.</li>
                      <li>Fixture-Daten doppelt in zwei Specs.</li>
                    </ol>
                  </div>
                </UserMessage>
              </div>
              <div className="mt-4">
                <AgentMessage harness="claude" streaming>
                  <p>Punkt 1 ist berechtigt. Ich setze ein explizites Timeout von 10 s und stelle den Selektor auf getByRole um.</p>
                </AgentMessage>
              </div>
            </F>
          )}

          {cancelled && (
            <F id="AGT-009">
              <SystemNote tone="deny">Von dir abgebrochen · 2 laufende Sub-Agents wurden mitbeendet</SystemNote>
            </F>
          )}
        </div>
      </div>
      <Composer harness="claude" running={!cancelled} />
    </AppLayout>
  )
}
