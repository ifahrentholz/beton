import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { BetonError, type ChangedFile, type ChangeScope, type Event, type SessionWorktree } from '@beton/sdk'
import { client } from '@/lib/client'
import { useWorkspaceInfo } from './info'
import { turnLabel, turnsOf } from '@/lib/workspace'
import { cn } from '@/lib/utils'
import { DiffList, type Anchor } from './diff'
import { AddDel, Callout, FileStatus, Segmented } from './ui'

function FileList({ files, active, onPick }: { files: ChangedFile[]; active?: string | undefined; onPick: (path: string) => void }) {
  return (
    <div className="flex max-h-48 shrink-0 flex-col overflow-y-auto border-b border-border py-1" role="list" aria-label="Geänderte Dateien">
      {files.map((f) => (
        <button
          key={f.path}
          role="listitem"
          onClick={() => onPick(f.path)}
          className={cn('flex items-center gap-2 border-l-2 px-3 py-1 text-left text-[12px]', f.path === active ? 'border-foreground bg-accent' : 'border-transparent hover:bg-accent/60')}
          data-testid="changed-file"
        >
          <FileStatus status={f.status} />
          <span className="min-w-0 flex-1 truncate font-mono">{f.path}</span>
          <AddDel add={f.additions} del={f.deletions} />
        </button>
      ))}
    </div>
  )
}

const short = (sha: string | undefined) => (sha ?? '').slice(0, 7)

/**
 * Changes-Tab (SES-018, WEB-011): drei Sichten, Dateiliste und Diffs. Ohne Git-Repository
 * stehen nur die Änderungen pro Turn zur Verfügung.
 */
