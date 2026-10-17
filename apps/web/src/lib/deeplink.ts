import { validateWorkspaceSearch, type WorkspaceSearch } from './workspace'

/**
 * Parameter der Session-Route (`/s/$sessionId`): Diff-Links der Workspace-Rail (WEB-011) und
 * Sprungziele aus Deep-Links bzw. Benachrichtigungen der Desktop-App (DESK-006, DESK-005):
 * `seq` (Event), `comment` (Kommentar-Anker, ab M4) und `opened=link` für den Hinweis
 * „Geöffnet über beton://…“. Ein Link öffnet nur die Ansicht; er löst nie eine Aktion aus.
 */
export interface SessionSearch extends WorkspaceSearch {
  seq?: number | undefined
  comment?: string | undefined
  opened?: 'link' | undefined
}

export function validateSessionSearch(raw: Record<string, unknown>): SessionSearch {
  const seq = typeof raw.seq === 'number' ? raw.seq : typeof raw.seq === 'string' ? Number(raw.seq) : NaN
  const comment = typeof raw.comment === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(raw.comment) ? raw.comment : undefined
  return {
    ...validateWorkspaceSearch(raw),
    seq: Number.isSafeInteger(seq) && seq > 0 ? seq : undefined,
    comment,
    opened: raw.opened === 'link' ? 'link' : undefined,
  }
}

/** Der Link, über den die Session geöffnet wurde (für den Hinweis im Kopf). */
export function linkOf(sessionId: string, search: Pick<SessionSearch, 'seq' | 'comment'>): string {
  const q = new URLSearchParams()
  if (search.seq !== undefined) q.set('seq', String(search.seq))
  if (search.comment) q.set('comment', search.comment)
  const qs = q.toString()
  return `beton://local/s/${sessionId}${qs ? `?${qs}` : ''}`
}

/** `seq` eines Stream-Elements aus seinem Schlüssel (`e<seq>` bzw. `e<seq>-<n>`). */
function seqOfKey(key: string): number | undefined {
  const m = /^e(\d+)(?:-\d+)?$/.exec(key)
  return m ? Number(m[1]) : undefined
}

/**
 * Index des Stream-Elements, das Event `seq` zeigt (DESK-006 AC3): das letzte Element, das bei
 * oder vor `seq` beginnt (ein Tool-Call fasst `requested` bis `completed` zusammen).
 */
export function indexForSeq(items: { key: string }[], seq: number): number | undefined {
  let found: number | undefined
  items.forEach((item, i) => {
    const s = seqOfKey(item.key)
    if (s !== undefined && s <= seq) found = i
  })
  return found
}
