import type { Capabilities, ChangeScope, Event, FileDiff } from '@beton/sdk'
import { payloadOf, textOf } from './events'

// --- Tabs der Workspace-Rail (WEB-008) ---------------------------------------------------

export type RailTab = 'files' | 'changes' | 'terminal' | 'browser' | 'agents' | 'pr' | 'side' | 'comments'

/** Was die Rail über Session, Rolle und Build weiß. */
export interface RailContext {
  capabilities?: Capabilities | undefined
  /** Rolle des Users in der Session; lokal (M1) immer `owner`. */
  role?: 'owner' | 'editor' | 'viewer' | undefined
}

interface TabDef {
  id: RailTab
  label: string
  /** In diesem Build umgesetzt (Meilenstein erreicht). */
  ready: boolean
  /** Weitere Voraussetzung (Rolle, Harness-Capability). */
  needs?: (ctx: RailContext) => boolean
}

/** Reihenfolge und Beschriftung wie im Prototyp (`session-chrome.tsx`). */
const TABS: TabDef[] = [
  { id: 'files', label: 'Dateien', ready: true },
  { id: 'changes', label: 'Änderungen', ready: true },
  // Terminals und Browser folgen mit M3 (WEB-010, BRW-009).
  { id: 'terminal', label: 'Terminal', ready: false, needs: (c) => c.role !== 'viewer' },
  { id: 'browser', label: 'Browser', ready: false },
  // Sub-Agent-Graph (WEB-012) folgt mit WP-27; nur bei Harnesses mit Sub-Agents.
  { id: 'agents', label: 'Agents', ready: false, needs: (c) => (c.capabilities?.subagents ?? 'none') !== 'none' },
  // PR/MR, Side-Chats und Kommentare folgen mit M4.
  { id: 'pr', label: 'PR', ready: false },
  { id: 'side', label: 'Side-Chats', ready: false },
  { id: 'comments', label: 'Kommentare', ready: false },
]

/**
 * Sichtbare Tabs (WEB-008 AC2): Was der Build noch nicht kann, wofür die Rolle fehlt oder
 * was der Harness nicht unterstützt, wird ausgeblendet statt kaputt angezeigt.
 */
export function visibleTabs(ctx: RailContext): { id: RailTab; label: string }[] {
  return TABS.filter((t) => t.ready && (t.needs?.(ctx) ?? true)).map(({ id, label }) => ({ id, label }))
}

/** Standardbreite je Tab laut Prototyp (Dateien 780 px, Änderungen 640 px). */
export const TAB_WIDTH: Partial<Record<RailTab, number>> = { files: 780, changes: 640 }
export const RAIL_MIN = 320

// --- fs.changed (WEB-008 AC1) --------------------------------------------------------------

/** Pfade aus `fs.changed`-Events nach `afterSeq` (ohne Doppelte, in Reihenfolge). */
export function changedPaths(events: readonly Event[] | undefined, afterSeq: number): string[] {
  const list = events ?? []
  let start = list.length
  while (start > 0 && list[start - 1]!.seq > afterSeq) start--
  const out = new Set<string>()
  for (let i = start; i < list.length; i++) {
    const p = payloadOf(list[i]!, 'fs.changed')
    if (!p) continue
    for (const c of p.changes) {
      if (c.from) out.add(c.from)
      out.add(c.path)
    }
  }
  return [...out]
}

/** `seq` des letzten `fs.changed` (0 ohne). Dient als Version für Abfragen. */
export function lastFsSeq(events: readonly Event[] | undefined): number {
  for (let i = (events?.length ?? 0) - 1; i >= 0; i--) {
    const e = events![i]!
    if (e.type === 'fs.changed') return e.seq
  }
  return 0
}

const WS_VERSION_EVENTS = new Set(['fs.changed', 'turn.completed', 'turn.failed', 'turn.interrupted', 'git.commit_created', 'git.worktree_created'])

/**
 * Version des Workspace für Abfragen (Baum, Änderungen, Diffs): `seq` des letzten Events,
 * nach dem sich Dateien oder Turn-Snapshots geändert haben können.
 */
export function workspaceVersion(events: readonly Event[] | undefined): number {
  for (let i = (events?.length ?? 0) - 1; i >= 0; i--) {
    const e = events![i]!
    if (WS_VERSION_EVENTS.has(e.type)) return e.seq
  }
  return 0
}

