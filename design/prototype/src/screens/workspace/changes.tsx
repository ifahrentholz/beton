import { Link2, Paperclip } from 'lucide-react'
import { WorkspaceRail } from '@/app/session-chrome'
import { Diff } from '@/app/stream'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { rateLimiterDiff } from '@/mock/data'
import { AddDel, FileStatus } from './bits'
import { Btn, Callout, FakeSelect, Segmented, SessionShell, ShortStream } from '@/app/kit/workspace'
import { branchFiles, serverDiff, specDiff, turnFiles, uncommitted, type ChangedFile } from './mock'

function FileList({ files, active }: { files: ChangedFile[]; active: string }) {
  return (
    <div className="flex flex-col">
      {files.map((f) => (
        <div key={f.path} className={cn('flex items-center gap-2 border-l-2 px-3 py-1 text-[12px]', f.path === active ? 'border-foreground bg-accent' : 'border-transparent hover:bg-accent/60')}>
          <FileStatus status={f.status} />
          <span className="min-w-0 flex-1 truncate font-mono">{f.path}</span>
          <AddDel add={f.add} del={f.del} />
        </div>
      ))}
    </div>
  )
}

/** Diff mit Zeilenanker-Leiste: Hover auf einer Zeile bietet Link und „An Agent anhängen“. */
function AnchoredDiff() {
  return (
    <div className="relative">
      <Diff file="src/routes/auth.ts" lines={rateLimiterDiff} />
      <div className="absolute top-[92px] right-2 flex items-center gap-0.5 rounded-md border border-border bg-popover p-0.5 shadow-sm">
        <span className="px-1.5 font-mono text-[10px] text-muted-foreground">Z. 16 neu</span>
        <Btn size="sm" variant="ghost" title="Link auf diese Zeile kopieren">
          <Link2 className="size-3.5" />
        </Btn>
        <Btn size="sm" variant="ghost">
          <Paperclip className="size-3.5" /> An Agent anhängen
        </Btn>
      </div>
    </div>
  )
}

function ChangesBody({ state }: { state: string }) {
  if (state === 'empty') {
    return (
      <div className="flex flex-col items-center gap-1 px-6 py-16 text-center">
        <p className="text-[14px] font-medium">Keine nicht committeten Änderungen</p>
        <p className="max-w-xs text-[13px] text-muted-foreground">
          Der Worktree entspricht <code className="font-mono text-[12px]">HEAD</code> (a81f0c3). Änderungen des Agents erscheinen hier, sobald er Dateien schreibt.
        </p>
      </div>
    )
  }
  if (state === 'no-git') {
    return (
      <div className="flex flex-col gap-3 p-3">
        <Callout>
          <p className="font-semibold">Branch-Sicht nicht verfügbar</p>
          <p className="mt-0.5 text-muted-foreground">
            <code className="font-mono text-[12px]">~/scratch/csv-cleanup</code> ist kein Git-Repository. beton zeigt dir die Änderungen pro Turn aus eigenen
            Schnappschüssen. Mit <code className="font-mono text-[12px]">git init</code> stehen alle drei Sichten zur Verfügung.
          </p>
        </Callout>
      </div>
    )
  }
  const files = state === 'branch' ? branchFiles : state === 'turn' ? turnFiles : uncommitted
  return (
    <div className="flex flex-col">
      <FileList files={files} active={state === 'turn' ? 'src/middleware/rate-limit.spec.ts' : 'src/routes/auth.ts'} />
      <div className="flex flex-col gap-3 border-t border-border p-3">
        {state === 'turn' ? (
          <Diff file="src/middleware/rate-limit.spec.ts (neu)" lines={specDiff} />
        ) : (
          <>
            <AnchoredDiff />
            {state === 'branch' && <Diff file="src/server.ts · Commit 9d2e4b1 „trust proxy für req.ip setzen“" lines={serverDiff} />}
          </>
        )}
      </div>
    </div>
  )
}

export function ChangesScreen({ state }: { state: string }) {
  const files = state === 'branch' ? branchFiles : state === 'turn' ? turnFiles : state === 'empty' || state === 'no-git' ? [] : uncommitted
  const add = files.reduce((s, f) => s + f.add, 0)
  const del = files.reduce((s, f) => s + f.del, 0)
  const scope = state === 'branch' || state === 'no-git' ? 'branch' : state === 'turn' ? 'turn' : 'uncommitted'
  return (
    <SessionShell
      sessionList={false}
      title={state === 'no-git' ? 'CSV-Export bereinigen' : undefined}
      branch={state === 'no-git' ? '' : undefined}
      rail={
        <WorkspaceRail active="changes" width="w-[640px]">
          <F id="SES-018" className="flex min-h-full flex-col">
            <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
              <Segmented
                value={scope}
                items={[
                  { id: 'uncommitted', label: 'Nicht committet' },
                  { id: 'branch', label: state === 'no-git' ? 'Branch (nicht verfügbar)' : 'Branch gegenüber main' },
                  { id: 'turn', label: 'Pro Turn' },
                ]}
              />
              {state === 'turn' && <FakeSelect value="Turn 3 · 14:06 · „Bitte mit Tests“" className="h-7 w-56 text-xs" />}
              {files.length > 0 && (
                <span className="ml-auto text-[12px] text-muted-foreground">
                  {files.length} {files.length === 1 ? 'Datei' : 'Dateien'} · <AddDel add={add} del={del} />
                </span>
              )}
            </div>
            {state === 'branch' && (
              <div className="border-b border-border px-3 py-1.5 font-mono text-[11px] text-muted-foreground">
                git diff main...HEAD · Merge-Base 4c1e9a2 · 1 Commit + nicht committete Änderungen
              </div>
            )}
            <ChangesBody state={state} />
          </F>
        </WorkspaceRail>
      }
    >
      <ShortStream compact />
    </SessionShell>
  )
}
