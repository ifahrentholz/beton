import { lazy, Suspense, useCallback, useEffect, useState } from 'react'
import { Eye, Paperclip, X } from 'lucide-react'
import { client } from '@/lib/client'
import { changedRegion, isMarkdown, languageName, rangeLabel } from '@/lib/workspace'
import { cn } from '@/lib/utils'
import {
  attach,
  clearReveal,
  closeFile,
  resolveConflict,
  save,
  saveMerged,
  setActive,
  setDraft,
  togglePreview,
  useSessionWorkspace,
  type OpenFile,
} from '@/store/workspace'
import { harnessName } from '../harness'
import { Markdown } from '../markdown'
import type { Selected } from './editor-impl'
import { Btn, Callout, DialogPanel, Overlay } from './ui'

// Monaco nur bei Bedarf laden (WEB-009 AC2).
const CodeEditor = lazy(() => import('./editor-impl').then((m) => ({ default: m.CodeEditor })))
const MergeEditor = lazy(() => import('./editor-impl').then((m) => ({ default: m.MergeEditor })))

const base = (p: string) => p.split('/').pop() ?? p
const dirty = (f: OpenFile) => f.draft !== f.base

function Loading() {
  return <div className="p-4 text-[12px] text-muted-foreground">Editor lädt …</div>
}

function EditorTabs({ sessionId }: { sessionId: string }) {
  const ws = useSessionWorkspace(sessionId)
  return (
    <div role="tablist" aria-label="Geöffnete Dateien" className="flex h-9 shrink-0 items-end overflow-x-auto border-b border-border bg-sidebar">
      {ws.open.map((p) => {
        const f = ws.files[p]
        const active = p === ws.active
        return (
          <span
            key={p}
            role="tab"
            aria-selected={active}
            title={p}
            tabIndex={0}
            onClick={() => setActive(sessionId, p)}
            onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && setActive(sessionId, p)}
            className={cn(
              'group -mb-px flex h-8 shrink-0 cursor-default items-center gap-2 border-r border-border px-3 text-[12px]',
              active ? 'border-b border-b-card bg-card font-medium' : 'text-muted-foreground hover:text-foreground',
            )}
          >
            {base(p)}
            {f && dirty(f) ? (
              <span className="size-2 rounded-full bg-foreground" title="Ungespeicherte Änderungen" data-testid="dirty-mark" />
            ) : (
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  closeFile(sessionId, p)
                }}
                className="rounded-sm text-muted-foreground hover:text-foreground"
                aria-label={`${base(p)} schließen`}
              >
                <X className="size-3" />
              </button>
            )}
          </span>
        )
      })}
    </div>
  )
}

/** Gegenüberstellung im Konfliktdialog: der erste geänderte Bereich mit Zeilennummern. */
function Region({ base: original, text }: { base: string; text: string }) {
  const r = changedRegion(original, text)
  const width = String(r.start + r.lines.length).length
  const body = r.lines.length ? r.lines.map((l, i) => `${String(r.start + i).padStart(width)}  ${l}`).join('\n') : '(gelöscht)'
  return <pre className="max-h-48 overflow-auto rounded-md border border-border bg-card p-2 font-mono text-[11.5px]">{body}</pre>
}

/** Der Agent hat eine Datei mit ungespeicherten Änderungen geändert (WEB-009 AC1). */
export function ConflictDialog({ sessionId, harness, file }: { sessionId: string; harness: string; file: OpenFile }) {
  const c = file.conflict!
  const [, tick] = useState(0)
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 1000)
    return () => clearInterval(t)
  }, [])
  const ago = Math.max(1, Math.round((Date.now() - c.at) / 1000))
  return (
    <Overlay>
      <DialogPanel
        waiting
        role="alertdialog"
        title={`${harnessName(harness)} hat ${base(file.path)} ${c.deleted ? 'gelöscht' : 'geändert'}`}
        subtitle="Du hast in dieser Datei ungespeicherte Änderungen. beton überschreibt nichts, bis du dich entscheidest."
        footer={
          <>
            {!c.deleted && (
              <Btn variant="signal" onClick={() => void resolveConflict(sessionId, file.path, 'merge')}>
                Vergleichen und zusammenführen
              </Btn>
            )}
            <Btn onClick={() => void resolveConflict(sessionId, file.path, 'theirs')}>{c.deleted ? 'Löschung übernehmen' : 'Version des Agents laden'}</Btn>
            <Btn variant="ghost" className="ml-auto" disabled={file.saving} onClick={() => void resolveConflict(sessionId, file.path, 'mine')}>
              Meine Version speichern
            </Btn>
          </>
        }
      >
        <div className="grid grid-cols-1 gap-3 text-[12px] sm:grid-cols-2">
          <div className="min-w-0">
            <div className="mb-1 font-medium">Deine Änderung (ungespeichert)</div>
            <Region base={file.base} text={file.draft} />
          </div>
          <div className="min-w-0">
            <div className="mb-1 font-medium">Änderung des Agents · vor {ago} s</div>
            <Region base={file.base} text={c.theirs} />
          </div>
        </div>
        <p className="mt-3 text-[12px] text-muted-foreground">„Meine Version speichern“ überschreibt die Änderung des Agents. Er erfährt davon im nächsten Turn.</p>
      </DialogPanel>
    </Overlay>
  )
}