/** Anzahl geänderter Dateien seit `seenSeq` für das Change-Badge eines Tabs. */
export function badgeCount(events: readonly Event[] | undefined, seenSeq: number): number {
  return changedPaths(events, seenSeq).length
}

// --- Turns (Sicht „Pro Turn“) ----------------------------------------------------------------

export interface TurnInfo {
  id: string
  index: number
  ts: string
  prompt: string
}

/** Turns der Session in Reihenfolge mit dem Text der auslösenden Nachricht. */
export function turnsOf(events: readonly Event[] | undefined): TurnInfo[] {
  const turns: TurnInfo[] = []
  let lastPrompt = ''
  for (const e of events ?? []) {
    const m = payloadOf(e, 'message.completed')
    if (m && m.role === 'user') {
      lastPrompt = textOf(m.content)
    }
    const t = payloadOf(e, 'turn.started')
    if (t) turns.push({ id: t.turn_id, index: turns.length + 1, ts: e.ts, prompt: lastPrompt })
  }
  return turns
}

/** „Turn 3 · 14:06 · „Bitte mit Tests““ wie im Prototyp. */
export function turnLabel(t: TurnInfo): string {
  const time = new Date(t.ts).toLocaleTimeString('de-DE', { hour: '2-digit', minute: '2-digit' })
  const prompt = t.prompt.split('\n')[0]?.trim() ?? ''
  const short = prompt.length > 32 ? `${prompt.slice(0, 31)}…` : prompt
  return short ? `Turn ${t.index} · ${time} · „${short}“` : `Turn ${t.index} · ${time}`
}

// --- Diffs (WEB-011) ---------------------------------------------------------------------------

export type DiffSide = 'new' | 'old'

/** Zeilenanker im Link: `N16` (neue Seite) bzw. `A16` (alte Seite). */
export function lineAnchor(side: DiffSide, line: number): string {
  return `${side === 'new' ? 'N' : 'A'}${line}`
}

export function parseAnchor(anchor: string | undefined): { side: DiffSide; line: number } | undefined {
  const m = /^([NA])(\d+)$/.exec(anchor ?? '')
  if (!m) return undefined
  return { side: m[1] === 'N' ? 'new' : 'old', line: Number(m[2]) }
}

/** Parameter eines teilbaren Links auf eine Diff-Zeile (Route `/s/$sessionId`). */
export interface WorkspaceSearch {
  tab?: RailTab | undefined
  scope?: ChangeScope | undefined
  turn?: string | undefined
  file?: string | undefined
  line?: string | undefined
}

export function validateWorkspaceSearch(raw: Record<string, unknown>): WorkspaceSearch {
  const str = (v: unknown) => (typeof v === 'string' && v ? v : undefined)
  const tab = str(raw.tab)
  const scope = str(raw.scope)
  return {
    tab: tab === 'files' || tab === 'changes' ? tab : undefined,
    scope: scope === 'uncommitted' || scope === 'branch' || scope === 'turn' ? scope : undefined,
    turn: str(raw.turn),
    file: str(raw.file),
    line: parseAnchor(str(raw.line)) ? str(raw.line) : undefined,
  }
}

/** Absoluter, teilbarer Link auf eine Diff-Zeile (WEB-011 AC2). */
export function lineLink(origin: string, sessionId: string, p: { scope: ChangeScope; turn?: string | undefined; file: string; anchor: string }): string {
  const q = new URLSearchParams({ tab: 'changes', scope: p.scope })
  if (p.turn) q.set('turn', p.turn)
  q.set('file', p.file)
  q.set('line', p.anchor)
  return `${origin}/s/${encodeURIComponent(sessionId)}?${q.toString()}`
}

/** Eine Zeile der virtualisierten Diff-Liste. */
export type DiffRow =
  | { kind: 'hunk'; text: string }
  | { kind: 'line'; type: 'context' | 'add' | 'delete'; old?: number | undefined; new?: number | undefined; text: string; anchor: string }

/** Hunks als flache Zeilenliste; Anker zeigt auf die neue Seite, bei Löschungen auf die alte. */
export function diffRows(diff: Pick<FileDiff, 'hunks'>): DiffRow[] {
  const rows: DiffRow[] = []
  for (const h of diff.hunks) {
    rows.push({ kind: 'hunk', text: h.header || `@@ -${h.old_start},${h.old_lines} +${h.new_start},${h.new_lines} @@` })
    for (const l of h.lines) {
      const anchor = l.kind === 'delete' ? lineAnchor('old', l.old_line ?? 0) : lineAnchor('new', l.new_line ?? 0)
      rows.push({ kind: 'line', type: l.kind, old: l.old_line, new: l.new_line, text: l.text, anchor })
    }
  }
  return rows
}

