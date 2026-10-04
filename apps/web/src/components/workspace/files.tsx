import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ChangeStatus, SearchHit, SessionWorktree, TreeEntry } from '@beton/sdk'
import { ChevronDown, ChevronRight, File, Folder, Lock, Search, X } from 'lucide-react'
import { client } from '@/lib/client'
import { cn } from '@/lib/utils'
import { openFile, useSessionWorkspace } from '@/store/workspace'
import { EditorArea } from './editor'
import { useWorkspaceInfo } from './info'
import { Segmented, TreeStatus } from './ui'

/** Nicht committete Änderungen als Pfad → Status (für die Buchstaben im Baum). */
function useStatusMap(sessionId: string, version: number): Map<string, ChangeStatus> {
  const info = useWorkspaceInfo(sessionId)
  const q = useQuery({
    queryKey: ['ws-changes', sessionId, 'uncommitted', '', version],
    queryFn: () => client.session(sessionId).workspace.changes('uncommitted', { limit: 200 }),
    enabled: info.data?.git_repo === true,
    retry: false,
    staleTime: Infinity,
  })
  return useMemo(() => new Map((q.data?.items ?? []).map((f) => [f.path, f.status])), [q.data])
}

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value)
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms)
    return () => clearTimeout(t)
  }, [value, ms])
  return v
}

function TreeLevel({
  sessionId,
  dir,
  depth,
  version,
  expanded,
  toggle,
  statuses,
}: {
  sessionId: string
  dir: string
  depth: number
  version: number
  expanded: Set<string>
  toggle: (p: string) => void
  statuses: Map<string, ChangeStatus>
}) {
  const ws = useSessionWorkspace(sessionId)
  const q = useQuery({
    queryKey: ['ws-tree', sessionId, dir, version],
    queryFn: async () => {
      const items: TreeEntry[] = []
      let cursor: string | undefined
      do {
        const page = await client.session(sessionId).workspace.tree(dir, { limit: 200, ...(cursor ? { cursor } : {}) })
        items.push(...page.items)
        cursor = page.next_cursor ?? undefined
      } while (cursor && items.length < 5000)
      return items
    },
    placeholderData: (prev) => prev,
    staleTime: Infinity,
  })
  if (q.isError && depth === 0) {
    return <p className="px-3 py-2 text-[12px] text-muted-foreground">Der Workspace dieser Session ist nicht lesbar.</p>
  }
  // Verzeichnisse zuerst, dann Dateien (jeweils nach Namen, wie vom Server sortiert).
  const items = [...(q.data ?? [])].sort((a, b) => Number(b.kind === 'dir') - Number(a.kind === 'dir'))
  return (
    <>
      {items.map((n) => {
        const isDir = n.kind === 'dir'
        const open = isDir && expanded.has(n.path)
        const active = n.path === ws.active
        const outside = ws.outside.includes(n.path)
        const status = statuses.get(n.path)
        return (
          <div key={n.path} role="none">
            <div
              role="treeitem"
              aria-selected={active}
              aria-expanded={isDir ? open : undefined}
              tabIndex={0}
              data-path={n.path}
              title={outside ? `${n.path} zeigt aus dem Workspace hinaus – nicht lesbar` : n.path}
              onClick={() => (isDir ? toggle(n.path) : void openFile(sessionId, n.path))}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  if (isDir) toggle(n.path)
                  else void openFile(sessionId, n.path)
                }
              }}
              className={cn('flex cursor-default items-center gap-1 py-[3px] pr-2', active ? 'bg-accent' : 'hover:bg-accent/60', outside && 'text-muted-foreground')}
              style={{ paddingLeft: 8 + depth * 14 }}
            >
              {isDir ? (
                open ? <ChevronDown className="size-3 shrink-0 text-muted-foreground" /> : <ChevronRight className="size-3 shrink-0 text-muted-foreground" />
              ) : (
                <span className="w-3 shrink-0" />
              )}
              {isDir ? <Folder className="size-3.5 shrink-0 text-muted-foreground" /> : <File className="size-3.5 shrink-0 text-muted-foreground" />}
              <span className="truncate">{n.name}</span>
              {outside && <Lock className="ml-auto size-3 shrink-0" aria-label="außerhalb des Workspace" />}
              {!outside && status && <TreeStatus status={status} />}
            </div>
            {open && (
              <div role="group">
                <TreeLevel sessionId={sessionId} dir={n.path} depth={depth + 1} version={version} expanded={expanded} toggle={toggle} statuses={statuses} />
              </div>
            )}
          </div>
        )
      })}
    </>
  )
}

