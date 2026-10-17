import { useEffect, type ReactNode } from 'react'
import { Link, useRouterState } from '@tanstack/react-router'
import { Gauge, Inbox, MessagesSquare, Settings, ShieldCheck } from 'lucide-react'
import { useT } from '@/lib/i18n'
import { cn } from '@/lib/utils'
import { useSessions } from '@/store/sessions'
import { CommandPalette } from './command-palette'
import { useGlobalCommands } from './global-commands'
import { useInboxCount } from './inbox'
import { useLayout } from './layout-state'
import { NewSessionDialog } from './new-session'
import { SessionList } from './session-list'
import { SessionSwitcher } from './session-switcher'
import { ShortcutsOverlay } from './shortcuts-overlay'
import { useUi } from './ui-state'

/**
 * Grundlayout (WEB-001): Navigationsspalte, Session-Liste, Hauptbereich. Unter 1024 px ist
 * die Liste ein Drawer; bei 375 px gibt es keinen horizontalen Scroll. Darüber liegen
 * Befehlspalette (UX-002), Session-Switcher (UX-003) und Tastenkürzel (UX-004).
 */
export function AppLayout({ active, children }: { active?: string | undefined; children: ReactNode }) {
  const listOpen = useLayout((s) => s.listOpen)
  const closeList = useLayout((s) => s.closeList)
  const offline = useSessions((s) => s.offline)
  const t = useT()
  const newOpen = useUi((s) => s.newSession)
  const setNewOpen = (newSession: boolean) => useUi.getState().set({ newSession })
  const path = useRouterState({ select: (s) => s.location.pathname })
  const inbox = path.startsWith('/inbox')
  // Verbrauch (USE-006) ohne Session-Liste, wie im Entwurf `usage-overview`.
  const usage = path.startsWith('/usage')
  const policies = path.startsWith('/policies')
  const settings = path.startsWith('/settings')
  const count = useInboxCount()
  useGlobalCommands({ inboxCount: count })
  // `?new=1` öffnet den Dialog „Neue Session“ (Menü und Tray der Desktop-App, DESK-004).
  useEffect(() => {
    const url = new URL(window.location.href)
    if (url.searchParams.get('new') !== '1') return
    setNewOpen(true)
    url.searchParams.delete('new')
    window.history.replaceState(window.history.state, '', `${url.pathname}${url.search}${url.hash}`)
  }, [])
  return (
    <div className="flex h-dvh min-h-0 w-full overflow-hidden bg-background text-foreground">
      <div className="hidden w-12 shrink-0 flex-col items-center gap-1 border-r border-border bg-sidebar py-2 sm:flex">
        <Link
          to="/"
          title={t('nav.sessions')}
          aria-label={t('nav.sessions')}
          aria-current={!inbox && !usage && !policies && !settings ? 'page' : undefined}
          className={cn('flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', !inbox && !usage && !policies && !settings && 'bg-accent text-foreground')}
        >
          <MessagesSquare className="size-[18px]" />
        </Link>
        <Link
          to="/inbox"
          title={t('nav.inbox')}
          aria-label={count ? t('nav.inboxOpen', { count }) : t('nav.inbox')}
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
          title={t('nav.usage')}
          aria-label={t('nav.usage')}
          aria-current={usage ? 'page' : undefined}
          className={cn('flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', usage && 'bg-accent text-foreground')}
          data-testid="nav-usage"
        >
          <Gauge className="size-[18px]" />
        </Link>
        <Link
          to="/policies/shadow"
          title={t('nav.policies')}
          aria-label={t('nav.policies')}
          aria-current={policies ? 'page' : undefined}
          className={cn('flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', policies && 'bg-accent text-foreground')}
          data-testid="nav-policies"
        >
          <ShieldCheck className="size-[18px]" />
        </Link>
        <Link
          to="/settings/appearance"
          title={t('nav.settings')}
          aria-label={t('nav.settings')}
          aria-current={settings ? 'page' : undefined}
          className={cn('flex size-9 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground', settings && 'bg-accent text-foreground')}
          data-testid="nav-settings"
        >
          <Settings className="size-[18px]" />
        </Link>
        <div className="mt-auto flex flex-col items-center gap-2 pb-1">
          <span
            title={offline ? t('nav.offline') : t('nav.local')}
            className={cn('size-2 rounded-full', offline ? 'border border-muted-foreground' : 'bg-ok')}
          />
        </div>
      </div>
      {!usage && !settings && (
        <div className="hidden w-64 shrink-0 border-r border-border lg:block">
          <SessionList active={active} onNew={() => setNewOpen(true)} />
        </div>
      )}
      {listOpen && (
        <div className="fixed inset-0 z-40 lg:hidden" role="dialog" aria-label={t('nav.sessions')}>
          <button className="absolute inset-0 bg-foreground/30" aria-label={t('nav.closeList')} onClick={closeList} />
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
      <CommandPalette />
      <SessionSwitcher />
      <ShortcutsOverlay />
    </div>
  )
}
