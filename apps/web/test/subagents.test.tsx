import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { SubagentNode } from '@beton/sdk'
import { AgentNodeCard } from '@/components/workspace/agent-node'
import { costLabel, layoutTree, NODE_W, nodeStatus, treeSummary, treeVersion } from '@/lib/subagents'
import { ev, resetSeq } from './fixtures'

afterEach(() => cleanup())

function node(over: Partial<SubagentNode> & { id: string }): SubagentNode {
  return {
    depth: 1,
    title: over.id,
    harness: 'claude',
    status: 'idle',
    cost_micro: 0,
    subtree_cost_micro: 0,
    tokens: 0,
    subtree_tokens: 0,
    created_at: '2026-10-04T12:00:00Z',
    ...over,
  }
}

const tree: SubagentNode[] = [
  node({ id: 'root', depth: 0, title: 'Rate-Limiter', agent: 'maestra', status: 'running', tokens: 38_410, subtree_tokens: 62_480, auth_source: 'vendor_cli' }),
  node({ id: 'a', parent_id: 'root', title: 'impl-claude', agent: 'impl-claude', task: 'completed', tokens: 12_900, subtree_tokens: 12_900, auth_source: 'vendor_cli' }),
  node({ id: 'b', parent_id: 'root', title: 'review-codex', agent: 'review-codex', harness: 'codex', model: 'gpt-5.3-codex', status: 'waiting_approval', task: 'running', tokens: 9_120, subtree_tokens: 11_170, auth_source: 'vendor_cli' }),
  node({ id: 'c', parent_id: 'b', depth: 2, title: 'docs', harness: 'acp:gemini', task: 'running', tokens: 2_050, subtree_tokens: 2_050, auth_source: 'api_key' }),
]

describe('Sub-Agent-Graph', () => {
  it('WEB-012 AC2: Knoten zeigen Harness-Icon, Status und kumulierte Kosten', () => {
    const onOpen = vi.fn()
    render(<AgentNodeCard node={tree[2]!} onOpen={onOpen} />)
    const card = screen.getByTestId('agent-node')
    expect(card.dataset.harness).toBe('codex')
    // Harness-Icon: Stimmfarbe der Familie plus Name und Modell.
    const dot = card.querySelector('span[aria-hidden]') as HTMLElement
    expect(dot.style.background).toBe('var(--voice-codex)')
    expect(card.textContent).toContain('Codex')
    expect(card.textContent).toContain('gpt-5.3-codex')
    // Status (Form + Text) und kumulierte Kosten des Teilbaums.
    expect(card.querySelector('[data-status]')?.getAttribute('data-status')).toBe('waiting')
    expect(screen.getByTestId('agent-node-cost').textContent).toBe('11.170 Tokens · Subscription')
    expect(card.textContent).toContain('Agent review-codex · arbeitet')
    fireEvent.click(card)
    expect(onOpen).toHaveBeenCalledWith('b')
  })

  it('WEB-012 AC2: mit Geld kumulierter Betrag statt Subscription; laufender Auftrag zählt als läuft', () => {
    const paid = node({ id: 'p', subtree_tokens: 1200, subtree_cost_micro: 250_000, auth_source: 'api_key' })
    expect(costLabel(paid)).toMatch(/^1\.200 Tokens · 0,25\s\$$/)
    expect(nodeStatus(node({ id: 'x', status: 'idle', task: 'running' }))).toBe('running')
    expect(nodeStatus(node({ id: 'y', status: 'idle', task: 'completed' }))).toBe('idle')
    expect(treeSummary(tree)).toBe('4 Sessions · 62.480 Tokens')
    expect(treeSummary(tree.slice(0, 3))).toBe('3 Sessions · 62.480 Tokens · alles über Subscriptions')
    expect(treeSummary(tree.slice(0, 1))).toBe('Keine Sub-Agents')
  })

  it('WEB-012 AC1: agent.spawned im Stream lädt den Baum sofort neu', () => {
    resetSeq()
    const events = [ev('turn.started', { turn_id: 'trn_1', author: 'usr_local' }), ev('message.completed', { role: 'assistant', content: [] })]
    expect(treeVersion(events)).toBe(0)
    events.push(ev('agent.spawned', { child_session_id: 'ses_c', agent_ref: 'maestra#impl-claude', harness: 'claude', async: true }))
    expect(treeVersion(events)).toBe(3)
    events.push(ev('tool.call.completed', { call_id: 'c', status: 'ok' }))
    expect(treeVersion(events)).toBe(3)
    events.push(ev('agent.completed', { child_session_id: 'ses_c', status: 'completed' }))
    expect(treeVersion(events)).toBe(5)
  })

  it('WEB-012: Baum-Layout von links nach rechts, Eltern mittig zu ihren Kindern', () => {
    const placed = layoutTree(tree)
    const at = (id: string) => placed.find((p) => p.id === id)!
    expect(at('root').x).toBe(0)
    expect(at('a').x).toBeGreaterThanOrEqual(NODE_W)
    expect(at('c').x).toBeGreaterThan(at('b').x)
    expect(at('a').y).toBeLessThan(at('c').y)
    expect(at('b').y).toBe(at('c').y)
    expect(at('root').y).toBe((at('a').y + at('b').y) / 2)
    // Unbekannter Parent: am Root.
    expect(layoutTree([tree[0]!, node({ id: 'z', parent_id: 'weg' })]).map((p) => p.x)).toEqual([0, NODE_W + 60])
    expect(layoutTree([])).toEqual([])
  })
})