// --- Editor ------------------------------------------------------------------------------------

/** Erster abweichender Zeilenbereich zweier Texte (für die Gegenüberstellung im Konfliktdialog). */
export function changedRegion(base: string, other: string, context = 0): { start: number; lines: string[] } {
  const a = base.split('\n')
  const b = other.split('\n')
  let start = 0
  while (start < a.length && start < b.length && a[start] === b[start]) start++
  let endA = a.length - 1
  let endB = b.length - 1
  while (endA >= start && endB >= start && a[endA] === b[endB]) {
    endA--
    endB--
  }
  const from = Math.max(0, start - context)
  const lines = b.slice(from, Math.max(endB + 1 + context, from))
  return { start: from + 1, lines: lines.slice(0, 12) }
}

/** Sprache für Shiki aus der Dateiendung (`undefined`: ohne Hervorhebung). */
export function shikiLang(path: string): string | undefined {
  const name = path.split('/').pop()?.toLowerCase() ?? ''
  if (name === 'dockerfile') return 'dockerfile'
  if (name === 'makefile') return 'make'
  const ext = name.includes('.') ? name.split('.').pop()! : ''
  const map: Record<string, string> = {
    ts: 'typescript', mts: 'typescript', cts: 'typescript', tsx: 'tsx', js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'jsx',
    rs: 'rust', py: 'python', go: 'go', json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'toml', md: 'markdown', css: 'css', scss: 'scss',
    html: 'html', sh: 'bash', bash: 'bash', zsh: 'bash', sql: 'sql', java: 'java', kt: 'kotlin', c: 'c', h: 'c', cpp: 'cpp', hpp: 'cpp',
    rb: 'ruby', php: 'php', swift: 'swift', cs: 'csharp', xml: 'xml', svg: 'xml', vue: 'vue', svelte: 'svelte',
  }
  return map[ext]
}

/** Lesbarer Sprachname für die Statuszeile des Editors. */
export function languageName(path: string): string {
  const lang = shikiLang(path)
  const names: Record<string, string> = {
    typescript: 'TypeScript', tsx: 'TypeScript (TSX)', javascript: 'JavaScript', jsx: 'JavaScript (JSX)', rust: 'Rust', python: 'Python',
    go: 'Go', json: 'JSON', yaml: 'YAML', toml: 'TOML', markdown: 'Markdown', css: 'CSS', html: 'HTML', bash: 'Shell', sql: 'SQL',
  }
  return lang ? (names[lang] ?? lang) : 'Text'
}

export function isMarkdown(path: string): boolean {
  return /\.(md|markdown|mdx)$/i.test(path)
}

// --- „An Agent anhängen“ (WEB-009) ------------------------------------------------------------

export interface Attachment {
  id: string
  path: string
  /** Erste und letzte Zeile (1-basiert, inklusive). */
  from: number
  to: number
  /** Zusatz zur Herkunft, z. B. „neu, Änderungen pro Turn“. */
  note?: string | undefined
  snippet: string
}

const SNIPPET_MAX = 20_000

/** Zeilenbereich als Anzeige, z. B. „Z. 13–20“ oder „Z. 16“. */
export function rangeLabel(from: number, to: number): string {
  return from === to ? `Z. ${from}` : `Z. ${from}–${to}`
}

/** Nachricht mit angehängten Referenzen und Ausschnitten. */
export function withAttachments(text: string, attachments: readonly Attachment[]): string {
  if (attachments.length === 0) return text
  const parts = attachments.map((a) => {
    const lang = shikiLang(a.path) ?? ''
    const snippet = a.snippet.length > SNIPPET_MAX ? `${a.snippet.slice(0, SNIPPET_MAX)}\n…` : a.snippet
    const fence = snippet.includes('```') ? '````' : '```'
    return `\`${a.path}\` ${rangeLabel(a.from, a.to)}${a.note ? ` (${a.note})` : ''}:\n${fence}${lang}\n${snippet}\n${fence}`
  })
  return `${text}\n\n${parts.join('\n\n')}`
}
