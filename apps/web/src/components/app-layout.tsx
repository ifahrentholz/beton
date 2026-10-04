import { useState, type ReactNode } from 'react'
import { MessagesSquare } from 'lucide-react'
import { cn } from '@/lib/utils'
import { useSessions } from '@/store/sessions'
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
  return (
    <div className="flex h-dvh min-h-0 w-full overflow-hidden bg-background text-foreground">
      <div className="hidden w-12 shrink-0 flex-col items-center gap-1 border-r border-border bg-sidebar py-2 sm:flex">
        <span className="flex size-9 items-center justify-center rounded-md bg-accent text-foreground" title="Sessions" aria-current="page">
          <MessagesSquare className="size-[18px]" />
        </span>
        <div className="mt-auto flex flex-col items-center gap-2 pb-1">
          <span
            title={offline ? 'Daemon nicht erreichbar' : 'Lokaler Server auf diesem Rechner'}
            className={cn('size-2 rounded-full', offline ? 'border border-muted-foreground' : 'bg-ok')}
          />
        </div>
      </div>
      <div className="hidden w-64 shrink-0 border-r border-border lg:block">
        <SessionList active={active} onNew={() => setNewOpen(true)} />
      </div>
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
