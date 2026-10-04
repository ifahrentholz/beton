import type { Attachment, Event } from '@beton/sdk'
import type { SessionLog } from '@/store/events'
import { isInline, isOffloaded, textOf } from './events'

/** Status einer Tool-Card (WEB-002 AC2). */
export type ToolState = 'requested' | 'running' | 'completed' | 'failed' | 'denied'

export type Item =
  | { kind: 'user'; key: string; text: string; attachments: Attachment[] }
  | { kind: 'assistant'; key: string; text: string; streaming: boolean }
  | { kind: 'reasoning'; key: string; text: string; streaming: boolean }
  | {
      kind: 'tool'
      key: string
      callId: string
      tool: string
      args: unknown
      status: ToolState
      durationMs?: number
      output?: string
      result?: unknown
    }
  | {
      kind: 'approval'
      key: string
      approvalId: string
      tool: string
      args: unknown
      decision?: string
      comment?: string
    }
  | { kind: 'policy'; key: string; rule: string; reason: string }
  | { kind: 'error'; key: string; title: string; detail?: string }
  | { kind: 'note'; key: string; text: string; tone: 'neutral' | 'deny' | 'ok' }
  | { kind: 'offloaded'; key: string; eventType: string; ref: string; seq: number }

function toolState(status: string): ToolState {
  switch (status) {
    case 'ok':
      return 'completed'
    case 'denied':
      return 'denied'
    default:
      return 'failed'
  }
}

function problemText(problem: unknown): { title: string; detail?: string } {
  const p = (problem ?? {}) as { title?: unknown; detail?: unknown }
  const title = typeof p.title === 'string' ? p.title : 'Fehler'
  return typeof p.detail === 'string' ? { title, detail: p.detail } : { title }
}

/** Erste passende Regel einer Policy-Entscheidung (Name und Begründung). */
function policyRule(matched: unknown[]): { rule: string; reason: string } {
  const m = (matched[0] ?? {}) as Record<string, unknown>
  const pick = (...keys: string[]) => {
    for (const k of keys) {
      const v = m[k]
      if (typeof v === 'string' && v) return v
    }
    return undefined
  }
  return {
    rule: pick('rule', 'name', 'id', 'rule_id') ?? 'unbenannte Regel',
    reason: pick('reason', 'message', 'explain') ?? 'ohne Begründung',
  }
}

/** Anhänge einer Nachricht (Inhaltsblöcke `{"type": "attachment", …}`, WEB-006). */
export function attachmentsOf(content: unknown[]): Attachment[] {
  return content.flatMap((b) => {
    const a = (b ?? {}) as Partial<Attachment> & { type?: unknown }
    if (a.type !== 'attachment' || typeof a.blob !== 'string' || typeof a.name !== 'string' || typeof a.mime !== 'string') return []
    return [{ blob: a.blob, name: a.name, mime: a.mime, size: Number(a.size ?? 0) }]
  })
}

/**
 * Leitet die Elemente des Chat-Streams aus dem Log ab (WEB-002, WEB-018). Rein und ohne
 * Seiteneffekte; laufende Deltas erscheinen am Ende.
 */
