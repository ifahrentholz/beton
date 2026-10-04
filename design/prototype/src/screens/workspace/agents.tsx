import type { CSSProperties } from 'react'
import { WorkspaceRail } from '@/app/session-chrome'
import { HarnessBadge, StatusMark } from '@/app/harness'
import { Score, type ScoreVoice } from '@/app/score'
import { SystemNote } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import type { HarnessId, SessionStatus } from '@/mock/data'
import { Segmented, SessionShell, ShortStream, Tag } from '@/app/kit/workspace'

type Node = { id: string; title: string; agent: string; harness: HarnessId; model: string; status: SessionStatus; cost: string; isNew?: boolean }

function nodesFor(state: string): { root: Node; children: Node[] } {
  const done = state === 'done'
  return {
    root: { id: 'root', title: 'Rate-Limiter für die Login-API', agent: 'implementer', harness: 'claude', model: 'claude-opus-5-5', status: done ? 'idle' : 'running', cost: '38.410 Tokens · Subscription' },
    children: [
      { id: 'tests', title: 'Tests für rate-limit.ts', agent: 'test-writer', harness: 'claude', model: 'claude-sonnet-5-5', status: 'idle', cost: '12.900 Tokens · Subscription' },
      { id: 'review', title: 'Review: Rate-Limiter', agent: 'reviewer', harness: 'codex', model: 'gpt-5.3-codex', status: done ? 'idle' : 'waiting', cost: '9.120 Tokens · Subscription' },
      { id: 'docs', title: 'API-Doku ergänzen', agent: 'docs', harness: 'gemini', model: 'gemini-3-pro', status: done ? 'idle' : 'running', cost: '2.050 Tokens · Google-Konto', isNew: state === 'live' },
    ],
  }
}

function NodeBox({ n, style }: { n: Node; style: CSSProperties }) {
  const waiting = n.status === 'waiting'
  return (
    <button
      style={style}
      className={cn(
        'absolute flex flex-col gap-1 border bg-card px-2.5 py-2 text-left hover:border-foreground/50',
        waiting ? 'chamfer border-signal bg-signal-soft' : 'rounded-md border-border',
        n.isNew && 'outline-2 outline-offset-2 outline-foreground/40 outline-dashed',
      )}
      title="Sub-Session öffnen"
    >
      <span className="flex items-center gap-2">
        <StatusMark status={n.status} />
        <span className="truncate text-[13px] font-medium">{n.title}</span>
        {n.isNew && <Tag className="ml-auto">neu</Tag>}
      </span>
      <HarnessBadge id={n.harness} model={n.model} className="text-[11px]" />
      <span className="flex items-center gap-2 text-[11px] text-muted-foreground">
        <span>Agent {n.agent}</span>
        <span className="ml-auto tabular-nums">{n.cost}</span>
      </span>
    </button>
  )
}

function Graph({ state }: { state: string }) {
  const { root, children } = nodesFor(state)
  const rootTop = 104
  const tops = [8, 104, 200]
  return (
    <div className="relative h-[290px]">
      <svg className="absolute inset-0 h-full w-full" aria-hidden>
        {children.map((c, i) => {
          const y = tops[i] + 38
          return <path key={c.id} d={`M232 ${rootTop + 38} H262 V${y} H292`} fill="none" stroke="var(--border)" strokeWidth="1.5" strokeDasharray={c.isNew ? '4 3' : undefined} />
        })}
      </svg>
      <NodeBox n={root} style={{ left: 12, top: rootTop, width: 220 }} />
      {children.map((c, i) => (
        <NodeBox key={c.id} n={c} style={{ left: 292, top: tops[i], width: 270 }} />
      ))}
    </div>
  )
}

const voices: ScoreVoice[] = [
  { id: 'impl', name: 'implementer', role: 'Claude Code · opus', voice: 'claude', events: [{ kind: 'turn', start: 0, end: 6 }, { kind: 'handoff', at: 6, to: 'tests' }, { kind: 'handoff', at: 9, to: 'review' }, { kind: 'turn', start: 10, end: 14 }] },
  { id: 'tests', name: 'test-writer', role: 'Claude Code · sonnet', voice: 'claude', events: [{ kind: 'turn', start: 6, end: 9 }, { kind: 'tool', at: 8, label: 'pnpm vitest' }] },
  { id: 'review', name: 'reviewer', role: 'Codex', voice: 'codex', events: [{ kind: 'turn', start: 9, end: 12 }, { kind: 'fermata', at: 12, label: 'Freigabe: git push' }] },
  { id: 'docs', name: 'docs', role: 'Gemini CLI', voice: 'acp', events: [{ kind: 'turn', start: 13, end: 14 }] },
]

export function AgentsScreen({ state }: { state: string }) {
  return (
    <SessionShell
      sessionList={false}
      status={state === 'done' ? 'idle' : 'running'}
      rail={
        <WorkspaceRail active="agents" width="w-[600px]">
          <F id="WEB-012" className="flex flex-col">
            <div className="flex items-center gap-2 border-b border-border px-3 py-2">
              <Segmented
                value="graph"
                items={[
                  { id: 'graph', label: 'Graph' },
                  { id: 'score', label: 'Partitur' },
                ]}
              />
              <span className="ml-auto text-[12px] text-muted-foreground">
                {state === 'empty' ? 'Keine Sub-Agents' : '4 Sessions · 62.480 Tokens · alles über Subscriptions'}
              </span>
            </div>
            {state === 'empty' ? (
              <div className="px-6 py-16 text-center">
                <p className="text-[14px] font-medium">Noch keine Sub-Agents</p>
                <p className="mx-auto mt-1 max-w-sm text-[13px] text-muted-foreground">
                  Wenn der Agent Teilaufgaben abgibt, etwa Tests oder ein Review auf einem anderen Harness, erscheinen sie hier als Knoten. Klick öffnet die Sub-Session.
                </p>
              </div>
            ) : (
              <>
                <div className="border-b border-border p-3">
                  <Graph state={state} />
                </div>
                <div className="p-3">
                  <div className="mb-1 text-[12px] font-medium">Ablauf</div>
                  <Score voices={voices} length={15} now={state === 'done' ? undefined : 13.6} ticks={[{ at: 0, label: '14:00' }, { at: 5, label: '14:05' }, { at: 10, label: '14:10' }]} />
                </div>
              </>
            )}
          </F>
        </WorkspaceRail>
      }
    >
      <ShortStream compact />
      {state === 'live' && <SystemNote>implementer hat „API-Doku ergänzen“ an docs (Gemini CLI) abgegeben</SystemNote>}
    </SessionShell>
  )
}