export function ChangesPanel({
  sessionId,
  events,
  version,
  worktree,
  initial,
  onAnchor,
  copyLink,
}: {
  sessionId: string
  events: readonly Event[] | undefined
  version: number
  worktree?: SessionWorktree | undefined
  initial: { scope?: ChangeScope | undefined; turn?: string | undefined; anchor?: Anchor | undefined }
  onAnchor: (p: { scope: ChangeScope; turn?: string | undefined; file: string; line: string }) => void
  copyLink: (p: { scope: ChangeScope; turn?: string | undefined; file: string; line: string }) => Promise<void>
}) {
  const [scope, setScope] = useState<ChangeScope>(initial.scope ?? 'uncommitted')
  const [chosen, setChosen] = useState(initial.scope !== undefined)
  const turns = useMemo(() => turnsOf(events), [events])
  const [turnChoice, setTurn] = useState<string | undefined>(initial.turn)
  const turn = scope === 'turn' ? (turnChoice ?? turns.at(-1)?.id) : undefined
  const [jump, setJump] = useState<{ path: string; n: number } | undefined>()
  const [activeFile, setActiveFile] = useState<string | undefined>()
  const info = useWorkspaceInfo(sessionId)

  useEffect(() => {
    if (initial.scope) setScope(initial.scope)
    if (initial.turn) setTurn(initial.turn)
  }, [initial.scope, initial.turn])

  const q = useQuery({
    queryKey: ['ws-changes', sessionId, scope, turn ?? '', version],
    queryFn: async () => {
      const ws = client.session(sessionId).workspace
      const page = await ws.changes(scope, { ...(turn ? { turn } : {}), limit: 200 })
      const items = [...page.items]
      let cursor = page.next_cursor
      while (cursor && items.length < 5000) {
        const next = await ws.changes(scope, { ...(turn ? { turn } : {}), cursor, limit: 200 })
        items.push(...next.items)
        cursor = next.next_cursor
      }
      return { ...page, items }
    },
    retry: false,
    placeholderData: keepPreviousData,
    // Ohne Git keine Abfrage, die erwartbar mit 409 scheitert; pro Turn erst ab dem ersten Turn.
    enabled: scope === 'turn' ? turns.length > 0 : info.data?.git_repo === true,
  })
  const noGit = info.data?.git_repo === false || (q.error instanceof BetonError && q.error.code === 'not_a_git_repo')

  // Ohne Git-Repository gleich die Sicht zeigen, die funktioniert (pro Turn).
  useEffect(() => {
    if (noGit && !chosen) setScope('turn')
  }, [noGit, chosen])

  const pick = (s: ChangeScope) => {
    setChosen(true)
    setScope(s)
  }
  const files = q.data && q.data.scope === scope ? q.data.items : []
  const add = files.reduce((s, f) => s + (f.additions ?? 0), 0)
  const del = files.reduce((s, f) => s + (f.deletions ?? 0), 0)
  const baseName = worktree?.base.replace(/^origin\//, '')
  const onTopFile = useCallback((p: string) => setActiveFile(p), [])

  let body
  if (scope !== 'turn' && noGit) {
    body = (
      <div className="flex flex-col gap-3 p-3">
        <Callout>
          <p className="font-semibold">{scope === 'branch' ? 'Branch-Sicht' : 'Sicht „Nicht committet“'} nicht verfügbar</p>
          <p className="mt-0.5 text-muted-foreground">
            Der Workspace dieser Session ist kein Git-Repository. beton zeigt dir die Änderungen pro Turn aus eigenen Schnappschüssen. Mit{' '}
            <code className="font-mono text-[12px]">git init</code> stehen alle drei Sichten zur Verfügung.
          </p>
        </Callout>
      </div>
    )
  } else if (scope === 'turn' && turns.length === 0) {
    body = <Empty title="Noch kein Turn" text="Änderungen pro Turn erscheinen hier, sobald der Agent einen Turn bearbeitet hat." />
  } else if (q.isError) {
    body = (
      <div className="p-3">
        <Callout tone="deny">
          <p className="font-semibold">Änderungen nicht ladbar</p>
          <p className="mt-0.5 text-muted-foreground">{q.error instanceof Error ? q.error.message : ''}</p>
        </Callout>
      </div>
    )
  } else if (q.isPending || (scope !== 'turn' && info.isPending)) {
    body = <p className="p-3 text-[12px] text-muted-foreground">Lädt …</p>
  } else if (files.length === 0) {
    body =
      scope === 'uncommitted' ? (
        <Empty
          title="Keine nicht committeten Änderungen"
          text={
            <>
              Der Worktree entspricht <code className="font-mono text-[12px]">HEAD</code> ({short(q.data?.base_sha)}). Änderungen des Agents erscheinen hier, sobald er Dateien
              schreibt.
            </>
          }
        />
      ) : scope === 'branch' ? (
        <Empty title="Keine Änderungen auf diesem Branch" text={<>Der Branch enthält noch nichts gegenüber {baseName ?? 'der Base'}.</>} />
      ) : (
        <Empty title="Keine Änderungen in diesem Turn" text="Der Agent hat in diesem Turn keine Dateien geändert." />
      )
  } else {
    body = (
      <>
        <FileList
          files={files}
          active={activeFile}
          onPick={(p) => {
            setActiveFile(p)
            setJump((j) => ({ path: p, n: (j?.n ?? 0) + 1 }))
          }}
        />
        <DiffList
          sessionId={sessionId}
          scope={scope}
          turn={turn}
          files={files}
          version={version}
          anchor={initial.anchor && (initial.scope ?? scope) === scope ? initial.anchor : undefined}
          onAnchor={(file, line) => onAnchor({ scope, turn, file, line })}
          copyLink={(file, line) => copyLink({ scope, turn, file, line })}
          jump={jump}
          onTopFile={onTopFile}
        />
      </>
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="changes-panel">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
        <Segmented
          label="Sicht"
          value={scope}
          onChange={pick}
          items={[
            { id: 'uncommitted', label: noGit ? 'Nicht committet (nicht verfügbar)' : 'Nicht committet' },
            { id: 'branch', label: noGit ? 'Branch (nicht verfügbar)' : baseName ? `Branch gegenüber ${baseName}` : 'Branch' },
            { id: 'turn', label: 'Pro Turn' },
          ]}
        />
        {scope === 'turn' && turns.length > 0 && (
          <select
            aria-label="Turn"
            value={turn}
            onChange={(e) => setTurn(e.target.value)}
            className="h-7 w-56 max-w-full rounded-md border border-input bg-card px-2 text-xs"
          >
            {[...turns].reverse().map((t) => (
              <option key={t.id} value={t.id}>
                {turnLabel(t)}
              </option>
            ))}
          </select>
        )}
        {files.length > 0 && (
          <span className="ml-auto text-[12px] text-muted-foreground">
            {files.length} {files.length === 1 ? 'Datei' : 'Dateien'} · <AddDel add={add} del={del} />
          </span>
        )}
      </div>
      {scope === 'branch' && q.data?.scope === 'branch' && (
        <div className="border-b border-border px-3 py-1.5 font-mono text-[11px] text-muted-foreground">
          git diff {baseName ?? 'base'}...HEAD · Merge-Base {short(q.data.base_sha)}
        </div>
      )}
      {body}
    </div>
  )
}

function Empty({ title, text }: { title: string; text: ReactNode }) {
  return (
    <div className="flex flex-col items-center gap-1 px-6 py-16 text-center">
      <p className="text-[14px] font-medium">{title}</p>
      <p className="max-w-xs text-[13px] text-muted-foreground">{text}</p>
    </div>
  )
}
