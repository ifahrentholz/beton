import { createRootRoute, createRoute, createRouter, Outlet, useParams } from '@tanstack/react-router'
import { Menu } from 'lucide-react'
import { AppLayout } from '@/components/app-layout'
import { useLayout } from '@/components/layout-state'
import { SessionView } from '@/components/session-view'
import { useSessions } from '@/store/sessions'

function Root() {
  const params = useParams({ strict: false }) as { sessionId?: string }
  const unauthorized = useSessions((s) => s.unauthorized)
  if (unauthorized) return <LoginRequired />
  return (
    <AppLayout active={params.sessionId}>
      <Outlet />
    </AppLayout>
  )
}

function LoginRequired() {
  return (
    <div className="concrete-grain flex h-dvh items-center justify-center p-6">
      <div className="max-w-md">
        <h1 className="type-wide text-xl font-semibold">Nicht angemeldet</h1>
        <p className="mt-2 text-[14px]">
          Öffne die Web-UI aus dem Terminal mit <code className="rounded-sm bg-sunken px-1">beton open</code>. Der Befehl meldet diesen Browser mit
          einem Einmal-Link an.
        </p>
      </div>
    </div>
  )
}

function Home() {
  const toggleList = useLayout((s) => s.toggleList)
  return (
    <div className="concrete-grain flex flex-1 flex-col">
      <header className="flex h-12 items-center border-b border-border px-3 lg:hidden">
        <button onClick={toggleList} className="flex size-8 items-center justify-center rounded-md hover:bg-accent" aria-label="Sessions anzeigen">
          <Menu className="size-4" />
        </button>
      </header>
      <div className="flex flex-1 items-center justify-center p-6">
        <div className="max-w-md">
          <h1 className="type-wide text-xl font-semibold">beton</h1>
          <p className="mt-2 text-[14px] text-muted-foreground">
            Wähle links eine Session oder starte eine neue – hier oder im Terminal mit <code className="rounded-sm bg-sunken px-1">beton run</code>.
          </p>
        </div>
      </div>
    </div>
  )
}

function SessionRoute() {
  const { sessionId } = sessionRoute.useParams()
  return <SessionView key={sessionId} sessionId={sessionId} />
}

const rootRoute = createRootRoute({ component: Root })
const indexRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: Home })
const sessionRoute = createRoute({ getParentRoute: () => rootRoute, path: '/s/$sessionId', component: SessionRoute })

export const router = createRouter({ routeTree: rootRoute.addChildren([indexRoute, sessionRoute]) })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
