import { createRootRoute, createRoute, createRouter, Outlet, useParams } from '@tanstack/react-router'
import { Menu } from 'lucide-react'
import { AppLayout } from '@/components/app-layout'
import { InboxView } from '@/components/inbox'
import { rich } from '@/components/kbd'
import { PolicyShadowView } from '@/components/policy-shadow'
import { useLayout } from '@/components/layout-state'
import { SessionView } from '@/components/session-view'
import { AppearanceSettingsView } from '@/components/settings/appearance'
import { NotificationSettingsView } from '@/components/settings/notifications'
import { PricingView } from '@/components/usage/pricing-view'
import { UsageView } from '@/components/usage/usage-view'
import { validateSessionSearch } from '@/lib/deeplink'
import { useT } from '@/lib/i18n'
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
  const t = useT()
  return (
    <div className="concrete-grain flex h-dvh items-center justify-center p-6">
      <div className="max-w-md">
        <h1 className="type-wide text-xl font-semibold">{t('login.title')}</h1>
        <p className="mt-2 text-[14px]">{rich(t('login.text'), { cmd: <code className="rounded-sm bg-sunken px-1">beton open</code> })}</p>
      </div>
    </div>
  )
}

function Home() {
  const toggleList = useLayout((s) => s.toggleList)
  const t = useT()
  return (
    <div className="concrete-grain flex flex-1 flex-col">
      <header className="flex h-12 items-center border-b border-border px-3 lg:hidden">
        <button onClick={toggleList} className="flex size-8 items-center justify-center rounded-md hover:bg-accent" aria-label={t('nav.showSessions')}>
          <Menu className="size-4" />
        </button>
      </header>
      <div className="flex flex-1 items-center justify-center p-6">
        <div className="max-w-md">
          <h1 className="type-wide text-xl font-semibold">beton</h1>
          <p className="mt-2 text-[14px] text-muted-foreground">{rich(t('home.text'), { cmd: <code className="rounded-sm bg-sunken px-1">beton run</code> })}</p>
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
const sessionRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/s/$sessionId',
  component: SessionRoute,
  // Teilbare Links auf Diff-Zeilen (WEB-011 AC2) und Sprungziele aus Deep-Links (DESK-006).
  validateSearch: validateSessionSearch,
})

// Inbox (UX-001).
const inboxRoute = createRoute({ getParentRoute: () => rootRoute, path: '/inbox', component: InboxView })

// Einstellungen: Darstellung (UX-005, WEB-016, WEB-017) und Benachrichtigungen (UX-006).
const appearanceRoute = createRoute({ getParentRoute: () => rootRoute, path: '/settings/appearance', component: AppearanceSettingsView })
const settingsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/settings/notifications', component: NotificationSettingsView })

// Shadow-Modus der Policies (POL-027 AC4).
const shadowRoute = createRoute({ getParentRoute: () => rootRoute, path: '/policies/shadow', component: PolicyShadowView })
// Verbrauch und Preise (USE-006).
const usageRoute = createRoute({ getParentRoute: () => rootRoute, path: '/usage', component: UsageView })
const pricingRoute = createRoute({ getParentRoute: () => rootRoute, path: '/usage/pricing', component: PricingView })

export const router = createRouter({ routeTree: rootRoute.addChildren([indexRoute, sessionRoute, inboxRoute, usageRoute, pricingRoute, shadowRoute, appearanceRoute, settingsRoute]) })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
