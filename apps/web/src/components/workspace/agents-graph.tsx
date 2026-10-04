import { useEffect, useMemo, useRef } from 'react'
import { Handle, Position, ReactFlow, type Edge, type Node, type NodeProps, type ReactFlowInstance } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import type { SubagentNode } from '@beton/sdk'
import { layoutTree, NODE_H, NODE_W } from '@/lib/subagents'
import { AgentNodeCard } from './agent-node'

/*
 * Graph der Session-Hierarchie mit xyflow (WEB-012). Dieses Modul (und damit xyflow) wird nur
 * über `React.lazy` aus dem Agents-Tab geladen und liegt nicht im initialen Bundle (WEB-015).
 */

type FlowData = { node: SubagentNode; isNew: boolean; self: boolean; onOpen: (id: string) => void }

function SessionFlowNode({ data }: NodeProps<Node<FlowData>>) {
  return (
    <div style={{ width: NODE_W }}>
      <Handle type="target" position={Position.Left} className="!size-1 !min-h-0 !min-w-0 !border-0 !bg-transparent" isConnectable={false} />
      <AgentNodeCard node={data.node} isNew={data.isNew} self={data.self} onOpen={data.onOpen} />
      <Handle type="source" position={Position.Right} className="!size-1 !min-h-0 !min-w-0 !border-0 !bg-transparent" isConnectable={false} />
    </div>
  )
}

const nodeTypes = { session: SessionFlowNode }

const ignoreClick = () => undefined

const FIT = { padding: 0.06, maxZoom: 1 }

export default function AgentsGraph({
  nodes,
  fresh,
  onOpen,
}: {
  nodes: readonly SubagentNode[]
  /** IDs der Knoten, die gerade erst erschienen sind (gestrichelt, „neu“). */
  fresh: ReadonlySet<string>
  onOpen: (id: string) => void
}) {
  const { flowNodes, flowEdges, height } = useMemo(() => {
    const placed = layoutTree(nodes)
    const byId = new Map(placed.map((p) => [p.id, p]))
    const rootId = nodes[0]?.id
    const flowNodes: Node<FlowData>[] = nodes.map((n) => ({
      id: n.id,
      type: 'session',
      position: { x: byId.get(n.id)?.x ?? 0, y: byId.get(n.id)?.y ?? 0 },
      data: { node: n, isNew: fresh.has(n.id), self: n.id === rootId, onOpen },
      draggable: false,
      selectable: false,
      connectable: false,
    }))
    const flowEdges: Edge[] = nodes
      .filter((n) => n.id !== rootId)
      .map((n) => ({
        id: `${n.parent_id ?? rootId}-${n.id}`,
        source: n.parent_id ?? rootId!,
        target: n.id,
        type: 'smoothstep',
        style: { stroke: 'var(--border)', strokeWidth: 1.5, strokeDasharray: fresh.has(n.id) ? '4 3' : undefined },
      }))
    const bottom = Math.max(0, ...placed.map((p) => p.y)) + NODE_H
    return { flowNodes, flowEdges, height: Math.min(560, Math.max(180, bottom + 40)) }
  }, [nodes, fresh, onOpen])

  // `fitView` wirkt nur beim ersten Rendern; neue Knoten (live, AC1) bringen den Graphen neu
  // in den sichtbaren Bereich, sobald xyflow sie vermessen hat.
  const flow = useRef<ReactFlowInstance<Node<FlowData>> | undefined>(undefined)
  const shape = flowNodes.map((n) => n.id).join(',')
  useEffect(() => {
    const frame = requestAnimationFrame(() => void flow.current?.fitView(FIT))
    return () => cancelAnimationFrame(frame)
  }, [shape, height])

  return (
    <div className="relative" style={{ height }} data-testid="agents-graph">
      <ReactFlow
        onInit={(instance) => {
          flow.current = instance
        }}
        nodes={flowNodes}
        edges={flowEdges}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={FIT}
        minZoom={0.3}
        maxZoom={1.5}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        // Ohne Klick-Handler setzt xyflow `pointer-events: none` auf nicht ziehbare Knoten; den
        // Klick selbst (auch per Tastatur) behandelt die Karte.
        onNodeClick={ignoreClick}
        zoomOnScroll={false}
        panOnScroll
        proOptions={{ hideAttribution: true }}
        colorMode="system"
        className="!bg-transparent"
      />
    </div>
  )
}