export function timeline(log: Pick<SessionLog, 'events' | 'streaming' | 'reasoning' | 'toolOutput'>): Item[] {
  const items: Item[] = []
  const tools = new Map<string, Extract<Item, { kind: 'tool' }>>()
  const approvals = new Map<string, Extract<Item, { kind: 'approval' }>>()

  for (const e of log.events as Event[]) {
    const key = `e${e.seq}`
    if (isOffloaded(e)) {
      items.push({ kind: 'offloaded', key, eventType: e.type, ref: e.payload_ref, seq: e.seq })
      continue
    }
    if (!isInline(e)) continue
    switch (e.type) {
      case 'message.completed': {
        const text = textOf(e.payload.content)
        if (e.payload.role === 'user') items.push({ kind: 'user', key, text, attachments: attachmentsOf(e.payload.content) })
        else if (e.payload.role === 'assistant' && text) items.push({ kind: 'assistant', key, text, streaming: false })
        break
      }
      case 'reasoning.completed': {
        const text = e.payload.redacted ? '(redigiert)' : (e.payload.summary ?? '')
        if (text) items.push({ kind: 'reasoning', key, text, streaming: false })
        break
      }
      case 'tool.call.requested': {
        const item: Extract<Item, { kind: 'tool' }> = {
          kind: 'tool',
          key,
          callId: e.payload.call_id,
          tool: e.payload.tool,
          args: e.payload.args,
          status: 'requested',
        }
        tools.set(e.payload.call_id, item)
        items.push(item)
        break
      }
      case 'tool.call.started': {
        const t = tools.get(e.payload.call_id)
        if (t && t.status === 'requested') t.status = 'running'
        break
      }
      case 'tool.call.completed': {
        const t = tools.get(e.payload.call_id)
        if (t) {
          t.status = toolState(e.payload.status)
          t.durationMs = e.payload.duration_ms
          t.result = e.payload.result
        }
        break
      }
      case 'approval.requested': {
        const subject = (e.payload.subject ?? {}) as { tool?: string; args?: unknown }
        const item: Extract<Item, { kind: 'approval' }> = {
          kind: 'approval',
          key,
          approvalId: e.payload.approval_id,
          tool: subject.tool ?? e.payload.kind,
          args: subject.args ?? e.payload.subject,
        }
        approvals.set(e.payload.approval_id, item)
        items.push(item)
        break
      }
      case 'approval.resolved': {
        const a = approvals.get(e.payload.approval_id)
        if (a) {
          a.decision = e.payload.decision
          if (e.payload.comment) a.comment = e.payload.comment
        }
        break
      }
      case 'policy.decision':
        if (e.payload.outcome === 'deny') {
          items.push({ kind: 'policy', key, ...policyRule(e.payload.matched) })
        }
        break
      case 'turn.failed':
        items.push({ kind: 'error', key, ...problemText(e.payload.problem) })
        break
      case 'error':
        items.push({ kind: 'error', key, ...problemText(e.payload.problem) })
        break
      case 'harness.exited':
        items.push({
          kind: 'error',
          key,
          title: `Harness beendet (Exit-Code ${e.payload.code ?? '?'})`,
          detail: e.payload.stderr_tail.split('\n').slice(-5).join('\n') || undefined,
        })
        break
      case 'harness.auth_required':
        items.push({
          kind: 'error',
          key,
          title: `${e.payload.harness} ist nicht angemeldet`,
          detail: `Im Terminal anmelden: ${e.payload.hint}`,
        })
        break
      case 'harness.incompatible':
        items.push({
          kind: 'error',
          key,
          title: `Version ${e.payload.detected_version} wird nicht unterstützt`,
          detail: `Erwartet: ${e.payload.expected_range}`,
        })
        break
      case 'turn.interrupted':
        items.push({ kind: 'note', key, text: 'Turn unterbrochen', tone: 'neutral' })
        break
      case 'session.resumed':
        items.push({ kind: 'note', key, text: 'Session fortgesetzt', tone: 'ok' })
        break
      case 'notice':
        items.push({ kind: 'note', key, text: e.payload.text, tone: 'neutral' })
        break
      default:
        break
    }
  }
  for (const [callId, output] of Object.entries(log.toolOutput)) {
    const t = tools.get(callId)
    if (t) t.output = output
  }
  for (const [id, text] of Object.entries(log.reasoning)) {
    items.push({ kind: 'reasoning', key: `r-${id}`, text, streaming: true })
  }
  for (const [id, text] of Object.entries(log.streaming)) {
    items.push({ kind: 'assistant', key: `s-${id}`, text, streaming: true })
  }
  return items
}

/** Offene Freigaben (für Status und Tastenkürzel). */
export function openApprovals(items: Item[]): Extract<Item, { kind: 'approval' }>[] {
  return items.filter((i): i is Extract<Item, { kind: 'approval' }> => i.kind === 'approval' && !i.decision)
}
