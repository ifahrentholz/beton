import { useEffect, useRef } from 'react'
import { monaco } from './monaco'

/*
 * Monaco-Komponenten. Dieses Modul (und damit Monaco) wird nur über `React.lazy` geladen
 * (WEB-009 AC2).
 */

const FONT = '"IBM Plex Mono", ui-monospace, monospace'
const models = new Map<string, monaco.editor.ITextModel>()

void document.fonts?.ready.then(() => monaco.editor.remeasureFonts())

function modelFor(key: string, path: string, value: string): monaco.editor.ITextModel {
  let m = models.get(key)
  if (!m || m.isDisposed()) {
    // Die Sprache folgt aus der Dateiendung der URI (Monarch-Definitionen).
    m = monaco.editor.createModel(value, undefined, monaco.Uri.from({ scheme: 'inmemory', path: `/${key}/${path}` }))
    models.set(key, m)
  }
  return m
}

/** Modelle geschlossener Dateien freigeben. */
function prune(prefix: string, keep: readonly string[]): void {
  for (const [key, m] of models) {
    if (key.startsWith(prefix) && !keep.includes(key)) {
      m.dispose()
      models.delete(key)
    }
  }
}

export interface Selected {
  from: number
  to: number
  /** Abstand der Unterkante der Auswahl zum oberen Rand des Editors (px). */
  top: number
  text: string
}

export function CodeEditor({
  sessionId,
  path,
  value,
  rev,
  openPaths,
  reveal,
  onRevealed,
  onChange,
  onSave,
  onSelection,
  onCursor,
}: {
  sessionId: string
  path: string
  value: string
  /** Steigt, wenn `value` von außen ersetzt wurde. */
  rev: number
  openPaths: readonly string[]
  reveal?: number | undefined
  onRevealed: () => void
  onChange: (text: string) => void
  onSave: () => void
  onSelection: (s: Selected | undefined) => void
  onCursor: (line: number, column: number) => void
}) {
  const host = useRef<HTMLDivElement>(null)
  const editor = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const cb = useRef({ onChange, onSave, onSelection, onCursor })
  cb.current = { onChange, onSave, onSelection, onCursor }
  const key = `${sessionId}/${path}`

  useEffect(() => {
    const ed = monaco.editor.create(host.current!, {
      automaticLayout: true,
      fontFamily: FONT,
      fontSize: 12,
      lineHeight: 19,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      renderLineHighlight: 'line',
      padding: { top: 8 },
      fixedOverflowWidgets: true,
    })
    editor.current = ed
    const report = () => {
      const sel = ed.getSelection()
      const model = ed.getModel()
      if (!sel || !model || sel.isEmpty()) return cb.current.onSelection(undefined)
      const to = sel.endColumn === 1 && sel.endLineNumber > sel.startLineNumber ? sel.endLineNumber - 1 : sel.endLineNumber
      const from = sel.startLineNumber
      const top = ed.getTopForLineNumber(to + 1) - ed.getScrollTop()
      const text = model.getValueInRange(new monaco.Range(from, 1, to, model.getLineMaxColumn(to)))
      cb.current.onSelection({ from, to, top, text })
    }
    const subs = [
      ed.onDidChangeModelContent(() => cb.current.onChange(ed.getValue())),
      ed.onDidChangeCursorSelection(report),
      ed.onDidScrollChange(report),
      ed.onDidChangeCursorPosition((e) => cb.current.onCursor(e.position.lineNumber, e.position.column)),
    ]
    ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => cb.current.onSave())
    return () => {
      for (const s of subs) s.dispose()
      ed.dispose()
      editor.current = null
    }
  }, [])

  // Datei wechseln: Modell je Datei (Undo-Verlauf bleibt erhalten).
  useEffect(() => {
    const ed = editor.current
    if (!ed) return
    const m = modelFor(key, path, value)
    if (ed.getModel() !== m) ed.setModel(m)
    // Abhängigkeiten bewusst: `value` nur beim Anlegen.
  }, [key, path])

  // Inhalt von außen ersetzt (neu geladen, Version des Agents übernommen).
  useEffect(() => {
    const m = models.get(key)
    if (!m || m.getValue() === value) return
    m.pushEditOperations([], [{ range: m.getFullModelRange(), text: value }], () => null)
    // Abhängigkeiten bewusst: nur bei neuer Revision.
  }, [key, rev])

  useEffect(() => {
    if (!reveal || !editor.current) return
    editor.current.revealLineInCenter(reveal)
    editor.current.setPosition({ lineNumber: reveal, column: 1 })
    editor.current.focus()
    onRevealed()
  }, [reveal, key, onRevealed])

  useEffect(() => prune(`${sessionId}/`, openPaths.map((p) => `${sessionId}/${p}`)), [sessionId, openPaths])

  return <div ref={host} className="h-full min-h-0 w-full" data-testid="code-editor" />
}

/** Zusammenführen: links der Stand des Agents, rechts die eigene, bearbeitbare Version. */
export function MergeEditor({ path, theirs, mine, onChange, onSave }: { path: string; theirs: string; mine: string; onChange: (text: string) => void; onSave: () => void }) {
  const host = useRef<HTMLDivElement>(null)
  const cb = useRef({ onChange, onSave })
  cb.current = { onChange, onSave }
  useEffect(() => {
    const ed = monaco.editor.createDiffEditor(host.current!, {
      automaticLayout: true,
      fontFamily: FONT,
      fontSize: 12,
      lineHeight: 19,
      minimap: { enabled: false },
      originalEditable: false,
      renderSideBySide: true,
      useInlineViewWhenSpaceIsLimited: false,
    })
    const uri = (side: string) => monaco.Uri.from({ scheme: 'inmemory', path: `/merge-${side}-${Date.now()}/${path}` })
    const original = monaco.editor.createModel(theirs, undefined, uri('theirs'))
    const modified = monaco.editor.createModel(mine, undefined, uri('mine'))
    ed.setModel({ original, modified })
    const sub = modified.onDidChangeContent(() => cb.current.onChange(modified.getValue()))
    ed.getModifiedEditor().addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => cb.current.onSave())
    return () => {
      sub.dispose()
      ed.dispose()
      original.dispose()
      modified.dispose()
    }
    // Abhängigkeiten bewusst: einmal je Konflikt.
  }, [path, theirs])
  return <div ref={host} className="h-full min-h-0 w-full" data-testid="merge-editor" />
}