function MergeView({ sessionId, file }: { sessionId: string; file: OpenFile }) {
  const [merged, setMerged] = useState(file.draft)
  const doSave = () => void saveMerged(sessionId, file.path, merged)
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-1.5 text-[12px]">
        <span className="text-muted-foreground">Zusammenführen · links der Stand des Agents, rechts deine Version</span>
        <span className="ml-auto flex items-center gap-1">
          <Btn size="sm" variant="ghost" onClick={() => void resolveConflict(sessionId, file.path, 'cancel-merge')}>
            Abbrechen
          </Btn>
          <Btn size="sm" variant="primary" disabled={file.saving} onClick={doSave}>
            Zusammengeführt speichern
          </Btn>
        </span>
      </div>
      <div className="min-h-0 flex-1">
        <Suspense fallback={<Loading />}>
          <MergeEditor path={file.path} theirs={file.conflict?.theirs ?? ''} mine={file.draft} onChange={setMerged} onSave={doSave} />
        </Suspense>
      </div>
    </div>
  )
}

/** Zeigt den Konfliktdialog, egal welcher Tab gerade offen ist (WEB-009 AC1). */
export function ConflictHost({ sessionId, harness }: { sessionId: string; harness: string }) {
  const ws = useSessionWorkspace(sessionId)
  const files = Object.values(ws.files).filter((f) => f.conflict && !f.merging)
  const file = files.find((f) => f.path === ws.active) ?? files[0]
  return file ? <ConflictDialog sessionId={sessionId} harness={harness} file={file} /> : null
}

