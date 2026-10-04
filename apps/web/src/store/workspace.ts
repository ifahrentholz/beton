import { create } from 'zustand'
import { BetonError, type WorkspaceFile, type WrittenFile } from '@beton/sdk'
import { client } from '@/lib/client'
import type { Attachment, RailTab } from '@/lib/workspace'

/** Zugriff auf die Workspace-API einer Session (in Tests ersetzbar). */
export interface WorkspaceApi {
  readFile: (path: string) => Promise<WorkspaceFile>
  writeFile: (path: string, content: string, ifMatch?: string) => Promise<WrittenFile>
}

let apiFor: (sessionId: string) => WorkspaceApi = (id) => client.session(id).workspace

/** Für Tests: eigene API einsetzen. */
export function setWorkspaceApi(f: (sessionId: string) => WorkspaceApi): void {
  apiFor = f
}

/** Stand des Agents, der mit ungespeicherten Änderungen kollidiert (WEB-009 AC1). */
export interface Conflict {
  theirs: string
  theirsSha: string
  /** Der Agent hat die Datei gelöscht. */
  deleted: boolean
  at: number
}

/** Eine geöffnete Datei im Editor. */
export interface OpenFile {
  path: string
  status: 'loading' | 'ready' | 'error'
  error?: { kind: 'outside' | 'missing' | 'other'; message: string } | undefined
  /** SHA-256 des Stands, auf dem `base` beruht (für `If-Match`). */
  sha: string
  /** Zuletzt gelesener bzw. gespeicherter Inhalt. */
  base: string
  /** Inhalt im Editor. */
  draft: string
  binary: boolean
  tooLarge: boolean
  size: number
  saving: boolean
  saveError?: string | undefined
  conflict?: Conflict | undefined
  /** Zusammenführen im Diff-Editor läuft. */
  merging: boolean
  /** Steigt, wenn der Inhalt von außen ersetzt wird (Editor übernimmt dann `draft`). */
  rev: number
  /** Zeile, zu der der Editor springen soll. */
  reveal?: number | undefined
  preview: boolean
}

export interface SessionWorkspace {
  tab: RailTab
  open: string[]
  active?: string | undefined
  files: Record<string, OpenFile>
  /** Letzte gesehene `seq` je Tab (Change-Badge, WEB-008 AC1). */
  seen: Partial<Record<RailTab, number>>
  attachments: Attachment[]
  /** Pfade, die aus dem Workspace hinauszeigen (403). */
  outside: string[]
}

const emptyWs = (): SessionWorkspace => ({ tab: 'changes', open: [], files: {}, seen: {}, attachments: [], outside: [] })

const RAIL_KEY = 'beton.rail'

function loadRail(): { open: boolean; width: number | null } {
  try {
    const raw = JSON.parse(localStorage.getItem(RAIL_KEY) ?? '{}') as { open?: unknown; width?: unknown }
    return { open: raw.open !== false, width: typeof raw.width === 'number' ? raw.width : null }
  } catch {
    return { open: true, width: null }
  }
}

function saveRail(open: boolean, width: number | null): void {
  try {
    localStorage.setItem(RAIL_KEY, JSON.stringify({ open, width }))
  } catch {
    // Speicher nicht verfügbar: Einstellung gilt nur in diesem Fenster.
  }
}

interface WorkspaceState {
  sessions: Record<string, SessionWorkspace>
  /** Rail ab 1024 px eingeblendet. */
  railOpen: boolean
  /** Vom User gezogene Breite; `null` = Standard des Tabs. */
  railWidth: number | null
  /** Rail unter 1024 px als Vollbild offen. */
  railSheet: boolean
  setRailOpen: (open: boolean) => void
  setRailWidth: (width: number | null) => void
  setRailSheet: (open: boolean) => void
  ws: (sessionId: string) => SessionWorkspace
  update: (sessionId: string, f: (ws: SessionWorkspace) => Partial<SessionWorkspace>) => void
  file: (sessionId: string, path: string, f: (file: OpenFile) => Partial<OpenFile>) => void
}

