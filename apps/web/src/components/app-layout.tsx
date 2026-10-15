import { useState, type ReactNode } from 'react'
import { Link, useRouterState } from '@tanstack/react-router'
import { Gauge, Inbox, MessagesSquare, ShieldCheck } from 'lucide-react'
import { cn } from '@/lib/utils'
import { useSessions } from '@/store/sessions'
import { useInboxCount } from './inbox'
import { useLayout } from './layout-state'
import { NewSessionDialog } from './new-session'
import { SessionList } from './session-list'

/**
 * Grundlayout (WEB-001): Navigationsspalte, Session-Liste, Hauptbereich. Unter 1024 px ist
 * die Liste ein Drawer; bei 375 px gibt es keinen horizontalen Scroll.
 */
export function AppLayout({ active, children }: { active?: string | undefined; children: ReactNode }) {
  const listOpen = useLayout((s) => s.listOpen)
  const closeList = useLayout((s) => s.closeList)
  const offline = useSessions((s) => s.offline)
  const [newOpen, setNewOpen] = useState(false)
  const path = useRouterState({ select: (s) => s.location.pathname })
  const inbox = path.startsWith('/inbox')
  // Verbrauch (USE-006) ohne Session-Liste, wie im Entwurf `usage-overview`.
  const usage = path.startsWith('/usage')
  const policies = path.startsWith('/policies')
  const count = useInboxCount()
  return (
    <div className="flex h-dvh min-h-0 w-full overflow-hidden bg-background text-foreground">
      <div className="hidden w-12 shrink-0 flex-col items-center gap-1 border-r border-border bg-sidebar py-2 sm:flex">
        <Link
          to="/"
          title="Sessions"
          aria-label="Sessions"
          aria-current={!inbox && !usage && !policies ? 'page' : undefined}
          className={cn('flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', !inbox && !usage && !policies && 'bg-accent text-foreground')}
        >
          <MessagesSquare className="size-[18px]" />
        </Link>
        <Link
          to="/inbox"
          title="Inbox"
          aria-label={count ? `Inbox, ${count} offen` : 'Inbox'}
          aria-current={inbox ? 'page' : undefined}
          className={cn('relative flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', inbox && 'bg-accent text-foreground')}
          data-testid="nav-inbox"
        >
          <Inbox className="size-[18px]" />
          {count > 0 && (
            <span className="chamfer-sm absolute -top-0.5 -right-0.5 min-w-4 bg-signal px-0.5 text-center text-[10px] leading-4 font-semibold text-signal-foreground" data-testid="inbox-count">
              {count}
            </span>
          )}
        </Link>
        <Link
          to="/usage"
          title="Verbrauch"
          aria-label="Verbrauch"
          aria-current={usage ? 'page' : undefined}
          className={cn('flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', usage && 'bg-accent text-foreground')}
          data-testid="nav-usage"
        >
          <Gauge className="size-[18px]" />
        </Link>
        <Link
          to="/policies/shadow"
          title="Policies: Shadow-Modus"
          aria-label="Policies: Shadow-Modus"
          aria-current={policies ? 'page' : undefined}
          className={cn('flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', policies && 'bg-accent text-foreground')}
          data-testid="nav-policies"
        >
          <ShieldCheck className="size-[18px]" />
        </Link>
        <div className="mt-auto flex flex-col items-center gap-2 pb-1">
          <span
            title={offline ? 'Daemon nicht erreichbar' : 'Lokaler Server auf diesem Rechner'}
            className={cn('size-2 rounded-full', offline ? 'border border-muted-foreground' : 'bg-ok')}
          />
        </div>
      </div>
      {!usage && (
        <div className="hidden w-64 shrink-0 border-r border-border lg:block">
          <SessionList active={active} onNew={() => setNewOpen(true)} />
        </div>
      )}
      {listOpen && (
        <div className="fixed inset-0 z-40 lg:hidden" role="dialog" aria-label="Sessions">
          <button className="absolute inset-0 bg-foreground/30" aria-label="Liste schließen" onClick={closeList} />
          <div className="absolute inset-y-0 left-0 w-[min(20rem,85vw)] border-r border-border shadow-lg">
            <SessionList
              active={active}
              onNew={() => {
                closeList()
                setNewOpen(true)
              }}
            />
          </div>
        </div>
      )}
      <main className="flex min-w-0 flex-1 flex-col">{children}</main>
      {newOpen && <NewSessionDialog onClose={() => setNewOpen(false)} />}
    </div>
  )
}
