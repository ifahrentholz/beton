import type { ReactNode } from 'react'
import { AppLayout } from '@/app/app-layout'
import { cn } from '@/lib/utils'

/**
 * Rahmen der Einstellungs- und Diagnose-Seiten (Paket D-6).
 * Links die Bereichsnavigation, rechts der Inhalt. Wird auch von `screens/diagnostics` genutzt.
 */

export type SettingsSection =
  | 'appearance'
  | 'notifications'
  | 'mcp'
  | 'privacy'
  | 'data'
  | 'flags'
  | 'doctor'
  | 'connection'
  | 'logs'
  | 'storage'
  | 'metrics'
  | 'bundle'

const SECTIONS: { title: string; items: { id: SettingsSection; label: string }[] }[] = [
  {
    title: 'Einstellungen',
    items: [
      { id: 'appearance', label: 'Darstellung' },
      { id: 'notifications', label: 'Benachrichtigungen' },
      { id: 'mcp', label: 'MCP-Server' },
      { id: 'privacy', label: 'Datenschutz' },
      { id: 'data', label: 'Sessions & Daten' },
      { id: 'flags', label: 'Experimentelle Funktionen' },
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

export function SettingsShell({
  section,
  children,
  connection,
  wide,
  overlay,
}: {
  /** Dialog über dem Inhalt (wird innerhalb des Fensters gezeigt, nicht als Portal). */
  overlay?: ReactNode
  section: SettingsSection
  children: ReactNode
  connection?: 'local' | 'server' | 'offline'
  /** Inhalt ohne Breitenbegrenzung (Tabellen, Log-Viewer). */
  wide?: boolean
}) {
  return (
    <AppLayout nav="settings" sessionList={false} connection={connection}>
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Einstellungsbereiche" className="flex w-52 shrink-0 flex-col border-r border-border bg-sidebar py-3">
          {SECTIONS.map((g) => (
            <div key={g.title} className="mb-4">
              <div className="px-4 pb-1 text-[11px] font-medium text-muted-foreground">{g.title}</div>
              {g.items.map((it) => (
                <button
                  key={it.id}
                  aria-current={it.id === section ? 'page' : undefined}
                  className={cn(
                    'block w-full border-l-2 px-4 py-1 text-left text-[13px]',
                    it.id === section ? 'border-foreground bg-accent font-medium' : 'border-transparent text-muted-foreground hover:bg-accent/60 hover:text-foreground',
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
        <div className="relative min-w-0 flex-1 overflow-y-auto">
          <div className={cn('px-8 py-6', !wide && 'max-w-3xl')}>{children}</div>
          {overlay}
        </div>
      </div>
    </AppLayout>
  )
}

export function PageHeader({ title, children, actions }: { title: string; children?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="mb-5 flex items-start gap-6 border-b border-border pb-4">
      <div className="min-w-0 flex-1">
        <h1 className="type-wide text-xl font-[700]">{title}</h1>
        {children && <p className="mt-1 max-w-2xl text-[13px] text-muted-foreground">{children}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  )
}

export function SectionTitle({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="mt-6 mb-2 flex items-baseline gap-3">
      <h2 className="text-[14px] font-semibold">{children}</h2>
      {aside && <div className="ml-auto text-[12px] text-muted-foreground">{aside}</div>}
    </div>
  )
}

/** Eine Einstellungszeile: Beschriftung + Erklärung links, Bedienelement rechts. */
export function Row({ label, hint, children, className }: { label: ReactNode; hint?: ReactNode; children?: ReactNode; className?: string }) {
  return (
    <div className={cn('flex items-start gap-6 border-b border-border py-3 last:border-b-0', className)}>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium">{label}</div>
        {hint && <div className="mt-0.5 text-[12px] text-muted-foreground">{hint}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  )
}

/** Segmentierte Auswahl (rein visuell). */
export function Segmented({ options, value, label }: { options: { id: string; label: ReactNode }[]; value: string; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex rounded-md border border-border bg-card p-0.5">
      {options.map((o) => (
        <button
          key={o.id}
          role="radio"
          aria-checked={o.id === value}
          className={cn(
            'rounded-[3px] px-2.5 py-1 text-[12px] whitespace-nowrap',
            o.id === value ? 'bg-foreground text-background' : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

/** Hinweis „bleibt auf diesem Rechner“ bzw. „verlässt den Rechner“. */
export function NetNote({ kind, children }: { kind: 'local' | 'network'; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
      {kind === 'local' ? (
        <span aria-hidden className="size-2 rounded-full bg-ok" />
      ) : (
        <span aria-hidden className="size-2 rotate-45 border border-foreground" />
      )}
      {children}
    </span>
  )
}

export function Btn({
  children,
  primary,
  danger,
  disabled,
  title,
  className,
}: {
  children: ReactNode
  /** Die eine Primäraktion des Screens (Schalungsgelb nur dort, wo der Mensch dran ist). */
  primary?: boolean
  danger?: boolean
  disabled?: boolean
  title?: string
  className?: string
}) {
  return (
    <button
      disabled={disabled}
      title={title}
      className={cn(
        'inline-flex h-8 items-center gap-1.5 rounded-md border px-3 text-[13px] whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50',
        primary && 'border-foreground bg-foreground font-semibold text-background',
        danger && 'border-deny/50 text-deny hover:bg-deny-soft',
        !primary && !danger && 'border-border hover:bg-accent',
        className,
      )}
    >
      {children}
    </button>
  )
}

/** Problem nach RFC 9457, wie die UI es zeigt: verständlich oben, technische Details aufklappbar. */
export function ProblemBox({
  title,
  detail,
  next,
  problem,
  open,
}: {
  open?: boolean
  title: string
  detail: ReactNode
  next?: ReactNode
  problem: { type: string; status: number; code: string; trace_id: string; instance?: string; errors?: { pointer: string; detail: string }[] }
}) {
  return (
    <div className="rounded-md border border-deny/40 bg-deny-soft p-3 text-[13px]">
      <div className="flex items-center gap-2 font-semibold">
        <span aria-hidden className="size-2 rotate-45 bg-deny" />
        {title}
      </div>
      <div className="mt-1">{detail}</div>
      {next && <div className="mt-1 text-muted-foreground">{next}</div>}
      <details className="mt-2" open={open}>
        <summary className="cursor-pointer text-[12px] text-muted-foreground hover:text-foreground">Technische Details (application/problem+json)</summary>
        <pre className="mt-1 overflow-x-auto rounded-sm bg-card px-2 py-1.5 font-mono text-[11.5px] leading-relaxed">
          {JSON.stringify({ type: problem.type, title, status: problem.status, code: problem.code, instance: problem.instance, trace_id: problem.trace_id, errors: problem.errors }, null, 2)}
        </pre>
      </details>
    </div>
  )
}

/** Modaler Dialog innerhalb des Prototyp-Fensters. */
export function Overlay({ title, children, footer, width = 'max-w-lg', waiting }: { title: string; children: ReactNode; footer?: ReactNode; width?: string; waiting?: boolean }) {
  return (
    <div className="absolute inset-0 z-20 flex items-start justify-center bg-foreground/25 px-6 pt-16" role="dialog" aria-modal aria-label={title}>
      <div className={cn('w-full border border-border bg-popover text-popover-foreground shadow-xl', width, waiting ? 'chamfer border-l-4 border-l-signal' : 'rounded-lg')}>
        <div className="border-b border-border px-5 py-3">
          <h2 className="text-[15px] font-semibold">{title}</h2>
        </div>
        <div className="max-h-[480px] overflow-y-auto px-5 py-4 text-[13px]">{children}</div>
        {footer && <div className="flex items-center gap-2 border-t border-border px-5 py-3">{footer}</div>}
      </div>
    </div>
  )
}