export const useWorkspace = create<WorkspaceState>()((set, get) => {
  const rail = loadRail()
  return {
    sessions: {},
    railOpen: rail.open,
    railWidth: rail.width,
    railSheet: false,
    setRailOpen: (open) => {
      set({ railOpen: open })
      saveRail(open, get().railWidth)
    },
    setRailWidth: (width) => {
      set({ railWidth: width })
      saveRail(get().railOpen, width)
    },
    setRailSheet: (open) => set({ railSheet: open }),
    ws: (sessionId) => get().sessions[sessionId] ?? emptyWs(),
    update: (sessionId, f) =>
      set((s) => {
        const cur = s.sessions[sessionId] ?? emptyWs()
        return { sessions: { ...s.sessions, [sessionId]: { ...cur, ...f(cur) } } }
      }),
    file: (sessionId, path, f) =>
      set((s) => {
        const cur = s.sessions[sessionId] ?? emptyWs()
        const file = cur.files[path]
        if (!file) return s
        return { sessions: { ...s.sessions, [sessionId]: { ...cur, files: { ...cur.files, [path]: { ...file, ...f(file) } } } } }
      }),
  }
})

const EMPTY = emptyWs()

/** Workspace-Zustand einer Session (für Komponenten). */
export function useSessionWorkspace(sessionId: string): SessionWorkspace {
  return useWorkspace((s) => s.sessions[sessionId] ?? EMPTY)
}

function problemKind(e: unknown): { kind: 'outside' | 'missing' | 'other'; message: string } {
  if (e instanceof BetonError) {
    if (e.status === 403) return { kind: 'outside', message: e.message }
    if (e.status === 404) return { kind: 'missing', message: e.message }
    return { kind: 'other', message: e.message }
  }
  return { kind: 'other', message: e instanceof Error ? e.message : 'Datei nicht lesbar' }
}

const dirty = (f: OpenFile) => f.draft !== f.base

// --- Aktionen ------------------------------------------------------------------------------

export function setTab(sessionId: string, tab: RailTab): void {
  useWorkspace.getState().update(sessionId, () => ({ tab }))
}

/** Tab als gesehen markieren (bis `seq`): das Change-Badge verschwindet. */
export function markSeen(sessionId: string, tab: RailTab, seq: number): void {
  const ws = useWorkspace.getState().ws(sessionId)
  if ((ws.seen[tab] ?? -1) >= seq) return
  useWorkspace.getState().update(sessionId, (w) => ({ seen: { ...w.seen, [tab]: seq } }))
}

/** Startwert der Badges: alles bis `seq` gilt als gesehen (nur beim ersten Mal). */
export function initSeen(sessionId: string, seq: number): void {
  const ws = useWorkspace.getState().ws(sessionId)
  if (ws.seen.files !== undefined) return
  useWorkspace.getState().update(sessionId, (w) => ({ seen: { files: seq, changes: seq, ...w.seen } }))
}

/** Datei öffnen (oder zum Tab wechseln); optional an eine Zeile springen. */
export async function openFile(sessionId: string, path: string, line?: number): Promise<void> {
  const st = useWorkspace.getState()
  const existing = st.ws(sessionId).files[path]
  st.update(sessionId, (w) => ({
    tab: 'files',
    active: path,
    open: w.open.includes(path) ? w.open : [...w.open, path],
    files: existing
      ? { ...w.files, [path]: { ...existing, reveal: line ?? existing.reveal } }
      : {
          ...w.files,
          [path]: { path, status: 'loading', sha: '', base: '', draft: '', binary: false, tooLarge: false, size: 0, saving: false, merging: false, rev: 0, reveal: line, preview: false },
        },
  }))
  if (existing && existing.status !== 'error') return
  try {
    const f = await apiFor(sessionId).readFile(path)
    const content = f.content ?? ''
    useWorkspace.getState().file(sessionId, path, (cur) => ({
      status: 'ready',
      error: undefined,
      sha: f.sha256,
      base: content,
      draft: content,
      binary: f.binary,
      tooLarge: f.too_large,
      size: f.size,
      rev: cur.rev + 1,
    }))
  } catch (e) {
    const error = problemKind(e)
    useWorkspace.getState().file(sessionId, path, () => ({ status: 'error', error }))
    if (error.kind === 'outside') {
      useWorkspace.getState().update(sessionId, (w) => ({ outside: w.outside.includes(path) ? w.outside : [...w.outside, path] }))
    }
  }
}

