import type { Attachment, Compaction, Event } from '@beton/sdk'
import { harnessName } from '@/components/harness'
import { effortLabel } from '@/components/settings-picker'
import type { SessionLog } from '@/store/events'
import { isInline, isOffloaded, textOf } from './events'
import { argSummary } from './format'
import { t } from './i18n'

/** Ein Grund einer Approval-Card (POL-010). */
export interface ApprovalReason {
  ruleId: string
  scope: string
  set: string
  reason: string
  severity?: string
}

/** Wie eine Freigabe entschieden wurde (`approval.resolved`). */
export interface ApprovalResolution {
  decision: string
  via: string
  actor: unknown
  comment?: string
  answer?: string
  remember?: string
  onTimeoutApplied?: string
}

/** Freigabe bzw. Frage des Agents (WEB-007, WEB-018, AGT-007). */
export interface ApprovalItem {
  kind: 'approval'
  key: string
  approvalId: string
  /** `tool`, `policy`, `question` … */
  approvalKind: string
  title: string
  summary: string
  tool: string
  toolKind?: string
  args: unknown
  argsModified?: unknown
  reasons: ApprovalReason[]
  options: string[]
  expiresAt?: string
  onTimeout?: string
  question?: string
  choices: string[]
  /** Wer fragt (Harness, Agent). */
  requestedBy?: string
  decision?: string
  comment?: string
  resolution?: ApprovalResolution
}

/** Status einer Tool-Card (WEB-002 AC2). */
export type ToolState = 'requested' | 'running' | 'completed' | 'failed' | 'denied'

export type Item =
  | { kind: 'user'; key: string; text: string; attachments: Attachment[] }
  | { kind: 'assistant'; key: string; text: string; streaming: boolean; messageId?: string | undefined }
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
  | ApprovalItem
  | { kind: 'policy'; key: string; rule: string; reason: string }
  | { kind: 'error'; key: string; title: string; detail?: string }
  | { kind: 'note'; key: string; text: string; tone: 'neutral' | 'deny' | 'ok'; detail?: string }
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

