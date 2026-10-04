import type { SubagentNode } from '@beton/sdk'
import { HarnessBadge, listStatus, StatusMark } from '@/components/harness'
import { costLabel, nodeStatus, taskLabel } from '@/lib/subagents'
import { cn } from '@/lib/utils'
import { Tag } from './ui'

/**
 * Knoten des Sub-Agent-Graphen (WEB-012 AC2): Status, Titel, Harness mit Stimmfarbe und Modell,
 * Agent und kumulierte Kosten. Gestaltung wie `NodeBox` im Prototyp
 * (`screens/workspace/agents.tsx`); wartet die Session auf eine Freigabe, ist der Knoten
 * gelb abgeschrägt. Ohne xyflow, damit er sich ohne Graph testen lässt.
 */
export function AgentNodeCard({ node, isNew, self, onOpen }: { node: SubagentNode; isNew?: boolean; self?: boolean; onOpen?: (id: string) => void }) {
  const status = listStatus(nodeStatus(node))
  const waiting = status === 'waiting'
  const task = taskLabel(node.task)
  return (
    <button
      type="button"
      data-testid="agent-node"
      data-session={node.id}
      data-harness={node.harness}
      onClick={() => onOpen?.(node.id)}
      className={cn(
        'flex w-full flex-col gap-1 border bg-card px-2.5 py-2 text-left hover:border-foreground/50',
        waiting ? 'chamfer border-signal bg-signal-soft' : 'rounded-md border-border',
        isNew && 'outline-2 outline-offset-2 outline-foreground/40 outline-dashed',
      )}
      title={self ? 'Diese Session' : 'Sub-Session öffnen'}
    >
      <span className="flex min-w-0 items-center gap-2">
        <StatusMark status={status} />
        <span className="truncate text-[13px] font-medium">{node.title || node.agent || node.id}</span>
        {isNew && <Tag className="ml-auto">neu</Tag>}
      </span>
      <HarnessBadge harness={node.harness} model={node.model ?? null} className="text-[11px]" />
      <span className="flex items-center gap-2 text-[11px] text-muted-foreground">
        <span className="truncate">
          {node.agent ? `Agent ${node.agent}` : self ? 'diese Session' : 'Sub-Session'}
          {task && ` · ${task}`}
        </span>
        <span className="ml-auto shrink-0 tabular-nums" data-testid="agent-node-cost">
          {costLabel(node)}
        </span>
      </span>
    </button>
  )
}
