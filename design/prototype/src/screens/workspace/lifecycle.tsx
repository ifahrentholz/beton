import { Archive, ArchiveRestore, Download, MoreHorizontal, Pencil, Pin, Plus, Sparkles, Trash2 } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { HarnessBadge, StatusMark } from '@/app/harness'
import { F } from '@/proto/feature-marker'
import { cn } from '@/lib/utils'
import { projects, sessions, type SessionSummary } from '@/mock/data'
import { Btn, DialogPanel, FakeInput, FakeSelect, Menu, MenuItem, MenuSeparator, Overlay, PageHeader, Segmented, Tag } from './parts'

const pinned = new Set(['ses_6p9z'])

function Row({ s, state, menu }: { s: SessionSummary; state: string; menu?: boolean }) {
  const renaming = state === 'rename' && s.id === 'ses_7f3k'
  return (
    <div className={cn('group relative grid grid-cols-[18px_1fr_150px_210px_90px_28px] items-center gap-3 border-b border-border px-2 py-2', menu && 'bg-accent')}>
      <StatusMark status={s.status} />
      <div className="min-w-0">
        {renaming ? (
          <F id="SES-010">
            <FakeInput value="Rate-Limiter für die Login-API" focus className="h-7" />
            <p className="mt-1 flex items-center gap-1 text-[11px] text-muted-foreground">
              <Sparkles className="size-3" /> Automatisch erzeugt nach dem ersten Turn mit claude-haiku-4-5 über deine Claude-Max-Anmeldung. Wenn du umbenennst, bleibt dein Titel.
            </p>
          </F>
        ) : (
          <div className="flex items-center gap-1.5">
            {s.unread && <span className="size-1.5 shrink-0 rounded-full bg-foreground" aria-label="ungelesen" />}
            <span className={cn('truncate text-[13px]', s.unread && 'font-semibold')}>{s.title}</span>
            {pinned.has(s.id) && <Pin className="size-3 shrink-0 text-muted-foreground" aria-label="angepinnt" />}
            {s.shared && <Tag tone="muted">geteilt</Tag>}
            {s.async && <Tag tone="muted">im Hintergrund</Tag>}
          </div>
        )}
      </div>
      <HarnessBadge id={s.harness} />
      <span className="truncate font-mono text-[11px] text-muted-foreground">{s.branch ?? 'kein Worktree'}</span>
      <span className="text-right text-[12px] text-muted-foreground">{s.updated}</span>
      <button aria-label="Aktionen" className="flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-accent">
        <MoreHorizontal className="size-4" />
      </button>
      {menu && (
        <div className="absolute top-9 right-2 z-30">
          <F id={['SES-001', 'SES-012']}>
            <Menu className="w-64">
              <MenuItem icon={<Pencil className="size-3.5" />}>Umbenennen</MenuItem>
              <MenuItem icon={<Pin className="size-3.5" />} active>
                Anpinnen
              </MenuItem>
              <MenuItem icon={<Download className="size-3.5" />}>Exportieren …</MenuItem>
              <MenuSeparator />
              <MenuItem icon={<Archive className="size-3.5" />} hint="Stoppt den laufenden Agent. Worktree bleibt erhalten.">
                Archivieren
              </MenuItem>
              <MenuItem icon={<Trash2 className="size-3.5" />} danger>
                Löschen …
              </MenuItem>
            </Menu>
          </F>
        </div>
      )}
    </div>
  )
}

function SearchResults() {
  const hits = [
    { s: sessions[0], where: 'Antwort von Claude Code · 14:06', pre: '… danach antwortet sie mit 429 und einem ', hit: 'Retry-After', post: '-Header.' },
    { s: sessions[0], where: 'Tool-Ausgabe · pnpm dev', pre: 'POST /auth/login 429 2ms ', hit: 'Retry-After', post: ': 12' },
    { s: sessions[1], where: 'Antwort von Codex · 14:12', pre: 'Der ', hit: 'Retry-After', post: '-Wert rundet auf, das passt zur Spezifikation.' },
  ]
  return (
    <F id="SES-012" className="flex flex-col">
      <div className="py-2 text-[12px] text-muted-foreground">3 Treffer in 2 Sessions · Titel und Nachrichten · 46 ms</div>
      {hits.map((h, i) => (
        <div key={i} className="grid grid-cols-[18px_1fr] gap-3 border-b border-border px-2 py-2">
          <StatusMark status={h.s.status} />
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-[13px]">
              <span className="font-medium">{h.s.title}</span>
              <span className="text-[12px] text-muted-foreground">{h.where}</span>
            </div>
            <div className="mt-0.5 truncate text-[13px] text-muted-foreground">
              {h.pre}
              <mark className="rounded-[2px] bg-foreground/15 font-medium text-foreground">{h.hit}</mark>
              {h.post}
            </div>
          </div>
        </div>
      ))}
    </F>
  )
}

