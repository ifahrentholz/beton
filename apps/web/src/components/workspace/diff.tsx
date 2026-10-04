import { memo, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { ChangedFile, ChangeScope, FileDiff } from '@beton/sdk'
import { Check, Link2, Paperclip } from 'lucide-react'
import { client } from '@/lib/client'
import { highlightChunked, type Lines } from '@/lib/highlight'
import { diffRows, parseAnchor, shikiLang, type DiffRow } from '@/lib/workspace'
import { cn } from '@/lib/utils'
import { attach } from '@/store/workspace'
import { Btn } from './ui'

/** Zeilenhöhen (fest, damit die Virtualisierung ohne Messen auskommt, WEB-011 AC1). */
const H = { header: 27, hunk: 20, line: 20, spacer: 12, note: 32 } as const

type Row =
  | { kind: 'header'; file: ChangedFile }
  | { kind: 'pending'; file: ChangedFile }
  | { kind: 'note'; file: ChangedFile; text: string }
  | { kind: 'spacer' }
  | { kind: 'diff'; file: ChangedFile; row: DiffRow; last: boolean; oldIdx: number; newIdx: number }

type Loaded = FileDiff | { error: string }

const SCOPE_NOTE: Record<ChangeScope, string> = { uncommitted: 'nicht committet', branch: 'Branch', turn: 'pro Turn' }

function headerLabel(f: ChangedFile): string {
  if (f.status === 'added') return `${f.path} (neu)`
  if (f.status === 'deleted') return `${f.path} (gelöscht)`
  if (f.status === 'renamed') return `${f.path} (umbenannt von ${f.old_path ?? '?'})`
  return f.path
}

/** Diffs pro Datei bei Bedarf laden (große Änderungen, WEB-011). */
function useDiffs(sessionId: string, scope: ChangeScope, turn: string | undefined, version: number) {
  const key = `${scope}|${turn ?? ''}|${version}`
  const [state, setState] = useState<{ key: string; diffs: Map<string, Loaded> }>({ key, diffs: new Map() })
  const inflight = useRef(new Set<string>())
  const diffs = state.key === key ? state.diffs : new Map<string, Loaded>()
  useEffect(() => {
    inflight.current = new Set()
    setState({ key, diffs: new Map() })
  }, [key])
  const load = useCallback(
    (path: string) => {
      const id = `${key}|${path}`
      if (inflight.current.has(id)) return
      inflight.current.add(id)
      client
        .session(sessionId)
        .workspace.diff(path, scope, turn)
        .then(
          (d): Loaded => d,
          (e: unknown): Loaded => ({ error: e instanceof Error ? e.message : 'Diff nicht ladbar' }),
        )
        .then((d) =>
          setState((s) => {
            if (s.key !== key) return s
            const m = new Map(s.diffs)
            m.set(path, d)
            return { key, diffs: m }
          }),
        )
    },
    [sessionId, scope, turn, key],
  )
  return { diffs, load }
}

/** Hervorhebung je Datei (alte und neue Seite), abschnittsweise im Hintergrund. */
function useHighlight(rowsByFile: Map<string, DiffRow[]>): (rows: DiffRow[] | undefined) => { old: Lines; new: Lines } | undefined {
  const tokens = useRef(new WeakMap<DiffRow[], { old: Lines; new: Lines }>())
  const [, setTick] = useState(0)
  // Höchstens ein Neuzeichnen pro Frame, egal wie viele Abschnitte fertig werden.
  const frame = useRef(0)
  const bump = useCallback(() => {
    if (frame.current) return
    frame.current = requestAnimationFrame(() => {
      frame.current = 0
      setTick((n) => n + 1)
    })
  }, [])
  const abort = useRef(new AbortController())
  useEffect(() => {
    const ctrl = new AbortController()
    abort.current = ctrl
    return () => {
      ctrl.abort()
      cancelAnimationFrame(frame.current)
      frame.current = 0
    }
  }, [])
  useEffect(() => {
    for (const [path, rows] of rowsByFile) {
      if (tokens.current.has(rows)) continue
      const lang = shikiLang(path)
      if (!lang) continue
      const entry = { old: [] as Lines, new: [] as Lines }
      tokens.current.set(rows, entry)
      const lines = (pred: (r: Extract<DiffRow, { kind: 'line' }>) => boolean) =>
        rows.filter((r): r is Extract<DiffRow, { kind: 'line' }> => r.kind === 'line' && pred(r)).map((r) => r.text)
      const run = (side: 'old' | 'new', src: string[]) =>
        highlightChunked(
          src,
          lang,
          (from, chunk) => {
            for (let i = 0; i < chunk.length; i++) entry[side][from + i] = chunk[i]!
            bump()
          },
          abort.current.signal,
        ).catch(() => undefined)
      void run('new', lines((r) => r.type !== 'delete')).then(() => run('old', lines((r) => r.type !== 'add')))
    }
  }, [rowsByFile, bump])
  return (rows) => (rows ? tokens.current.get(rows) : undefined)
}

function Pending({ file, load }: { file: ChangedFile; load: (p: string) => void }) {
  useEffect(() => load(file.path), [file.path, load])
  return <span className="text-muted-foreground">Diff lädt …</span>
}

const Code = memo(function Code({ text, tokens }: { text: string; tokens: [string, string][] | undefined }) {
  if (!tokens) return <>{text}</>
  return (
    <>
      {tokens.map(([t, color], i) => (
        <span key={i} style={color ? { color } : undefined}>
          {t}
        </span>
      ))}
    </>
  )
})

/**
 * Eine Diff-Zeile. Gemerkt: Beim Scrollen zeichnet React nur neu hinzukommende Zeilen
 * (feste Höhen, gleiche Position je Index), nicht alle sichtbaren (WEB-011 AC1).
 */
const LineRow = memo(function LineRow({
  file,
  row,
  last,
  start,
  size,
  tokens,
  onAnchor,
  toolbar,
}: {
  file: string
  row: Extract<DiffRow, { kind: 'line' }>
  last: boolean
  start: number
  size: number
  tokens: [string, string][] | undefined
  onAnchor: (file: string, line: string) => void
  toolbar?: ReactNode
}) {
  const num = row.type === 'delete' ? row.old : row.new
  const side = row.type === 'delete' ? 'alt' : 'neu'
  return (
    <div
      style={{ transform: `translateY(${start}px)`, height: size }}
      data-line={row.anchor}
      data-anchored={toolbar ? 'true' : undefined}
      className={cn(
        'absolute top-0 left-0 flex w-full border-x border-border bg-card font-mono text-[12px] leading-5',
        row.type === 'add' && 'bg-ok-soft',
        row.type === 'delete' && 'bg-deny-soft',
        toolbar && 'ring-1 ring-foreground/50 ring-inset',
        last && 'rounded-b-md border-b',
      )}
    >
      <button
        onClick={() => onAnchor(file, row.anchor)}
        className="w-10 shrink-0 pr-2 text-right text-muted-foreground select-none hover:text-foreground hover:underline"
        aria-label={`Link auf Zeile ${num} (${side})`}
        title="Link auf diese Zeile"
      >
        {num}
      </button>
      <span className="w-4 shrink-0 text-muted-foreground select-none" aria-hidden>
        {row.type === 'add' ? '+' : row.type === 'delete' ? '−' : ''}
      </span>
      <span className="whitespace-pre">
        <Code text={row.text} tokens={tokens} />
      </span>
      {toolbar}
    </div>
  )
})

export interface Anchor {
  file: string
  line: string
}

/**
 * Virtualisierte Diff-Liste über alle geänderten Dateien (WEB-011): Datei für Datei lazy
 * geladen, Zeilennummern als teilbare Anker (AC2), auch bei 5 000 Zeilen flüssig (AC1).
 */
export function DiffList({
  sessionId,
  scope,
  turn,
  files,
  version,
  anchor,
  onAnchor,
  copyLink,
  jump,
  onTopFile,
}: {
  sessionId: string
  scope: ChangeScope
  turn?: string | undefined
  files: ChangedFile[]
  version: number
  anchor?: Anchor | undefined
  onAnchor: (file: string, line: string) => void
  copyLink: (file: string, line: string) => Promise<void>
  jump?: { path: string; n: number } | undefined
  onTopFile: (path: string) => void
}) {
  const { diffs, load } = useDiffs(sessionId, scope, turn, version)

  const rowsByFile = useMemo(() => {
    const m = new Map<string, DiffRow[]>()
    for (const [path, d] of diffs) if (!('error' in d)) m.set(path, diffRows(d))
    return m
  }, [diffs])
  const tokensOf = useHighlight(rowsByFile)

  const { rows, headers, maxLen } = useMemo(() => {
    const rows: Row[] = []
    const headers = new Map<string, number>()
    let maxLen = 0
    for (const f of files) {
      headers.set(f.path, rows.length)
      rows.push({ kind: 'header', file: f })
      const d = diffs.get(f.path)
      if (!d) rows.push({ kind: 'pending', file: f })
      else if ('error' in d) rows.push({ kind: 'note', file: f, text: d.error })
      else if (d.binary) rows.push({ kind: 'note', file: f, text: 'Binärdatei – kein Text-Diff.' })
      else {
        const list = rowsByFile.get(f.path) ?? []
        if (list.length === 0) rows.push({ kind: 'note', file: f, text: 'Keine Textänderungen (nur Modus oder Umbenennung).' })
        let oldIdx = 0
        let newIdx = 0
        list.forEach((r, i) => {
          rows.push({ kind: 'diff', file: f, row: r, last: i === list.length - 1, oldIdx, newIdx })
          if (r.kind === 'line') {
            if (r.type !== 'add') oldIdx++
            if (r.type !== 'delete') newIdx++
            if (r.text.length > maxLen) maxLen = r.text.length
          }
        })
      }
      rows.push({ kind: 'spacer' })
    }
    return { rows, headers, maxLen: Math.min(maxLen, 2000) }
  }, [files, diffs, rowsByFile])

  const scrollRef = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => {
      const r = rows[i]!
      return r.kind === 'header' ? H.header : r.kind === 'spacer' ? H.spacer : r.kind === 'pending' || r.kind === 'note' ? H.note : r.row.kind === 'hunk' ? H.hunk : H.line
    },
    overscan: 12,
  })

  // Datei-Navigation: zum Kopf der gewählten Datei springen.
  useEffect(() => {
    if (!jump) return
    const idx = headers.get(jump.path)
    if (idx !== undefined) virtualizer.scrollToIndex(idx, { align: 'start' })
    // Abhängigkeiten bewusst: nur bei neuem Sprung.
  }, [jump])

  // Geteilter Link: Datei laden und die Zeile in die Mitte holen (einmal).
  const anchored = useRef<string | undefined>(undefined)
  useEffect(() => {
    if (!anchor) return
    const id = `${anchor.file}#${anchor.line}`
    if (anchored.current === id) return
    if (!diffs.has(anchor.file)) {
      load(anchor.file)
      return
    }
    const idx = rows.findIndex((r) => r.kind === 'diff' && r.file.path === anchor.file && r.row.kind === 'line' && r.row.anchor === anchor.line)
    if (idx >= 0) {
      anchored.current = id
      virtualizer.scrollToIndex(idx, { align: 'center' })
    }
  }, [anchor, diffs, rows, load, virtualizer])

  const items = virtualizer.getVirtualItems()
  const top = items.find((v) => v.start + v.size > (scrollRef.current?.scrollTop ?? 0))
  const topFile = top ? (rows[top.index] && 'file' in rows[top.index]! ? (rows[top.index] as { file: ChangedFile }).file.path : undefined) : undefined
  useEffect(() => {
    if (topFile) onTopFile(topFile)
  }, [topFile, onTopFile])

  const [copied, setCopied] = useState<string | undefined>()
  const active = anchor ? parseAnchor(anchor.line) : undefined
  const onAnchorRef = useRef(onAnchor)
  onAnchorRef.current = onAnchor
  const anchorCb = useCallback((file: string, line: string) => onAnchorRef.current(file, line), [])

  return (
    <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto px-3 pt-3" data-testid="diff-list">
      <div className="relative" style={{ height: virtualizer.getTotalSize(), width: `max(100%, calc(${maxLen}ch + 5rem))` }}>
        {items.map((v) => {
          const r = rows[v.index]!
          const style = { transform: `translateY(${v.start}px)`, height: v.size }
          const box = 'absolute top-0 left-0 w-full border-x border-border bg-card font-mono text-[12px]'
          if (r.kind === 'spacer') return <div key={v.key} className="absolute top-0 left-0 w-full" style={style} />
          if (r.kind === 'header') {
            return (
              <div key={v.key} style={style} className={cn(box, 'flex items-center rounded-t-md border-t border-b bg-sunken px-3 text-[11px] text-muted-foreground')} data-file={r.file.path}>
                <span className="sticky left-3 truncate">{headerLabel(r.file)}</span>
              </div>
            )
          }
          if (r.kind === 'pending' || r.kind === 'note') {
            return (
              <div key={v.key} style={style} className={cn(box, 'flex items-center rounded-b-md border-b px-3 font-sans text-[12px]')}>
                {r.kind === 'pending' ? <Pending file={r.file} load={load} /> : <span className="text-muted-foreground">{r.text}</span>}
              </div>
            )
          }
          const dr = r.row
          if (dr.kind === 'hunk') {
            return (
              <div key={v.key} style={style} className={cn(box, 'flex items-center bg-sunken/60 px-3 text-[11px] text-muted-foreground', r.last && 'rounded-b-md border-b')}>
                {dr.text}
              </div>
            )
          }
          const num = dr.type === 'delete' ? dr.old : dr.new
          const side = dr.type === 'delete' ? 'alt' : 'neu'
          const isAnchored = anchor?.file === r.file.path && anchor.line === dr.anchor
          const t = tokensOf(rowsByFile.get(r.file.path))
          const lineTokens = dr.type === 'delete' ? t?.old[r.oldIdx] : t?.new[r.newIdx]
          return (
            <LineRow
              key={v.key}
              file={r.file.path}
              row={dr}
              last={r.last}
              start={v.start}
              size={v.size}
              tokens={lineTokens}
              onAnchor={anchorCb}
              toolbar={
                isAnchored && active ? (
                  <span className="sticky right-2 ml-auto flex items-center gap-0.5 self-center rounded-md border border-border bg-popover p-0.5 font-sans shadow-sm">
                    <span className="px-1.5 font-mono text-[10px] text-muted-foreground">
                      Z. {num} {side}
                    </span>
                    <Btn
                      size="sm"
                      variant="ghost"
                      title="Link auf diese Zeile kopieren"
                      aria-label="Link auf diese Zeile kopieren"
                      onClick={() =>
                        void copyLink(r.file.path, dr.anchor).then(() => {
                          setCopied(dr.anchor)
                          setTimeout(() => setCopied(undefined), 1500)
                        })
                      }
                    >
                      {copied === dr.anchor ? <Check className="size-3.5" /> : <Link2 className="size-3.5" />}
                    </Btn>
                    <Btn
                      size="sm"
                      variant="ghost"
                      onClick={() => attach(sessionId, { path: r.file.path, from: num ?? 0, to: num ?? 0, note: `${side}, ${SCOPE_NOTE[scope]}`, snippet: dr.text })}
                    >
                      <Paperclip className="size-3.5" /> An Agent anhängen
                    </Btn>
                  </span>
                ) : undefined
              }
            />
          )
        })}
      </div>
    </div>
  )
}

