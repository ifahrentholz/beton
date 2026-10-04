import { useLayoutEffect, useRef, type ReactNode } from 'react'
import { AppLayout } from './app-layout'
import { cn } from '@/lib/utils'

/**
 * Eine Einstellungs-Navigation für alle Einstellungs-Screens (#31).
 * Links die Bereichsliste, rechts der Inhalt. Jede Screen-Gruppe gestaltet ihren Inhaltsbereich selbst
 * (Kopf, Scrollbereich); Rahmen, Sektionsliste und Optik der Navigation kommen nur von hier.
 */

export type SettingsSectionId =
  // App
  | 'general'
  | 'appearance'
  | 'notifications'
  | 'voice'
  | 'browser'
  | 'privacy'
  | 'data'
  | 'updates'
  | 'flags'
  // Harnesses
  | 'setup'
  | 'catalog'
  | 'acp'
  | 'direct-api'
  | 'import'
  // Ausführung
  | 'hosts'
  | 'runners'
  | 'providers'
  | 'image'
  | 'login'
  | 'dispatch'
  | 'reaper'
  // Sicherheit
  | 'sandbox'
  | 'stages'
  | 'proxy'
  | 'egress-log'
  | 'credentials'
  | 'secrets'
  | 'git'
  | 'audit'
  | 'redaction'
  // Zugang
  | 'local'
  | 'devices'
  | 'tokens'
  | 'servers'
  // Erweitern
  | 'mcp'
  | 'git-hosts'
  | 'plugins'
  | 'policies-wasm'
  | 'api'
  // Team-Betrieb
  | 'team-server'
  | 'members'
  | 'hardening'
  // Diagnose
  | 'doctor'
  | 'connection'
  | 'logs'
  | 'storage'
  | 'metrics'
  | 'bundle'

const SETTINGS_SECTIONS: { title: string; items: { id: SettingsSectionId; label: string }[] }[] = [
  {
    title: 'App',
    items: [
      { id: 'general', label: 'Allgemein' },
      { id: 'appearance', label: 'Darstellung' },
      { id: 'notifications', label: 'Benachrichtigungen' },
      { id: 'voice', label: 'Spracheingabe' },
      { id: 'browser', label: 'Browser' },
      { id: 'privacy', label: 'Datenschutz' },
      { id: 'data', label: 'Sessions & Daten' },
      { id: 'updates', label: 'Updates' },
      { id: 'flags', label: 'Experimentelle Funktionen' },
    ],
  },
  {
    title: 'Harnesses',
    items: [
      { id: 'setup', label: 'Einrichtung & Anmeldung' },
      { id: 'catalog', label: 'Harness-Katalog' },
      { id: 'acp', label: 'ACP-Agents' },
      { id: 'direct-api', label: 'Direkt-API & Gateways' },
      { id: 'import', label: 'Transcripts importieren' },
    ],
  },
  {
    title: 'Ausführung',
    items: [
      { id: 'hosts', label: 'Hosts' },
      { id: 'runners', label: 'Runner' },
      { id: 'providers', label: 'Provider & Workspaces' },
      { id: 'image', label: 'Runner-Image' },
      { id: 'login', label: 'CLI-Login im Container' },
      { id: 'dispatch', label: 'Labels & Dispatch' },
      { id: 'reaper', label: 'Aufräumen' },
    ],
  },
  {
    title: 'Sicherheit',
    items: [
      { id: 'sandbox', label: 'Sandbox' },
      { id: 'stages', label: 'Stufen & Harnesses' },
      { id: 'proxy', label: 'Netzwerk & Egress-Proxy' },
      { id: 'egress-log', label: 'Egress-Log' },
      { id: 'credentials', label: 'Credentials für Agents' },
      { id: 'secrets', label: 'Secrets' },
      { id: 'git', label: 'Git-Verbindungen' },
      { id: 'audit', label: 'Audit-Log' },
      { id: 'redaction', label: 'Redaction' },
    ],
  },
  {
    title: 'Zugang',
    items: [
      { id: 'local', label: 'Lokaler Zugang' },
      { id: 'devices', label: 'Geräte & Hosts' },
      { id: 'tokens', label: 'Tokens' },
      { id: 'servers', label: 'Server-Profile' },
    ],
  },
  {
    title: 'Erweitern',
    items: [
      { id: 'mcp', label: 'MCP-Server' },
      { id: 'git-hosts', label: 'GitHub & GitLab' },
      { id: 'plugins', label: 'Plugins' },
      { id: 'policies-wasm', label: 'WASM-Regeln' },
      { id: 'api', label: 'API & SDKs' },
    ],
  },
  {
    title: 'Team-Betrieb (optional)',
    items: [
      { id: 'team-server', label: 'Team-Server & Sync' },
      { id: 'members', label: 'Mitglieder & Rollen' },
      { id: 'hardening', label: 'Server-Härtung' },
    ],
  },
  {
    title: 'Diagnose',
    items: [
      { id: 'doctor', label: 'Umgebung prüfen' },
      { id: 'connection', label: 'Verbindung' },
      { id: 'logs', label: 'Logs' },
      { id: 'storage', label: 'Speicher' },
      { id: 'metrics', label: 'Metriken & Traces' },
      { id: 'bundle', label: 'Diagnose-Bundle' },
    ],
  },
]