/** Editorbereich der Files-Ansicht: Tabs, Dateileiste, Monaco, Statuszeile (WEB-009). */
export function EditorArea({ sessionId }: { sessionId: string }) {
  const ws = useSessionWorkspace(sessionId)
  const file = ws.active ? ws.files[ws.active] : undefined
  const [selection, setSelection] = useState<Selected | undefined>()
  const [cursor, setCursor] = useState<[number, number]>([1, 1])
  const onRevealed = useCallback(() => file && clearReveal(sessionId, file.path), [sessionId, file?.path])
  useEffect(() => setSelection(undefined), [file?.path])

  if (!file) {
    return (
      <div className="flex min-w-0 flex-1 items-center justify-center p-6 text-center text-[13px] text-muted-foreground">
        Wähle links eine Datei. Mit ⌘P öffnest du sie über den Namen.
      </div>
    )
  }

  if (file.error?.kind === 'outside') {
    return (
      <div className="flex min-w-0 flex-1 flex-col">
        <EditorTabs sessionId={sessionId} />
        <div className="flex h-9 shrink-0 items-center border-b border-border bg-sidebar px-3 font-mono text-[12px] text-muted-foreground">{file.path}</div>
        <div className="p-4">
          <Callout tone="deny">
            <p className="font-semibold">Diese Datei liegt außerhalb des Workspace.</p>
            <p className="mt-1 text-muted-foreground">
              <code className="font-mono text-[12px]">{file.path}</code> zeigt aus dem Workspace hinaus. beton öffnet nur Dateien innerhalb des Worktrees dieser
              Session. Öffne den Ordner im Finder, wenn du ihn ansehen willst.
            </p>
          </Callout>
        </div>
      </div>
    )
  }

  const md = isMarkdown(file.path)
  const isDirty = dirty(file)
  const lines = selection ? selection.to - selection.from + 1 : 0
  let body
  if (file.status === 'loading') body = <Loading />
  else if (file.status === 'error')
    body = (
      <div className="p-4">
        <Callout>
          <p className="font-semibold">{file.error?.kind === 'missing' ? 'Die Datei gibt es nicht mehr.' : 'Die Datei lässt sich nicht öffnen.'}</p>
          <p className="mt-1 text-muted-foreground">{file.error?.message}</p>
        </Callout>
      </div>
    )
  else if (file.binary || file.tooLarge)
    body = (
      <div className="p-4">
        <Callout>
          <p className="font-semibold">{file.tooLarge ? 'Die Datei ist größer als 5 MiB.' : 'Das ist eine Binärdatei.'}</p>
          <p className="mt-1 text-muted-foreground">
            Der Editor zeigt nur Textdateien bis 5 MiB.{' '}
            <a className="underline" href={client.session(sessionId).workspace.downloadUrl(file.path)} download>
              Herunterladen
            </a>
          </p>
        </Callout>
      </div>
    )
  else if (file.merging) body = <MergeView sessionId={sessionId} file={file} />
  else {
    const editor = (
      <div className="relative min-h-0 min-w-0 flex-1 bg-card">
        <Suspense fallback={<Loading />}>
          <CodeEditor
            sessionId={sessionId}
            path={file.path}
            value={file.draft}
            rev={file.rev}
            openPaths={ws.open}
            reveal={file.reveal}
            onRevealed={onRevealed}
            onChange={(t) => setDraft(sessionId, file.path, t)}
            onSave={() => void save(sessionId, file.path)}
            onSelection={setSelection}
            onCursor={(l, c) => setCursor([l, c])}
          />
        </Suspense>
        {selection && selection.top >= 0 && (
          <div className="absolute left-14 z-10 inline-flex items-center gap-1 rounded-md border border-border bg-popover p-1 shadow-sm" style={{ top: selection.top + 4 }}>
            <span className="px-1.5 text-[11px] text-muted-foreground">{rangeLabel(selection.from, selection.to)} markiert</span>
            <Btn
              size="sm"
              variant="primary"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                attach(sessionId, { path: file.path, from: selection.from, to: selection.to, snippet: selection.text })
                setSelection(undefined)
              }}
            >
              <Paperclip className="size-3.5" /> An Agent anhängen
            </Btn>
          </div>
        )}
      </div>
    )
    body = md && file.preview ? (
      <div className="grid min-h-0 flex-1 grid-cols-2 overflow-hidden">
        <div className="flex min-h-0 border-r border-border">{editor}</div>
        <article className="overflow-auto px-5 py-4 text-[13px] leading-relaxed" data-testid="markdown-preview">
          <Markdown text={file.draft} />
        </article>
      </div>
    ) : (
      editor
    )
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <EditorTabs sessionId={sessionId} />
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-3 text-[11px] text-muted-foreground">
        <span className="min-w-0 truncate font-mono">{file.path}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {md && (
            <button
              onClick={() => togglePreview(sessionId, file.path)}
              aria-pressed={file.preview}
              className={cn('inline-flex items-center gap-1 rounded-sm px-1.5 py-px', file.preview ? 'bg-accent text-foreground' : 'hover:bg-accent')}
            >
              <Eye className="size-3" /> Vorschau
            </button>
          )}
          <span className="hidden sm:inline">Suchen ⌘F</span>
          <span aria-hidden>·</span>
          {file.saveError ? (
            <span className="text-deny" role="alert">
              Nicht gespeichert: {file.saveError}
            </span>
          ) : file.saving ? (
            <span>Speichert …</span>
          ) : isDirty ? (
            <button className="text-foreground hover:underline" onClick={() => void save(sessionId, file.path)} data-testid="save-state">
              Ungespeichert · ⌘S speichert
            </button>
          ) : (
            <span data-testid="save-state">Gespeichert</span>
          )}
        </span>
      </div>
      {body}
      <div className="flex h-6 shrink-0 items-center gap-3 border-t border-border px-3 text-[11px] text-muted-foreground">
        <span>{languageName(file.path)}</span>
        <span>UTF-8 · LF</span>
        <span className="ml-auto">{selection ? `${rangeLabel(selection.from, selection.to)} · ${lines} ${lines === 1 ? 'Zeile' : 'Zeilen'} markiert` : `Z. ${cursor[0]}, Sp. ${cursor[1]}`}</span>
      </div>
    </div>
  )
}
