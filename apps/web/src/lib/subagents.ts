import type { Event, SubagentNode } from '@beton/sdk'
import { cost } from './format'

// --- Sub-Agent-Graph (WEB-012) ---------------------------------------------------------------

/** Events im Stream des Parents, nach denen der Baum neu geladen wird. */
const TREE_EVENTS = new Set(['agent.spawned', 'agent.completed', 'agent.message'])

/**
 * Version des Session-Baums aus dem Stream des Parents: `seq` des letzten `agent.*`-Events.
 * Ein neues `agent.spawned` lädt den Baum sofort neu (AC1), statt auf die Abfrage zu warten.
 */
export function treeVersion(events: readonly Event[] | undefined): number {
  for (let i = (events?.length ?? 0) - 1; i >= 0; i--) {
    const e = events![i]!
    if (TREE_EVENTS.has(e.type)) return e.seq
  }
  return 0
}

/** Abstand der Abfragen, solange der Agents-Tab offen ist (Status und Kosten der Childs). */
export const TREE_POLL_MS = 1000

/** Maße eines Knotens wie im Prototyp (`screens/workspace/agents.tsx`). */
export const NODE_W = 250
export const NODE_H = 76
const GAP_X = 60
const GAP_Y = 20

export interface Placed {
  id: string
  x: number
  y: number
}

/**
 * Baum-Layout von links nach rechts: Tiefe → Spalte, Blätter untereinander, Eltern mittig zu
 * ihren Kindern. Knoten ohne bekannten Parent hängen an der Wurzel.
 */
export function layoutTree(nodes: readonly SubagentNode[]): Placed[] {
  if (nodes.length === 0) return []
  const root = nodes[0]!
  const ids = new Set(nodes.map((n) => n.id))
  const children = new Map<string, SubagentNode[]>()
  for (const n of nodes.slice(1)) {
    const parent = n.parent_id && ids.has(n.parent_id) ? n.parent_id : root.id
    children.set(parent, [...(children.get(parent) ?? []), n])
  }
  const placed = new Map<string, Placed>()
  let row = 0
  const visit = (n: SubagentNode, depth: number): number => {
    const kids = children.get(n.id) ?? []
    const x = depth * (NODE_W + GAP_X)
    if (kids.length === 0) {
      const y = row * (NODE_H + GAP_Y)
      row += 1
      placed.set(n.id, { id: n.id, x, y })
      return y
    }
    const ys = kids.map((k) => visit(k, depth + 1))
    const y = (Math.min(...ys) + Math.max(...ys)) / 2
    placed.set(n.id, { id: n.id, x, y })
    return y
  }
  visit(root, 0)
  return nodes.map((n) => placed.get(n.id)).filter((p): p is Placed => p !== undefined)
}

function tokens(n: number): string {
  return `${n.toLocaleString('de-DE')} Tokens`
}

/** Herkunft der Abrechnung wie im Prototyp. */
function authLabel(auth: string | undefined): string | undefined {
  switch (auth) {
    case 'vendor_cli':
      return 'Subscription'
    case 'api_key':
      return 'API-Key'
    default:
      return undefined
  }
}

/**
 * Kumulierte Kosten eines Knotens (AC2): Tokens samt Nachfahren und, falls Geld anfiel, der
 * Betrag; bei Subscription „Subscription“ statt eines Betrags (HAR-021).
 */
export function costLabel(n: SubagentNode): string {
  const parts = [tokens(n.subtree_tokens)]
  if (n.subtree_cost_micro > 0) parts.push(cost(n.subtree_cost_micro))
  else {
    const auth = authLabel(n.auth_source)
    if (auth) parts.push(auth)
  }
  return parts.join(' · ')
}

/** Kopfzeile, z. B. „4 Sessions · 62.480 Tokens · alles über Subscriptions“. */
export function treeSummary(nodes: readonly SubagentNode[]): string {
  if (nodes.length <= 1) return 'Keine Sub-Agents'
  const root = nodes[0]!
  const parts = [`${nodes.length} Sessions`, tokens(root.subtree_tokens)]
  if (root.subtree_cost_micro > 0) parts.push(cost(root.subtree_cost_micro))
  if (nodes.every((n) => n.auth_source === undefined || n.auth_source === 'vendor_cli')) parts.push('alles über Subscriptions')
  return parts.join(' · ')
}

/** Status eines Knotens: ein laufender Auftrag zählt als „läuft“, auch wenn die Session kurz idle meldet. */
export function nodeStatus(n: SubagentNode): string {
  if (n.task === 'running' && n.status === 'idle') return 'running'
  return n.status
}

/** Kurzer Text zum Auftrag des Parents. */
export function taskLabel(task: string | undefined): string | undefined {
  switch (task) {
    case 'running':
      return 'arbeitet'
    case 'completed':
      return 'fertig'
    case 'failed':
      return 'fehlgeschlagen'
    case 'interrupted':
      return 'unterbrochen'
    case 'cancelled':
      return 'abgebrochen'
    default:
      return undefined
  }
}
