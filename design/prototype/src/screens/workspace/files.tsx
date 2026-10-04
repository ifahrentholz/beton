import { ChevronDown, ChevronRight, Eye, File, Folder, Lock, MessageSquarePlus, Paperclip, Search, X } from 'lucide-react'
import { Composer, WorkspaceRail } from '@/app/session-chrome'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { Btn, Callout, DialogPanel, FakeInput, Overlay, Segmented, SessionShell, ShortStream } from '@/app/kit/workspace'
import { rateLimitSource, readmeSource, repo, tree } from './mock'
import { CodeLines } from './parts'

function FileTree({ state }: { state: string }) {
  return (
    <div className="flex w-56 shrink-0 flex-col border-r border-border">
      <div className="flex items-center gap-1 p-2">
        <FakeInput placeholder="Datei öffnen  ⌘P" className="h-7 flex-1 text-xs" />
        <button aria-label="Im Workspace suchen" className="flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent">
          <Search className="size-3.5" />
        </button>
      </div>
      <div className="px-3 pb-1 font-mono text-[10px] text-muted-foreground" title={repo.worktree}>
        Worktree · {repo.branch}
      </div>
      <div role="tree" className="min-h-0 flex-1 overflow-y-auto pb-2 text-[13px]">
        {tree.map((n) => {
          const active = state === 'preview' ? n.name === 'README.md' : state === 'outside' ? n.name === '.cache' : n.active
          return (
            <div
              key={n.name + n.depth}
              role="treeitem"
              aria-selected={active}
              title={n.outside ? `Symlink nach ${n.outside} – außerhalb des Workspace, nicht lesbar` : undefined}
              className={cn('flex items-center gap-1 py-[3px] pr-2', active ? 'bg-accent' : 'hover:bg-accent/60', n.outside && 'text-muted-foreground')}
              style={{ paddingLeft: 8 + n.depth * 14 }}
            >
              {n.dir ? (
                n.open ? <ChevronDown className="size-3 shrink-0 text-muted-foreground" /> : <ChevronRight className="size-3 shrink-0 text-muted-foreground" />
              ) : (
                <span className="w-3 shrink-0" />
              )}
              {n.dir ? <Folder className="size-3.5 shrink-0 text-muted-foreground" /> : <File className="size-3.5 shrink-0 text-muted-foreground" />}
              <span className="truncate">{n.name}</span>
              {n.outside && <Lock className="ml-auto size-3 shrink-0" aria-label="außerhalb des Workspace" />}
              {n.status && (
                <span className={cn('ml-auto font-mono text-[10px] font-semibold', n.status === 'A' ? 'text-ok' : 'text-foreground')} title={n.status === 'A' ? 'neu' : 'geändert'}>
                  {n.status}
                </span>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

function SearchPane() {
  const results = [
    {
      file: 'src/middleware/rate-limit.ts',
      hits: [{ n: 19, pre: "      res.set('", hit: 'Retry-After', post: "', String(Math.ceil(…)))" }],
    },
    {
      file: 'src/middleware/rate-limit.spec.ts',
      hits: [
        { n: 12, pre: "  it('antwortet beim sechsten Versuch mit 429 und ", hit: 'Retry-After', post: "', () => {" },
        { n: 21, pre: "    expect(res.headers['", hit: 'retry-after', post: "']).toBe('12')" },
      ],
    },
    { file: 'docs/api.md', hits: [{ n: 88, pre: 'Bei 429 nennt der Header `', hit: 'Retry-After', post: '` die Wartezeit in Sekunden.' }] },
  ]
  return (
    <div className="flex w-[300px] shrink-0 flex-col border-r border-border">
      <div className="flex flex-col gap-2 border-b border-border p-2">
        <FakeInput value="Retry-After" focus className="h-7 text-xs" />
        <div className="flex items-center gap-2">
          <Segmented
            value="content"
            items={[
              { id: 'name', label: 'Dateiname' },
              { id: 'content', label: 'Inhalt' },
            ]}
          />
          <span className="ml-auto text-[11px] text-muted-foreground">Groß/klein egal</span>
        </div>
      </div>
      <div className="px-3 py-1.5 text-[11px] text-muted-foreground">4 Treffer in 3 Dateien · 1.284 Dateien in 38 ms · .gitignore beachtet</div>
      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {results.map((r) => (
          <div key={r.file} className="mt-1">
            <div className="flex items-center gap-1.5 px-3 py-1 text-[12px] font-medium">
              <ChevronDown className="size-3 text-muted-foreground" />
              <span className="truncate font-mono">{r.file}</span>
              <span className="ml-auto text-[11px] font-normal text-muted-foreground">{r.hits.length}</span>
            </div>
            {r.hits.map((h, i) => (
              <div
                key={h.n}
                className={cn('flex gap-2 py-1 pr-2 pl-7 font-mono text-[11.5px]', r.file.endsWith('rate-limit.ts') && i === 0 ? 'bg-accent' : 'hover:bg-accent/60')}
              >
                <span className="w-6 shrink-0 text-right text-muted-foreground">{h.n}</span>
                <span className="truncate">
                  <span className="text-muted-foreground">{h.pre.trimStart()}</span>
                  <mark className="rounded-[2px] bg-foreground/15 text-foreground">{h.hit}</mark>
                  <span className="text-muted-foreground">{h.post}</span>
                </span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  )
}

function EditorTabs({ state }: { state: string }) {
  const tabs =
    state === 'preview'
      ? [{ name: 'README.md', active: true }, { name: 'rate-limit.ts' }]
      : [{ name: 'rate-limit.ts', active: true, dirty: state === 'conflict' }, { name: 'auth.ts' }, { name: 'README.md' }]
  return (
    <div role="tablist" className="flex h-9 shrink-0 items-end border-b border-border bg-sidebar">
      {tabs.map((t) => (
        <span
          key={t.name}
          role="tab"
          aria-selected={!!t.active}
          className={cn(
            '-mb-px flex h-8 items-center gap-2 border-r border-border px-3 text-[12px]',
            t.active ? 'border-b border-b-card bg-card font-medium' : 'text-muted-foreground',
          )}
        >
          {t.name}
          {'dirty' in t && t.dirty ? (
            <span className="size-2 rounded-full bg-foreground" title="Ungespeicherte Änderungen" />
          ) : (
            <X className="size-3 text-muted-foreground" aria-label={`${t.name} schließen`} />
          )}
        </span>
      ))}
    </div>
  )
}

function Editor({ state }: { state: string }) {
  if (state === 'outside') {
    return (
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-9 shrink-0 items-center border-b border-border bg-sidebar px-3 font-mono text-[12px] text-muted-foreground">.cache</div>
        <div className="p-4">
          <Callout tone="deny">
            <p className="font-semibold">Diese Datei liegt außerhalb des Workspace.</p>
            <p className="mt-1 text-muted-foreground">
              <code className="font-mono text-[12px]">.cache</code> ist ein Symlink nach <code className="font-mono text-[12px]">/Volumes/Scratch/shop-cache</code>. beton öffnet nur
              Dateien innerhalb des Worktrees dieser Session. Öffne den Ordner im Finder, wenn du ihn ansehen willst.
            </p>
          </Callout>
        </div>
      </div>
    )
  }

  const md = state === 'preview'
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <EditorTabs state={state} />
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border px-3 text-[11px] text-muted-foreground">
        <span className="truncate font-mono">{md ? 'README.md' : 'src/middleware/rate-limit.ts'}</span>
        <span className="ml-auto flex items-center gap-2">
          {md && (
            <span className="inline-flex items-center gap-1 rounded-sm bg-accent px-1.5 py-px text-foreground">
              <Eye className="size-3" /> Vorschau
            </span>
          )}
          <span>Suchen ⌘F</span>
          <span>·</span>
          {state === 'conflict' ? <span className="text-foreground">Ungespeichert · ⌘S speichert</span> : <span>Gespeichert</span>}
        </span>
      </div>
      {md ? (
        <div className="grid min-h-0 flex-1 grid-cols-2 overflow-hidden">
          <div className="overflow-auto border-r border-border bg-card py-2">
            <CodeLines lines={readmeSource} />
          </div>
          <article className="overflow-auto px-5 py-4 text-[13px] leading-relaxed">
            <h1 className="type-wide text-xl font-[700]">shop-frontend</h1>
            <p className="mt-2">
              Storefront und Checkout für den Shop. Express-Backend unter <code className="rounded-sm bg-muted px-1 font-mono text-[12px]">src/</code>.
            </p>
            <h2 className="mt-4 text-[15px] font-semibold">Entwicklung</h2>
            <pre className="mt-2 rounded-md bg-sunken p-2 font-mono text-[12px]">{`pnpm install
pnpm dev        # http://localhost:5173
pnpm vitest`}</pre>
            <h2 className="mt-4 text-[15px] font-semibold">Rate-Limits</h2>
            <ul className="mt-1 list-disc pl-5">
              <li>Login: 5 Versuche pro Minute und IP</li>
            </ul>
          </article>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto bg-card py-2">
          <CodeLines
            lines={state === 'conflict' ? withLocalEdit(rateLimitSource) : rateLimitSource}
            selected={state === 'editor' ? [13, 20] : undefined}
            marks={
              state === 'editor'
                ? {
                    20: (
                      <F id="WEB-009" className="inline-flex items-center gap-1 rounded-md border border-border bg-popover p-1 shadow-sm">
                        <span className="px-1.5 text-[11px] text-muted-foreground">Z. 13–20 markiert</span>
                        <Btn size="sm" variant="primary">
                          <Paperclip className="size-3.5" /> An Agent anhängen
                        </Btn>
                        <Btn size="sm" variant="ghost">
                          <MessageSquarePlus className="size-3.5" /> Kommentieren
                        </Btn>
                      </F>
                    ),
                  }
                : undefined
            }
          />
        </div>
      )}
      <div className="flex h-6 shrink-0 items-center gap-3 border-t border-border px-3 text-[11px] text-muted-foreground">
        <span>{md ? 'Markdown' : 'TypeScript'}</span>
        <span>UTF-8 · LF</span>
        <span className="ml-auto">{state === 'editor' ? 'Z. 13–20 · 8 Zeilen markiert' : 'Z. 19, Sp. 7'}</span>
      </div>
    </div>
  )
}

function withLocalEdit(lines: string[]) {
  const copy = [...lines]
  copy[12] = "    const id = opts.key === 'ip' ? (req.ip ?? 'unknown') : req.user?.id"
  return copy
}

function ConflictDialog() {
  return (
    <Overlay>
      <F id="WEB-009">
        <DialogPanel
          waiting
          role="alertdialog"
          title="Claude Code hat rate-limit.ts geändert"
          subtitle="Du hast in dieser Datei ungespeicherte Änderungen. beton überschreibt nichts, bis du dich entscheidest."
          footer={
            <>
              <Btn variant="signal">Vergleichen und zusammenführen</Btn>
              <Btn>Version des Agents laden</Btn>
              <Btn variant="ghost" className="ml-auto">
                Meine Version speichern
              </Btn>
            </>
          }
        >
          <div className="grid grid-cols-2 gap-3 text-[12px]">
            <div>
              <div className="mb-1 font-medium">Deine Änderung (ungespeichert)</div>
              <pre className="overflow-x-auto rounded-md border border-border bg-card p-2 font-mono text-[11.5px]">{`13  const id = opts.key === 'ip'
      ? (req.ip ?? 'unknown')
      : req.user?.id`}</pre>
            </div>
            <div>
              <div className="mb-1 font-medium">Änderung des Agents · vor 8 s</div>
              <pre className="overflow-x-auto rounded-md border border-border bg-card p-2 font-mono text-[11.5px]">{`13  const id = keyOf(req, opts.key)
14  if (!id) return next()`}</pre>
            </div>
          </div>
          <p className="mt-3 text-[12px] text-muted-foreground">
            „Meine Version speichern“ überschreibt die Änderung des Agents. Er erfährt davon im nächsten Turn.
          </p>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

export function FilesScreen({ state }: { state: string }) {
  const composer =
    state === 'attached' ? (
      <div>
        <F id="WEB-009" className="flex flex-wrap items-center gap-1.5 px-4 pt-2">
          <span className="inline-flex items-center gap-1.5 rounded-md border border-border bg-card py-0.5 pr-1 pl-2 text-[12px]">
            <Paperclip className="size-3 text-muted-foreground" />
            <span className="font-mono">src/middleware/rate-limit.ts</span>
            <span className="text-muted-foreground">Z. 13–20</span>
            <X className="size-3 text-muted-foreground" aria-label="Referenz entfernen" />
          </span>
          <span className="text-[11px] text-muted-foreground">Ausschnitt wird mitgeschickt</span>
        </F>
        <Composer harness="claude" draft="Was passiert hier, wenn req.ip leer ist? Bitte mit einem Test absichern." />
      </div>
    ) : undefined

  return (
    <SessionShell
      sessionList={false}
      composer={composer}
      overlay={state === 'conflict' ? <ConflictDialog /> : undefined}
      rail={
        <WorkspaceRail active="files" width="w-[780px]">
          <F id={['SES-017', 'WEB-009']} className="flex h-full min-h-0">
            {state === 'search' ? <SearchPane /> : <FileTree state={state} />}
            <Editor state={state === 'attached' ? 'editor-plain' : state} />
          </F>
        </WorkspaceRail>
      }
    >
      <ShortStream compact />
    </SessionShell>
  )
}