/** Bereichsliste links. Der aktive Bereich wird in den sichtbaren Teil gescrollt. */
export function SettingsNav({ active }: { active: SettingsSectionId }) {
  const navRef = useRef<HTMLElement>(null)
  useLayoutEffect(() => {
    const nav = navRef.current
    const item = nav?.querySelector<HTMLElement>('[aria-current="page"]')
    if (!nav || !item) return
    const top = item.offsetTop
    if (top + item.offsetHeight > nav.scrollTop + nav.clientHeight || top < nav.scrollTop) {
      nav.scrollTop = Math.max(0, top - nav.clientHeight / 3)
    }
  }, [active])
  // Die Liste bestimmt die Fensterhöhe nicht mit: sie füllt die Spalte und scrollt bei Bedarf.
  return (
    <div className="relative w-52 shrink-0 border-r border-border bg-sidebar">
      <nav ref={navRef} aria-label="Einstellungsbereiche" className="absolute inset-0 flex flex-col overflow-y-auto py-3">
        {SETTINGS_SECTIONS.map((g) => (
          <div key={g.title} className="mb-4">
            <div className="px-4 pb-1 text-[11px] font-medium text-muted-foreground">{g.title}</div>
            {g.items.map((it) => (
              <button
                key={it.id}
                aria-current={it.id === active ? 'page' : undefined}
                className={cn(
                  'block w-full border-l-2 px-4 py-1 text-left text-[13px]',
                  it.id === active ? 'border-foreground bg-accent font-medium' : 'border-transparent text-muted-foreground hover:bg-accent/60 hover:text-foreground',
                )}
              >
                {it.label}
              </button>
            ))}
          </div>
        ))}
        <div className="mt-auto px-4 text-[11px] leading-relaxed text-muted-foreground">
          <button className="mb-2 block text-[12px] text-foreground underline-offset-2 hover:underline">Einrichtung erneut starten</button>
          beton 0.9.2 · Kanal stable
          <br />
          Läuft lokal auf diesem Rechner
        </div>
      </nav>
    </div>
  )
}

/**
 * Rahmen eines Einstellungs-Screens: App-Navigation („Einstellungen“ aktiv), Bereichsliste, Inhaltsspalte.
 * `children` landen in einer Spalte (`flex-col`); den Scrollbereich legt der Screen selbst an.
 * `overlay` liegt über dem ganzen Fenster (Dialoge, die die App blockieren).
 */
export function SettingsFrame({
  active,
  children,
  connection = 'local',
  overlay,
}: {
  active: SettingsSectionId
  children: ReactNode
  connection?: 'local' | 'server' | 'offline'
  overlay?: ReactNode
}) {
  return (
    <div className="relative h-full">
      <AppLayout nav="settings" sessionList={false} connection={connection}>
        <div className="relative flex min-h-0 flex-1">
          <SettingsNav active={active} />
          <div className="relative flex min-w-0 flex-1 flex-col">{children}</div>
        </div>
      </AppLayout>
      {overlay}
    </div>
  )
}
