import { createHashHistory, createRootRoute, createRoute, createRouter } from '@tanstack/react-router'
import { CatalogPage } from './proto/catalog-page'
import { OverviewPage } from './proto/overview-page'
import { ScreenPage } from './proto/screen-page'
import { DesignShell } from './proto/shell'

const rootRoute = createRootRoute({ component: DesignShell })

const overviewRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: OverviewPage })
const catalogRoute = createRoute({ getParentRoute: () => rootRoute, path: '/catalog', component: CatalogPage })
const screenRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/s/$screenId',
  validateSearch: (search: Record<string, unknown>): { state?: string } => ({
    state: typeof search.state === 'string' ? search.state : undefined,
  }),
  component: function ScreenRoute() {
    const { screenId } = screenRoute.useParams()
    const { state } = screenRoute.useSearch()
    return <ScreenPage key={screenId} screenId={screenId} state={state} />
  },
})

const routeTree = rootRoute.addChildren([overviewRoute, catalogRoute, screenRoute])

// Hash-Routing: der Prototyp läuft als statische Datei ohne Server-Fallback.
export const router = createRouter({ routeTree, history: createHashHistory() })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
