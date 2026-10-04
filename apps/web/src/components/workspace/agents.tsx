import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import type { Event } from '@beton/sdk'
import { client } from '@/lib/client'
import { TREE_POLL_MS, treeSummary, treeVersion } from '@/lib/subagents'
import { Callout } from './ui'

/** xyflow nur mit dem Agents-Tab laden (WEB-015). */
const AgentsGraph = lazy(() => import('./agents-graph'))

/** So lange gilt ein Knoten als neu (gestrichelt, „neu“). */
const FRESH_MS = 5000

/**
 * Agents-Tab der Workspace-Rail (WEB-012): die Session und ihre Sub-Agents als Graph mit
 * Harness, Status und kumulierten Kosten. Neu geladen bei `agent.*`-Events im Stream der
 * Session (AC1) und jede Sekunde, solange der Tab offen ist. Klick öffnet die Sub-Session.
 */
export function AgentsPanel({ sessionId, events }: { sessionId: string; events: readonly Event[] | undefined }) {
  const navigate = useNavigate()
  const version = treeVersion(events)
  const q = useQuery({
    queryKey: ['subagents', sessionId, version],
    queryFn: () => client.session(sessionId).subagents(),
    refetchInterval: TREE_POLL_MS,
    placeholderData: keepPreviousData,
    retry: false,
  })
  const nodes = useMemo(() => q.data?.nodes ?? [], [q.data])

  // Knoten, die nach dem ersten Laden erscheinen, sind eine Weile „neu“.
  const known = useRef<Set<string> | undefined>(undefined)
  const [fresh, setFresh] = useState<ReadonlySet<string>>(new Set())
  useEffect(() => {
    if (!q.data) return
    const ids = q.data.nodes.map((n) => n.id)
    if (!known.current) {
      known.current = new Set(ids)
      return
    }
    const added = ids.filter((id) => !known.current!.has(id))
    if (added.length === 0) return
    for (const id of added) known.current.add(id)
    setFresh((s) => new Set([...s, ...added]))
    const t = setTimeout(() => setFresh((s) => new Set([...s].filter((id) => !added.includes(id)))), FRESH_MS)
    return () => clearTimeout(t)
  }, [q.data])

  const open = useCallback(
    (id: string) => {
      if (id !== sessionId) void navigate({ to: '/s/$sessionId', params: { sessionId: id } })
    },
    [navigate, sessionId],
  )

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="agents-panel">
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        <span className="text-[12px] font-medium">Sub-Agents</span>
        <span className="ml-auto text-[12px] text-muted-foreground" data-testid="agents-summary">
          {q.data ? treeSummary(nodes) : ''}
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {q.isError && !q.data ? (
          <div className="p-3">
            <Callout tone="deny">Session-Baum nicht geladen.</Callout>
          </div>
        ) : !q.data ? (
          <p className="px-3 py-4 text-[13px] text-muted-foreground">Lädt …</p>
        ) : nodes.length <= 1 ? (
          <div className="px-6 py-16 text-center">
            <p className="text-[14px] font-medium">Noch keine Sub-Agents</p>
            <p className="mx-auto mt-1 max-w-sm text-[13px] text-muted-foreground">
              Wenn der Agent Teilaufgaben abgibt, etwa Tests oder ein Review auf einem anderen Harness, erscheinen sie hier als Knoten. Klick öffnet die Sub-Session.
            </p>
          </div>
        ) : (
          <div className="border-b border-border p-3">
            <Suspense fallback={<p className="py-4 text-[13px] text-muted-foreground">Lädt …</p>}>
              <AgentsGraph nodes={nodes} fresh={fresh} onOpen={open} />
            </Suspense>
          </div>
        )}
      </div>
    </div>
  )
}