/** Grund eines `event.rejected` in Klartext (Problem-Code aus PROTO-015). */
function rejectReason(code: string): string {
  switch (code) {
    case 'payload_too_large':
      return 'zu groß'
    case 'validation_failed':
      return 'ungültig'
    default:
      return code
  }
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

const EFFORT_RANK = ['low', 'medium', 'high', 'xhigh']

/**
 * Hinweise zu `session.settings_changed` (HAR-017, Screen `harness-switching`): Modellwechsel
 * live bzw. per Neustart und ein gemappter Effort. Andere Wechsel zeigt der Picker.
 */
export function settingsNotes(
  change: { model?: string; effort?: string; requested_effort?: string; mechanism?: string },
  ctx: { harness: string; previousModel?: string | null; turn: number },
): string[] {
  const notes: string[] = []
  if (change.model) {
    if (change.mechanism === 'restart') {
      notes.push(`Modell gewechselt auf ${change.model} · ${harnessName(ctx.harness)} wurde neu gestartet und hat die Session fortgesetzt`)
    } else {
      const what = ctx.previousModel && ctx.previousModel !== change.model ? `: ${ctx.previousModel} → ${change.model}` : ` auf ${change.model}`
      notes.push(`Modell gewechselt${what} · sofort wirksam, ab Turn ${ctx.turn} · Verlauf bleibt erhalten`)
    }
  }
  if (change.effort && change.requested_effort && change.requested_effort !== change.effort) {
    const who = change.model ?? ctx.previousModel ?? harnessName(ctx.harness)
    const higher = EFFORT_RANK.indexOf(change.requested_effort) > EFFORT_RANK.indexOf(change.effort)
    notes.push(
      `Effort: „${effortLabel(change.requested_effort)}“ angefragt, „${effortLabel(change.effort)}“ aktiv – ${who} unterstützt keine ${higher ? 'höhere' : 'niedrigere'} Stufe`,
    )
  }
  return notes
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
  // Nummer jedes Turns (für „ab Turn n“) sowie Harness und Modell für die Hinweise.
  const turnNumber = new Map<string, number>()
  for (const e of log.events as Event[]) {
    if (isInline(e) && e.type === 'turn.started') turnNumber.set(e.payload.turn_id, turnNumber.size + 1)
  }
  let turns = 0
  let harness = ''
  let model: string | null | undefined
  let compacting: Extract<Item, { kind: 'note' }> | undefined

  for (const e of log.events as Event[]) {
    const key = `e${e.seq}`
    if (isOffloaded(e)) {
      items.push({ kind: 'offloaded', key, eventType: e.type, ref: e.payload_ref, seq: e.seq })
      continue
    }
    if (!isInline(e)) continue
    switch (e.type) {
      case 'session.created':
        harness = e.payload.harness
        model = e.payload.model
        break
      case 'turn.started':
        turns += 1
        break
      case 'session.settings_changed': {
        const turn = (e.payload.effective_from_turn && turnNumber.get(e.payload.effective_from_turn)) || turns + 1
        settingsNotes(e.payload, { harness, previousModel: model, turn }).forEach((text, i) =>
          items.push({ kind: 'note', key: `${key}-${i}`, text, tone: 'neutral' }),
        )
        if (e.payload.model) model = e.payload.model
        break
      }
      case 'message.completed': {
        const text = textOf(e.payload.content)
        if (e.payload.role === 'user') items.push({ kind: 'user', key, text, attachments: attachmentsOf(e.payload.content) })
        else if (e.payload.role === 'assistant' && text) items.push({ kind: 'assistant', key, text, streaming: false, messageId: e.payload.message_id })
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
        const item = approvalItem(key, e.payload)
        approvals.set(e.payload.approval_id, item)
        items.push(item)
        break
      }
      case 'approval.resolved': {
        const a = approvals.get(e.payload.approval_id)
        if (a) {
          a.decision = e.payload.decision
          if (e.payload.comment) a.comment = e.payload.comment
          a.resolution = {
            decision: e.payload.decision,
            via: e.payload.via,
            actor: e.payload.actor,
            ...(e.payload.comment ? { comment: e.payload.comment } : {}),
            ...(e.payload.answer ? { answer: e.payload.answer } : {}),
            ...(e.payload.remember ? { remember: e.payload.remember } : {}),
            ...(e.payload.on_timeout_applied ? { onTimeoutApplied: e.payload.on_timeout_applied } : {}),
          }
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
      // Vom Server abgelehntes Runner-Event (PROTO-015): sichtbar an seiner Stelle, ohne Inhalt.
      case 'event.rejected':
        items.push({
          kind: 'error',
          key,
          title: 'Ein Ereignis wurde nicht gespeichert',
          detail: `${e.payload.original_type} · ${rejectReason(e.payload.reason)}. Der Server hat es abgelehnt; der Inhalt fehlt im Verlauf.`,
        })
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
      // Compaction im Verlauf (USE-009): solange sie läuft ein Hinweis, danach das Ergebnis.
      case 'compaction.started':
        compacting = compactingNote(key, e.payload, harness)
        items.push(compacting)
        break
      case 'compaction.completed': {
        if (compacting) items.splice(items.indexOf(compacting), 1)
        compacting = undefined
        items.push(compactedNote(key, e.payload, harness))
        break
      }
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
    items.push({ kind: 'assistant', key: `s-${id}`, text, streaming: true, messageId: id })
  }
  return items
}

/**
 * Herkunft der Compaction im Hinweis (USE-009, Entwurf `usage-context`): von dir, automatisch
 * ab der Schwelle (`compaction.auto: beton`) oder vom Harness selbst. Ältere Logs ohne
 * `trigger` bleiben ohne Zusatz.
 */
function triggerText(c: Compaction, harness: string): string | undefined {
  switch (c.trigger) {
    case 'user':
      return t('compaction.byUser')
    case 'beton':
      return c.threshold ? t('compaction.byBeton', { percent: Math.round(c.threshold * 100) }) : t('compaction.byBetonPlain')
    case 'harness':
      return t('compaction.byHarness', { harness: harness ? harnessName(harness) : 'Harness' })
    default:
      return undefined
  }
}

/** „Kontext wird kompaktiert … (von dir ausgelöst)“ bzw. „… · automatisch ab 80 % …“. */
function compactingNote(key: string, c: Compaction, harness: string): Extract<Item, { kind: 'note' }> {
  const by = triggerText(c, harness)
  const suffix = !by ? '' : c.trigger === 'user' ? ` (${by})` : ` · ${by}`
  return { kind: 'note', key, text: `${t('compaction.running')}${suffix}`, tone: 'neutral' }
}

const tokens = (n: number) => n.toLocaleString('de-DE')

/** „Kontext kompaktiert · 186.400 → 38.200 Tokens · von dir ausgelöst“ (Entwurf `usage-context`). */
function compactedNote(key: string, c: Compaction, harness: string): Extract<Item, { kind: 'note' }> {
  const parts = [t('compaction.done')]
  if (c.after_tokens !== undefined) parts.push(t('compaction.tokens', { before: tokens(c.before_tokens), after: tokens(c.after_tokens) }))
  const by = triggerText(c, harness)
  if (by) parts.push(by)
  return {
    kind: 'note',
    key,
    text: parts.join(' · '),
    tone: 'neutral',
    detail: t('compaction.explain', { harness: harness ? harnessName(harness) : t('compaction.theHarness') }),
  }
}

/** Offene Freigaben (für Status und Tastenkürzel), älteste zuerst. */
export function openApprovals(items: Item[]): ApprovalItem[] {
  return items.filter((i): i is ApprovalItem => i.kind === 'approval' && !i.decision)
}

const str = (v: unknown): string | undefined => (typeof v === 'string' && v !== '' ? v : undefined)

/**
 * Approval-Card aus `approval.requested` (POL-010). Seit WP-37 ist `subject.tool` ein Objekt
 * `{name, kind, args, args_modified}`; ältere Logs nennen nur den Namen und `subject.args`.
 */
export function approvalItem(
  key: string,
  p: { approval_id: string; kind: string; subject: unknown; options: string[]; expires_at?: string; on_timeout?: string },
): ApprovalItem {
  const subject = (p.subject ?? {}) as Record<string, unknown>
  const toolObj = subject.tool && typeof subject.tool === 'object' ? (subject.tool as Record<string, unknown>) : undefined
  const tool = str(subject.tool) ?? str(toolObj?.name) ?? str(subject.title) ?? p.kind
  const args = toolObj ? toolObj.args : (subject.args ?? subject)
  const reasons: ApprovalReason[] = Array.isArray(subject.reasons)
    ? (subject.reasons as unknown[]).map((r) => {
        if (typeof r === 'string') return { ruleId: '', scope: '', set: '', reason: r }
        const o = (r ?? {}) as Record<string, unknown>
        return {
          ruleId: str(o.rule_id) ?? '',
          scope: str(o.scope) ?? '',
          set: str(o.set) ?? '',
          reason: str(o.reason) ?? '',
          ...(str(o.severity) ? { severity: str(o.severity) } : {}),
        }
      })
    : []
  const requested = (subject.requested_by ?? {}) as Record<string, unknown>
  const by = [str(requested.harness) ? harnessName(str(requested.harness)!) : undefined, str(requested.agent) ? `Agent ${str(requested.agent)}` : undefined]
    .filter(Boolean)
    .join(' · ')
  const question = p.kind === 'question' ? str(subject.question) : undefined
  return {
    kind: 'approval',
    key,
    approvalId: p.approval_id,
    approvalKind: p.kind,
    title: question ? 'Frage des Agents' : (str(subject.title) ?? 'Freigabe nötig'),
    summary: question ?? str(subject.summary) ?? (argSummary(args) || tool),
    tool,
    ...(str(toolObj?.kind) ? { toolKind: str(toolObj?.kind) } : {}),
    args,
    ...(toolObj?.args_modified ? { argsModified: toolObj.args_modified } : {}),
    reasons,
    options: p.options ?? [],
    ...(p.expires_at ? { expiresAt: p.expires_at } : {}),
    ...(p.on_timeout ? { onTimeout: p.on_timeout } : {}),
    ...(question ? { question } : {}),
    choices: Array.isArray(subject.choices) ? (subject.choices as unknown[]).filter((c): c is string => typeof c === 'string') : [],
    ...(by ? { requestedBy: by } : {}),
  }
}
