import type { ReactNode } from 'react'
import { AppLayout } from '@/app/app-layout'
import { cn } from '@/lib/utils'

export type HarnessSection = 'setup' | 'catalog' | 'acp' | 'providers' | 'import'

/**
 * Einstellungen mit dem Bereich „Harnesses“. Navigation im selben Stil wie `screens/settings/shell.tsx`
 * (Paket D-6); deren Einträge stehen hier nur als Kontext darunter.
 */
const SECTIONS: { title: string; items: { id: HarnessSection | string; label: string }[] }[] = [
  {
    title: 'Harnesses',
    items: [
      { id: 'setup', label: 'Einrichtung' },
      { id: 'catalog', label: 'Harness-Katalog' },
      { id: 'acp', label: 'ACP-Agents' },
      { id: 'providers', label: 'Direkt-API & Gateways' },
      { id: 'import', label: 'Transcripts importieren' },
    ],
  },
  {
    title: 'Einstellungen',
    items: [
      { id: 'appearance', label: 'Darstellung' },
      { id: 'notifications', label: 'Benachrichtigungen' },
      { id: 'mcp', label: 'MCP-Server' },
      { id: 'privacy', label: 'Datenschutz' },
      { id: 'data', label: 'Sessions & Daten' },
    ],
  },
  {
    title: 'Diagnose',
    items: [{ id: 'doctor', label: 'Umgebung prüfen' }],
  },
]

export function HarnessSettings({ active, children }: { active: HarnessSection; children: ReactNode }) {
  return (
    <AppLayout nav="settings" sessionList={false}>
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Einstellungsbereiche" className="flex w-52 shrink-0 flex-col border-r border-border bg-sidebar py-3">
          {SECTIONS.map((g) => (
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
          <div className="mt-auto px-4 text-[11px] leading-relaxed text-muted-foreground">Läuft lokal auf diesem Rechner</div>
        </nav>
        <div className="flex min-w-0 flex-1 flex-col overflow-hidden">{children}</div>
      </div>
    </AppLayout>
  )
}