function Archived() {
  const list = [
    { title: 'Login-Seite: Fehlermeldungen übersetzen', harness: 'claude' as const, when: 'archiviert am 21.09.' },
    { title: 'Bundle-Größe analysieren', harness: 'codex' as const, when: 'archiviert am 14.09.' },
  ]
  return (
    <F id="SES-001" className="flex flex-col">
      <p className="py-2 text-[12px] text-muted-foreground">Archivierte Sessions sind gestoppt und fehlen in der normalen Liste. Ihre Worktrees bleiben erhalten.</p>
      {list.map((a) => (
        <div key={a.title} className="flex items-center gap-3 border-b border-border px-2 py-2 text-[13px]">
          <StatusMark status="stopped" />
          <span className="flex-1 truncate">{a.title}</span>
          <HarnessBadge id={a.harness} />
          <span className="w-40 text-right text-[12px] text-muted-foreground">{a.when}</span>
          <Btn size="sm">
            <ArchiveRestore className="size-3.5" /> Wiederherstellen
          </Btn>
          <Btn size="sm" variant="ghost" className="text-deny">
            Löschen
          </Btn>
        </div>
      ))}
    </F>
  )
}

export function LifecycleScreen({ state }: { state: string }) {
  const filter = state === 'archived' ? 'archived' : 'own'
  return (
    <div className="relative h-full">
      <AppLayout sessionList={false}>
        <PageHeader
          title="Sessions"
          actions={
            <>
              <FakeInput value={state === 'search' ? 'Retry-After' : ''} placeholder="Titel und Nachrichten durchsuchen  ⌘K" focus={state === 'search'} className="w-80" />
              <Btn variant="primary">
                <Plus className="size-3.5" /> Neue Session
              </Btn>
            </>
          }
        />
        <div className="flex flex-wrap items-center gap-2 border-b border-border px-6 py-2">
          <F id="SES-012" as="span">
            <Segmented
              value={filter}
              items={[
                { id: 'own', label: 'Eigene' },
                { id: 'shared', label: 'Mit mir geteilt · 2' },
                { id: 'archived', label: 'Archiviert · 2' },
              ]}
            />
          </F>
          <FakeSelect value="Project: alle" className="h-7 w-36 text-xs" />
          <FakeSelect value="Harness: alle" className="h-7 w-36 text-xs" />
          <FakeSelect value="Status: alle" className="h-7 w-36 text-xs" />
          <span className="ml-auto text-[12px] text-muted-foreground">Gelesen-Status gilt auf all deinen Geräten</span>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6">
          {state === 'search' ? (
            <SearchResults />
          ) : state === 'archived' ? (
            <Archived />
          ) : (
            <F id={['SES-012', 'SES-001']}>
              <div className="mt-3 mb-1 text-[12px] font-medium text-muted-foreground">Angepinnt</div>
              {sessions
                .filter((s) => pinned.has(s.id))
                .map((s) => (
                  <Row key={s.id} s={s} state={state} />
                ))}
              {projects.map((p) => (
                <div key={p.id}>
                  <div className="mt-4 mb-1 flex items-baseline gap-2">
                    <span className="text-[12px] font-medium">{p.name}</span>
                    <span className="font-mono text-[10px] text-muted-foreground">{p.path}</span>
                  </div>
                  {sessions
                    .filter((s) => s.project === p.id && !pinned.has(s.id))
                    .map((s) => (
                      <Row key={s.id} s={s} state={state} menu={state === 'menu' && s.id === 'ses_6q2a'} />
                    ))}
                </div>
              ))}
            </F>
          )}
        </div>
      </AppLayout>
      {state === 'delete' && (
        <Overlay>
          <F id={['SES-001', 'SES-016']}>
            <DialogPanel
              role="alertdialog"
              title="„Flaky Test in payment_spec“ endgültig löschen?"
              footer={
                <>
                  <Btn variant="danger">
                    <Trash2 className="size-3.5" /> Endgültig löschen
                  </Btn>
                  <Btn variant="ghost">Stattdessen archivieren</Btn>
                  <Btn variant="ghost" className="ml-auto">
                    Abbrechen
                  </Btn>
                </>
              }
            >
              <ul className="flex list-disc flex-col gap-1 pl-5 text-[13px]">
                <li>Verlauf, Anhänge und Kommentare werden von diesem Rechner entfernt.</li>
                <li>Der Worktree ist sauber und wird entfernt.</li>
                <li>
                  Branch <code className="font-mono text-[12px]">beton/payment-flaky-6k8e</code> ist in main gemergt und wird gelöscht.
                </li>
              </ul>
              <p className="mt-3 text-[12px] text-muted-foreground">Das lässt sich nicht rückgängig machen. Archivieren behält alles und stoppt nur die Session.</p>
            </DialogPanel>
          </F>
        </Overlay>
      )}
    </div>
  )
}