export function closeFile(sessionId: string, path: string): void {
  useWorkspace.getState().update(sessionId, (w) => {
    const open = w.open.filter((p) => p !== path)
    const files = { ...w.files }
    delete files[path]
    const idx = w.open.indexOf(path)
    const active = w.active === path ? (open[Math.min(idx, open.length - 1)] ?? undefined) : w.active
    return { open, files, active }
  })
}

export function setActive(sessionId: string, path: string): void {
  useWorkspace.getState().update(sessionId, () => ({ active: path }))
}

export function setDraft(sessionId: string, path: string, draft: string): void {
  useWorkspace.getState().file(sessionId, path, () => ({ draft, saveError: undefined }))
}

export function togglePreview(sessionId: string, path: string): void {
  useWorkspace.getState().file(sessionId, path, (f) => ({ preview: !f.preview }))
}

export function clearReveal(sessionId: string, path: string): void {
  useWorkspace.getState().file(sessionId, path, () => ({ reveal: undefined }))
}

function getFile(sessionId: string, path: string): OpenFile | undefined {
  return useWorkspace.getState().ws(sessionId).files[path]
}

/** Aktuellen Stand vom Server als Konflikt festhalten. */
async function captureConflict(sessionId: string, path: string): Promise<void> {
  try {
    const f = await apiFor(sessionId).readFile(path)
    useWorkspace.getState().file(sessionId, path, () => ({
      conflict: { theirs: f.content ?? '', theirsSha: f.sha256, deleted: false, at: Date.now() },
    }))
  } catch (e) {
    if (e instanceof BetonError && e.status === 404) {
      useWorkspace.getState().file(sessionId, path, () => ({ conflict: { theirs: '', theirsSha: '', deleted: true, at: Date.now() } }))
    }
  }
}

/**
 * Speichern (`⌘S`) mit `If-Match` auf den zuletzt gelesenen Stand. Hat sich die Datei
 * inzwischen geändert (`412`), entsteht ein Konflikt statt stillem Überschreiben.
 */
export async function save(sessionId: string, path: string, content?: string, ifMatch?: string): Promise<boolean> {
  const f = getFile(sessionId, path)
  if (!f || f.status !== 'ready' || f.saving) return false
  const text = content ?? f.draft
  useWorkspace.getState().file(sessionId, path, () => ({ saving: true, saveError: undefined }))
  try {
    const written = await apiFor(sessionId).writeFile(path, text, ifMatch ?? (f.sha || undefined))
    useWorkspace.getState().file(sessionId, path, (cur) => ({
      saving: false,
      sha: written.sha256,
      base: text,
      // Während des Speicherns Getipptes bleibt erhalten.
      draft: cur.draft === f.draft ? text : cur.draft,
      rev: content !== undefined ? cur.rev + 1 : cur.rev,
      conflict: undefined,
      merging: false,
    }))
    return true
  } catch (e) {
    useWorkspace.getState().file(sessionId, path, () => ({ saving: false }))
    if (e instanceof BetonError && e.status === 412) {
      await captureConflict(sessionId, path)
    } else {
      useWorkspace.getState().file(sessionId, path, () => ({ saveError: e instanceof Error ? e.message : 'Speichern fehlgeschlagen' }))
    }
    return false
  }
}

/**
 * Reaktion auf `fs.changed` für eine geöffnete Datei: unverändert lassen, still neu laden
 * (keine eigenen Änderungen) oder – bei ungespeicherten Änderungen – einen Konflikt melden
 * (WEB-009 AC1).
 */
