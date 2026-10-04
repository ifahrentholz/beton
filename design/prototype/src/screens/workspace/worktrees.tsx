import { Check, FolderGit2, GitBranch, Trash2 } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { StatusMark } from '@/app/harness'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { F } from '@/proto/feature-marker'
import { worktrees } from './mock'
import { Btn, Callout, DialogPanel, LocalNote, Menu, MenuItem, MenuLabel, MenuSeparator, Overlay, PageHeader, Tag } from '@/app/kit/workspace'

function DeleteDirty() {
  return (
    <Overlay>
      <F id="SES-016">
        <DialogPanel
          waiting
          role="alertdialog"
          title="Session löschen: Worktree hat ungesicherte Änderungen"
          subtitle="„Rate-Limiter für die Login-API“ · beton/rate-limiter-7f3k"
          footer={
            <>
              <Btn variant="signal">WIP-Commit anlegen und löschen</Btn>
              <Btn variant="danger">Änderungen verwerfen</Btn>
              <Btn variant="ghost" className="ml-auto">
                Abbrechen
              </Btn>
            </>
          }
        >
          <p className="text-[13px]">In diesem Worktree sind 3 Dateien nicht committet. Ohne deine Entscheidung entfernt beton nichts.</p>
          <ul className="mt-2 flex flex-col gap-0.5 font-mono text-[12px]">
            <li>M src/routes/auth.ts</li>
            <li>A src/middleware/rate-limit.ts</li>
            <li>A src/middleware/rate-limit.spec.ts</li>
          </ul>
          <p className="mt-3 text-[12px] text-muted-foreground">
            Mit WIP-Commit bleibt der Branch erhalten, weil er dann eigene Commits hat. Verlauf und Anhänge der Session werden in jedem Fall gelöscht.
          </p>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

function DeleteUnpushed() {
  return (
    <Overlay>
      <F id="SES-016">
        <DialogPanel
          waiting
          role="alertdialog"
          title="Branch mit 2 ungepushten Commits"
          subtitle="„Checkout-Formular barrierefrei machen“ · beton/checkout-a11y-6q2a"
          footer={
            <>
              <Btn variant="signal">Branch behalten, Worktree entfernen</Btn>
              <Btn variant="danger">Branch auch löschen</Btn>
              <Btn variant="ghost" className="ml-auto">
                Abbrechen
              </Btn>
            </>
          }
        >
          <p className="text-[13px]">Diese Commits gibt es nur auf deinem Rechner:</p>
          <ul className="mt-2 flex flex-col gap-1 text-[12px]">
            <li>
              <code className="font-mono text-muted-foreground">e41b07c</code> Fehlermeldungen mit aria-describedby verknüpfen
            </li>
            <li>
              <code className="font-mono text-muted-foreground">0a9f3d2</code> Fokus nach Absenden auf erste Fehlermeldung setzen
            </li>
          </ul>
          <p className="mt-3 text-[12px] text-muted-foreground">
            beton löscht Branches nur mit <code className="font-mono">git branch -d</code>. „Branch auch löschen“ nutzt <code className="font-mono">-D</code> und ist nicht rückgängig zu machen.
          </p>
        </DialogPanel>
      </F>
    </Overlay>
  )
}

export function WorktreesScreen({ state }: { state: string }) {
  return (
    <div className="relative h-full">
      <AppLayout sessionList={false}>
        <PageHeader
          title="Worktrees · shop-frontend"
          subtitle="Jede Session arbeitet in einem eigenen Worktree. Archivieren behält ihn, Löschen räumt ihn auf, ohne Arbeit zu verlieren."
          actions={<LocalNote>~/.beton/worktrees · 1,6 GB</LocalNote>}
        />
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          <F id={['SES-015', 'SES-016']}>
            <table className="w-full text-left text-[13px]">
              <thead className="text-[12px] text-muted-foreground">
                <tr className="border-b border-border">
                  <th className="py-1.5 pr-3 font-medium">Branch und Session</th>
                  <th className="py-1.5 pr-3 font-medium">Base</th>
                  <th className="py-1.5 pr-3 font-medium">Zustand</th>
                  <th className="py-1.5 pr-3 text-right font-medium">Größe</th>
                  <th className="py-1.5" />
                </tr>
              </thead>
              <tbody>
                {worktrees.map((w) => (
                  <tr key={w.branch} className="border-b border-border align-top">
                    <td className="py-2 pr-3">
                      <div className="flex items-center gap-1.5 font-mono text-[12px]">
                        <GitBranch className="size-3.5 text-muted-foreground" />
                        {w.branch}
                      </div>
                      <div className="mt-0.5 flex items-center gap-1.5 text-[12px] text-muted-foreground">
                        <StatusMark status={w.status} /> {w.session}
                      </div>
                      <div className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground">{w.path}</div>
                    </td>
                    <td className="py-2 pr-3 font-mono text-[12px]">{w.base}</td>
                    <td className="py-2 pr-3">
                      <Tag tone={w.stateTone === 'ok' ? 'ok' : 'neutral'}>{w.state}</Tag>
                    </td>
                    <td className="py-2 pr-3 text-right text-[12px] tabular-nums">{w.size}</td>
                    <td className="py-2 text-right">
                      <Btn size="sm" variant="ghost">
                        <Trash2 className="size-3.5" /> Session löschen
                      </Btn>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            <div className="mt-8">
              <h3 className="text-[13px] font-semibold">Verwaiste Worktrees</h3>
              <p className="mt-0.5 text-[12px] text-muted-foreground">
                Verzeichnisse ohne zugehörige Session, gefunden beim täglichen Aufräumen (<code className="font-mono">git worktree prune</code>) heute um 09:00.
              </p>
              <div className="mt-2 flex items-center gap-3 border-y border-border py-2 text-[13px]">
                <FolderGit2 className="size-4 text-muted-foreground" />
                <span className="font-mono text-[12px]">~/.beton/worktrees/shop-frontend-3fa2c1d9/old-search-2b1x</span>
                <span className="text-[12px] text-muted-foreground">Session am 12.09. gelöscht · Branch gemergt</span>
                <span className="ml-auto text-[12px] tabular-nums">388 MB</span>
                <Btn size="sm">Entfernen</Btn>
              </div>
            </div>
          </F>
        </div>
      </AppLayout>
      {state === 'delete-dirty' && <DeleteDirty />}
      {state === 'delete-unpushed' && <DeleteUnpushed />}
    </div>
  )
}

function BranchMenu() {
  return (
    <Menu className="w-[420px]">
      <MenuLabel>Wo arbeitet die Session?</MenuLabel>
      <MenuItem active icon={<Check className="size-3.5" />} hint="Eigener Branch, isoliert von anderen Sessions. Standard im Project.">
        Neuer Worktree
      </MenuItem>
      <MenuItem icon={<span />} hint="~/code/shop-frontend, Branch main. Parallele Sessions können sich überschreiben.">
        Im Haupt-Checkout
      </MenuItem>
      <MenuSeparator />
      <MenuLabel>Base-Branch für den neuen Worktree</MenuLabel>
      <MenuItem active icon={<Check className="size-3.5" />} right="origin/HEAD">
        <span className="font-mono text-[12px]">main</span>
      </MenuItem>
      <MenuItem icon={<span />} right="vor 2 Tagen">
        <span className="font-mono text-[12px]">develop</span>
      </MenuItem>
      <MenuItem icon={<span />} right="vor 3 Wochen">
        <span className="font-mono text-[12px]">release/2.3</span>
      </MenuItem>
      <div className="px-3 pt-1 pb-1.5 text-[11px] text-muted-foreground">Vorher wird origin/main geholt (Netzwerk, abschaltbar).</div>
    </Menu>
  )
}

export function NewSessionScreen({ state }: { state: string }) {
  const err = state === 'base-error'
  return (
    <div className="relative h-full">
      <AppLayout activeSession="new">
        <SessionHeader title="Neue Session" harness="claude" status="idle" />
        <div className="concrete-grain flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
          <p className="type-wide text-xl font-[700]">Woran soll der Agent arbeiten?</p>
          <F id="SES-013" className="max-w-md text-sm text-muted-foreground">
            Vorgaben aus dem Project <span className="font-medium text-foreground">shop-frontend</span>: Claude Code mit claude-opus-5-5, Effort hoch, fragt bei
            Schreibzugriff, eigener Worktree.
          </F>
        </div>
        <div className="relative">
          {state === 'branch-picker' && (
            <div className="absolute bottom-[calc(100%-6px)] left-4 z-30">
              <BranchMenu />
            </div>
          )}
          <F id="SES-015" className="flex flex-wrap items-center gap-2 px-4 pt-2 text-[12px]">
            <span className="text-muted-foreground">Project</span>
            <Tag>shop-frontend</Tag>
            <span className="ml-2 text-muted-foreground">Arbeitet in</span>
            <button className={err ? 'inline-flex items-center gap-1 rounded-md border border-deny/50 px-1.5 py-0.5' : 'inline-flex items-center gap-1 rounded-md border border-border px-1.5 py-0.5 hover:bg-accent'}>
              <GitBranch className="size-3" />
              neuer Worktree von <span className="font-mono">{err ? 'release/2.4' : 'main'}</span> ▾
            </button>
            <span className="font-mono text-[11px] text-muted-foreground">beton/&lt;titel&gt;-&lt;id&gt;</span>
          </F>
          {err && (
            <div className="px-4 pt-2">
              <Callout tone="deny">
                <p className="font-semibold">Worktree nicht angelegt: Base-Branch release/2.4 gibt es nicht.</p>
                <p className="mt-0.5">
                  Verfügbar sind <code className="font-mono text-[12px]">main</code>, <code className="font-mono text-[12px]">develop</code> und{' '}
                  <code className="font-mono text-[12px]">release/2.3</code>. Die Session startet erst, wenn du eine Base wählst oder die Vorgabe im Project korrigierst.
                </p>
              </Callout>
            </div>
          )}
          <Composer harness="claude" draft={err ? 'Hotfix: Gutscheincodes mit Leerzeichen am Ende akzeptieren' : undefined} />
        </div>
      </AppLayout>
    </div>
  )
}
