import type { ApprovalItem, ApprovalResolution } from './timeline'

/*
 * Hilfen der Freigabe-Leiste (WEB-007) und der Inbox (UX-001): Ablauf, Herkunft der Gründe,
 * Vermerk „entschieden von …“ und die Vorschau einer Datei-Änderung.
 */

const SCOPE: Record<string, string> = {
  org: 'Org',
  team: 'Team',
  user: 'User',
  project: 'Projekt',
  agent: 'Agent',
}

/** Herkunft eines Grundes, z. B. „Projekt · git-and-budget“. */
export function scopeLabel(scope: string, set: string): string {
  return [SCOPE[scope] ?? scope, set].filter(Boolean).join(' · ')
}

const AFTER: Record<string, string> = { deny: 'dann ablehnen', abort: 'dann abbrechen', allow: 'dann erlauben' }

/** Restzeit einer Card, z. B. „läuft in 28 Min. ab · dann ablehnen“; `urgent` unter 5 Min. */
export function timeLeft(expiresAt: string | undefined, onTimeout: string | undefined, now: number = Date.now()): { text: string; urgent: boolean } | undefined {
  if (!expiresAt) return undefined
  const ms = Date.parse(expiresAt) - now
  if (Number.isNaN(ms)) return undefined
  const after = onTimeout ? AFTER[onTimeout] : undefined
  const min = Math.ceil(ms / 60_000)
  let span: string
  if (ms <= 0) span = 'läuft gleich ab'
  else if (min < 60) span = `läuft in ${min} Min. ab`
  else if (min < 48 * 60) {
    const h = Math.floor(min / 60)
    const m = min % 60
    span = `läuft in ${h} Std.${m ? ` ${m} Min.` : ''} ab`
  } else span = `läuft am ${new Date(expiresAt).toLocaleDateString('de-DE')} ab`
  return { text: after ? `${span} · ${after}` : span, urgent: ms < 5 * 60_000 }
}

/** Wer entschieden hat, aus Sicht dieses Clients (WEB-007 AC2). */
export function actorLabel(actor: unknown, own: boolean): string {
  const a = (actor ?? {}) as { kind?: string; id?: string; component?: string }
  if (a.kind === 'user') {
    const you = !a.id || a.id === 'usr_local'
    if (you) return own ? 'dir' : 'dir in einem anderen Fenster'
    return a.id ?? 'einem Menschen'
  }
  if (a.kind === 'system') return 'beton'
  return 'dem Agent'
}

const VERB: Record<string, string> = {
  allow: 'Erlaubt',
  deny: 'Abgelehnt',
  abort: 'Abgebrochen',
  answer: 'Beantwortet',
}

/** `resolution` aus der Inbox-API (snake_case) als {@link ApprovalResolution}. */
export function resolutionFrom(v: unknown): ApprovalResolution | undefined {
  if (!v || typeof v !== 'object') return undefined
  const o = v as Record<string, unknown>
  const s = (k: string) => (typeof o[k] === 'string' ? (o[k] as string) : undefined)
  return {
    decision: s('decision') ?? '',
    via: s('via') ?? '',
    actor: o.actor,
    ...(s('comment') ? { comment: s('comment') } : {}),
    ...(s('answer') ? { answer: s('answer') } : {}),
    ...(s('remember') ? { remember: s('remember') } : {}),
    ...(s('on_timeout_applied') ? { onTimeoutApplied: s('on_timeout_applied') } : {}),
  }
}

/** Nur der Teil „von …“ (Inline-Karte im Verlauf), bei Ablauf „abgelaufen“. */
export function decidedBy(r: ApprovalResolution): string {
  if (r.via === 'timeout') return 'abgelaufen, niemand hat entschieden'
  if (r.via === 'system') return 'von beton'
  return `von ${actorLabel(r.actor, true)}`
}

/** Vermerk über eine entschiedene Freigabe, z. B. „Erlaubt von Mara“ oder „Abgelaufen – angewendet: abbrechen“. */
export function resolutionText(r: ApprovalResolution, own: boolean): string {
  if (r.via === 'timeout') {
    const applied = r.onTimeoutApplied ? (VERB[r.onTimeoutApplied] ?? r.onTimeoutApplied).toLowerCase() : 'abgelehnt'
    return `Abgelaufen, niemand hat entschieden – angewendet: ${applied}`
  }
  const verb = r.remember === 'session' ? 'Für diese Session erlaubt' : (VERB[r.decision] ?? r.decision)
  if (r.via === 'system') return `${verb} von beton${r.comment ? `: „${r.comment}“` : ''}`
  const extra = r.answer ? `: „${r.answer}“` : r.comment ? `: „${r.comment}“` : ''
  return `${verb} von ${actorLabel(r.actor, own)}${extra}`
}

export type DiffLine = { kind: 'ctx' | 'add' | 'del'; text: string }

/** Vorschau einer Datei-Änderung aus den Tool-Argumenten (Edit, MultiEdit, Write, apply_patch). */
export function editPreview(item: Pick<ApprovalItem, 'args' | 'tool' | 'toolKind'>): { file: string; lines: DiffLine[]; stat: string } | undefined {
  const a = (item.args ?? {}) as Record<string, unknown>
  const str = (v: unknown) => (typeof v === 'string' ? v : undefined)
  const file = str(a.file_path) ?? str(a.path) ?? ''
  const lines: DiffLine[] = []
  const pair = (oldText: string, newText: string) => {
    for (const l of oldText.split('\n')) lines.push({ kind: 'del', text: l })
    for (const l of newText.split('\n')) lines.push({ kind: 'add', text: l })
  }
  if (str(a.old_string) !== undefined && str(a.new_string) !== undefined) {
    pair(str(a.old_string)!, str(a.new_string)!)
  } else if (Array.isArray(a.edits)) {
    for (const e of a.edits as Record<string, unknown>[]) {
      if (typeof e?.old_string === 'string' && typeof e?.new_string === 'string') pair(e.old_string, e.new_string)
    }
  } else if (str(a.content) !== undefined && (item.toolKind === 'file_write' || item.tool === 'Write')) {
    for (const l of str(a.content)!.split('\n')) lines.push({ kind: 'add', text: l })
  } else if (str(a.input)?.includes('*** Begin Patch') || str(a.patch)?.includes('*** Begin Patch')) {
    const patch = str(a.input) ?? str(a.patch)!
    let name = ''
    for (const l of patch.split('\n')) {
      const m = /^\*\*\* (?:Update|Add|Delete) File: (.+)$/.exec(l)
      if (m) {
        name ||= m[1]!
        continue
      }
      if (l.startsWith('***') || l.startsWith('@@')) continue
      if (l.startsWith('+')) lines.push({ kind: 'add', text: l.slice(1) })
      else if (l.startsWith('-')) lines.push({ kind: 'del', text: l.slice(1) })
      else lines.push({ kind: 'ctx', text: l.startsWith(' ') ? l.slice(1) : l })
    }
    if (!file && name) return finish(name, lines)
  } else return undefined
  return finish(file, lines)
}

function finish(file: string, lines: DiffLine[]): { file: string; lines: DiffLine[]; stat: string } | undefined {
  if (lines.length === 0) return undefined
  const add = lines.filter((l) => l.kind === 'add').length
  const del = lines.filter((l) => l.kind === 'del').length
  return { file, lines, stat: `+${add} −${del}` }
}