export async function refresh(sessionId: string, path: string): Promise<void> {
  const f = getFile(sessionId, path)
  if (!f || f.status !== 'ready' || f.saving) return
  let next: WorkspaceFile | undefined
  try {
    next = await apiFor(sessionId).readFile(path)
  } catch (e) {
    if (e instanceof BetonError && e.status === 404) {
      const cur = getFile(sessionId, path)
      if (cur && dirty(cur)) {
        useWorkspace.getState().file(sessionId, path, () => ({ conflict: { theirs: '', theirsSha: '', deleted: true, at: Date.now() } }))
      } else {
        useWorkspace.getState().file(sessionId, path, () => ({ status: 'error', error: { kind: 'missing', message: 'Die Datei wurde gelöscht.' } }))
      }
    }
    return
  }
  const cur = getFile(sessionId, path)
  if (!cur || cur.saving || next.sha256 === cur.sha) return
  if (cur.conflict && cur.conflict.theirsSha === next.sha256) return
  const content = next.content ?? ''
  if (!dirty(cur)) {
    useWorkspace.getState().file(sessionId, path, (c) => ({ sha: next.sha256, base: content, draft: content, binary: next.binary, tooLarge: next.too_large, size: next.size, rev: c.rev + 1 }))
    return
  }
  useWorkspace.getState().file(sessionId, path, () => ({ conflict: { theirs: content, theirsSha: next.sha256, deleted: false, at: Date.now() } }))
}

export type Resolution = 'theirs' | 'mine' | 'merge' | 'cancel-merge'

/** Entscheidung im Konfliktdialog. */
export async function resolveConflict(sessionId: string, path: string, how: Resolution): Promise<void> {
  const f = getFile(sessionId, path)
  if (!f) return
  switch (how) {
    case 'theirs': {
      const c = f.conflict
      if (!c) return
      if (c.deleted) {
        closeFile(sessionId, path)
        return
      }
      useWorkspace.getState().file(sessionId, path, (cur) => ({ sha: c.theirsSha, base: c.theirs, draft: c.theirs, conflict: undefined, merging: false, rev: cur.rev + 1 }))
      return
    }
    case 'mine': {
      const c = f.conflict
      // Überschreibt den Stand des Agents, aber nur genau diesen (If-Match).
      await save(sessionId, path, f.draft, c && !c.deleted ? c.theirsSha : undefined)
      return
    }
    case 'merge':
      useWorkspace.getState().file(sessionId, path, () => ({ merging: true }))
      useWorkspace.getState().update(sessionId, () => ({ tab: 'files', active: path }))
      useWorkspace.getState().setRailSheet(true)
      if (!useWorkspace.getState().railOpen) useWorkspace.getState().setRailOpen(true)
      return
    case 'cancel-merge':
      useWorkspace.getState().file(sessionId, path, () => ({ merging: false }))
      return
  }
}

/** Ergebnis des Zusammenführens speichern (gegen den Stand des Agents). */
export async function saveMerged(sessionId: string, path: string, content: string): Promise<boolean> {
  const f = getFile(sessionId, path)
  if (!f?.conflict) return save(sessionId, path, content)
  useWorkspace.getState().file(sessionId, path, () => ({ draft: content }))
  return save(sessionId, path, content, f.conflict.deleted ? undefined : f.conflict.theirsSha)
}

// --- Anhänge für den Composer ----------------------------------------------------------------

let attachSeq = 0

export function attach(sessionId: string, a: Omit<Attachment, 'id'>): void {
  attachSeq += 1
  const id = `att${attachSeq}`
  useWorkspace.getState().update(sessionId, (w) => ({ attachments: [...w.attachments, { ...a, id }] }))
}

export function detach(sessionId: string, id: string): void {
  useWorkspace.getState().update(sessionId, (w) => ({ attachments: w.attachments.filter((a) => a.id !== id) }))
}

export function clearAttachments(sessionId: string): void {
  useWorkspace.getState().update(sessionId, () => ({ attachments: [] }))
}