/** „Datei öffnen ⌘P“: Dateinamensuche mit Auswahlliste. */
function QuickOpen({ sessionId }: { sessionId: string }) {
  const [q, setQ] = useState('')
  const [index, setIndex] = useState(0)
  const [focused, setFocused] = useState(false)
  const input = useRef<HTMLInputElement>(null)
  const term = useDebounced(q.trim(), 120)
  const res = useQuery({
    queryKey: ['ws-search', sessionId, 'name', term],
    queryFn: () => client.session(sessionId).workspace.search(term, 'name', { limit: 20 }),
    enabled: term.length > 0,
    staleTime: 5_000,
  })
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'p') {
        e.preventDefault()
        input.current?.focus()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
  const hits = term ? (res.data?.items ?? []) : []
  const pick = (h: SearchHit | undefined) => {
    if (!h) return
    void openFile(sessionId, h.path)
    setQ('')
    input.current?.blur()
  }
  return (
    <div className="relative flex-1">
      <input
        ref={input}
        value={q}
        onChange={(e) => {
          setQ(e.target.value)
          setIndex(0)
        }}
        onFocus={() => setFocused(true)}
        onBlur={() => setTimeout(() => setFocused(false), 150)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown') setIndex((i) => Math.min(i + 1, hits.length - 1))
          else if (e.key === 'ArrowUp') setIndex((i) => Math.max(i - 1, 0))
          else if (e.key === 'Enter') pick(hits[index])
          else if (e.key === 'Escape') setQ('')
        }}
        placeholder="Datei öffnen  ⌘P"
        aria-label="Datei öffnen"
        className="flex h-7 w-full items-center rounded-md border border-input bg-card px-2.5 text-xs outline-none placeholder:text-muted-foreground focus:outline-2 focus:outline-ring"
      />
      {focused && hits.length > 0 && (
        <div role="listbox" className="absolute top-8 right-0 left-0 z-30 max-h-72 overflow-y-auto rounded-lg border border-border bg-popover py-1 shadow-[0_16px_40px_-20px_rgb(0_0_0/0.5)]">
          {hits.map((h, i) => (
            <div
              key={h.path}
              role="option"
              aria-selected={i === index}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pick(h)}
              className={cn('mx-1 truncate rounded-md px-2 py-1 font-mono text-[12px]', i === index ? 'bg-accent' : 'hover:bg-accent/60')}
            >
              {h.path}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

/** Fundstelle mit hervorgehobenem Suchbegriff (Smart-Case wie auf dem Server). */
function Hit({ text, term }: { text: string; term: string }) {
  const sensitive = term !== term.toLowerCase()
  const at = sensitive ? text.indexOf(term) : text.toLowerCase().indexOf(term.toLowerCase())
  if (at < 0 || !term) return <span className="text-muted-foreground">{text.trimStart()}</span>
  const pre = text.slice(0, at).trimStart()
  return (
    <>
      <span className="text-muted-foreground">{pre.length > 40 ? `…${pre.slice(-40)}` : pre}</span>
      <mark className="rounded-[2px] bg-foreground/15 text-foreground">{text.slice(at, at + term.length)}</mark>
      <span className="text-muted-foreground">{text.slice(at + term.length)}</span>
    </>
  )
}

/** Inhalts- und Dateinamensuche im Workspace (SES-017). */
function SearchPane({ sessionId, onClose }: { sessionId: string; onClose: () => void }) {
  const [q, setQ] = useState('')
  const [mode, setMode] = useState<'name' | 'content'>('content')
  const term = useDebounced(q.trim(), 200)
  const res = useQuery({
    queryKey: ['ws-search', sessionId, mode, term],
    queryFn: () => client.session(sessionId).workspace.search(term, mode, { limit: 200 }),
    enabled: term.length > 0,
    staleTime: 5_000,
  })
  const hits = term ? (res.data?.items ?? []) : []
  const groups = useMemo(() => {
    const m = new Map<string, SearchHit[]>()
    for (const h of hits) m.set(h.path, [...(m.get(h.path) ?? []), h])
    return [...m.entries()]
  }, [hits])
  const sensitive = term !== term.toLowerCase()
  let summary: ReactNode = null
  if (term && res.data) {
    summary =
      mode === 'content'
        ? `${hits.length}${res.data.next_cursor ? '+' : ''} Treffer in ${groups.length} ${groups.length === 1 ? 'Datei' : 'Dateien'} · .gitignore beachtet`
        : `${hits.length}${res.data.next_cursor ? '+' : ''} ${hits.length === 1 ? 'Datei' : 'Dateien'} · .gitignore beachtet`
  }
  return (
    <div className="flex min-h-0 w-full shrink-0 flex-col border-border @xl:w-[300px] @xl:border-r" data-testid="search-pane">
      <div className="flex flex-col gap-2 border-b border-border p-2">
        <div className="flex items-center gap-1">
          <input
            autoFocus
            value={q}
            onChange={(e) => setQ(e.target.value)}
            aria-label="Im Workspace suchen"
            placeholder="Im Workspace suchen"
            className="h-7 min-w-0 flex-1 rounded-md border border-input bg-card px-2.5 text-xs outline-none focus:outline-2 focus:outline-ring"
          />
          <button onClick={onClose} aria-label="Suche schließen" className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent">
            <X className="size-3.5" />
          </button>
        </div>
        <div className="flex items-center gap-2">
          <Segmented
            label="Suchart"
            value={mode}
            onChange={setMode}
            items={[
              { id: 'name', label: 'Dateiname' },
              { id: 'content', label: 'Inhalt' },
            ]}
          />
          <span className="ml-auto text-[11px] text-muted-foreground">{sensitive ? 'Groß/klein beachtet' : 'Groß/klein egal'}</span>
        </div>
      </div>
      {summary && <div className="px-3 py-1.5 text-[11px] text-muted-foreground">{summary}</div>}
      {term && res.data && hits.length === 0 && <div className="px-3 py-1.5 text-[12px] text-muted-foreground">Keine Treffer für „{term}“.</div>}
      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {groups.map(([file, list]) => (
          <div key={file} className="mt-1">
            <button onClick={() => void openFile(sessionId, file)} className="flex w-full items-center gap-1.5 px-3 py-1 text-left text-[12px] font-medium hover:bg-accent/60">
              <ChevronDown className="size-3 text-muted-foreground" />
              <span className="truncate font-mono">{file}</span>
              {mode === 'content' && <span className="ml-auto text-[11px] font-normal text-muted-foreground">{list.length}</span>}
            </button>
            {mode === 'content' &&
              list.map((h) => (
                <button
                  key={`${h.line}:${h.column}`}
                  onClick={() => void openFile(sessionId, h.path, h.line)}
                  className="flex w-full gap-2 py-1 pr-2 pl-7 text-left font-mono text-[11.5px] hover:bg-accent/60"
                >
                  <span className="w-6 shrink-0 text-right text-muted-foreground">{h.line}</span>
                  <span className="truncate">
                    <Hit text={h.text ?? ''} term={term} />
                  </span>
                </button>
              ))}
          </div>
        ))}
      </div>
    </div>
  )
}

/** Files-Tab: Dateibaum oder Suche links, Editor rechts (SES-017, WEB-009). */
export function FilesPanel({ sessionId, version, worktree }: { sessionId: string; version: number; worktree?: SessionWorktree | undefined }) {
  const [searching, setSearching] = useState(false)
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const statuses = useStatusMap(sessionId, version)
  const toggle = (p: string) =>
    setExpanded((s) => {
      const n = new Set(s)
      if (n.has(p)) n.delete(p)
      else n.add(p)
      return n
    })
  return (
    <div className="@container flex h-full min-h-0">
      <div className="flex h-full min-h-0 w-full flex-col @xl:flex-row">
        {searching ? (
          <SearchPane sessionId={sessionId} onClose={() => setSearching(false)} />
        ) : (
          <div className="flex max-h-[40%] w-full shrink-0 flex-col border-b border-border @xl:max-h-none @xl:w-56 @xl:border-r @xl:border-b-0">
            <div className="flex items-center gap-1 p-2">
              <QuickOpen sessionId={sessionId} />
              <button
                onClick={() => setSearching(true)}
                aria-label="Im Workspace suchen"
                className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent"
              >
                <Search className="size-3.5" />
              </button>
            </div>
            {worktree && (
              <div className="truncate px-3 pb-1 font-mono text-[10px] text-muted-foreground" title={worktree.path}>
                Worktree · {worktree.branch}
              </div>
            )}
            <div role="tree" aria-label="Dateien" className="min-h-0 flex-1 overflow-y-auto pb-2 text-[13px]" data-testid="file-tree">
              <TreeLevel sessionId={sessionId} dir="" depth={0} version={version} expanded={expanded} toggle={toggle} statuses={statuses} />
            </div>
          </div>
        )}
        <EditorArea sessionId={sessionId} />
      </div>
    </div>
  )
}
