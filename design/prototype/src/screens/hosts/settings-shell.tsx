import type { ReactNode } from 'react'
import { SettingsFrame, type SettingsSectionId } from '@/app/settings-shell'
import { cn } from '@/lib/utils'

/**
 * Einstellungs-Seite mit Kopfzeile für die Gruppen hosts, plugins, install und cli:
 * Navigation und Sektionsliste aus `@/app/settings-shell`, Kopf und Inhalt rechts.
 */
export function SettingsShell({
  active,
  title,
  description,
  actions,
  children,
  connection = 'local',
  bodyClassName,
}: {
  active: SettingsSectionId
  title: string
  description?: ReactNode
  actions?: ReactNode
  children: ReactNode
  connection?: 'local' | 'server' | 'offline'
  bodyClassName?: string
}) {
  return (
    <SettingsFrame active={active} connection={connection}>
      <header className="flex shrink-0 items-start gap-4 border-b border-border px-6 py-4">
        <div className="min-w-0 flex-1">
          <h1 className="type-wide text-[17px] font-[650]">{title}</h1>
          {description && <p className="mt-0.5 max-w-3xl text-[13px] text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </header>
      <div className={cn('min-h-0 flex-1 overflow-y-auto px-6 py-5', bodyClassName)}>{children}</div>
    </SettingsFrame>
  )
}

/** Zustand als Form + Text (nie nur Farbe). */
export type MarkKind = 'ok' | 'busy' | 'idle' | 'off' | 'fail' | 'wait' | 'warn' | 'pending'

export function Mark({ kind, label, className }: { kind: MarkKind; label?: ReactNode; className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5 whitespace-nowrap', className)}>
      {kind === 'ok' && <span className="size-2 rounded-full bg-ok" />}
      {kind === 'busy' && <span className="size-2 animate-pulse rounded-full bg-ok" />}
      {kind === 'idle' && <span className="size-2 rounded-full border border-ok" />}
      {kind === 'off' && <span className="size-2 rounded-[1px] border border-muted-foreground" />}
      {kind === 'pending' && <span className="size-2 rounded-full border border-dashed border-muted-foreground" />}
      {kind === 'fail' && <span className="size-2 rotate-45 bg-deny" />}
      {kind === 'wait' && <span className="chamfer-sm size-2.5 bg-signal" />}
      {kind === 'warn' && (
        <svg viewBox="0 0 10 10" className="size-2.5" aria-hidden>
          <path d="M5 0.8 L9.4 9.2 L0.6 9.2 Z" fill="none" stroke="currentColor" strokeWidth="1.4" />
        </svg>
      )}
      {label !== undefined && <span className={cn('text-[12px]', kind === 'fail' && 'text-deny')}>{label}</span>}
    </span>
  )
}

/** Label-Chip; automatische `beton.`-Labels gestrichelt und gedämpft. */
export function LabelChip({ children }: { children: string }) {
  const auto = children.startsWith('beton.')
  return (
    <code
      className={cn(
        'inline-block rounded-sm border px-1 font-mono text-[11px] leading-[18px] whitespace-nowrap',
        auto ? 'border-dashed border-border text-muted-foreground' : 'border-border bg-card',
      )}
      title={auto ? 'Automatisch ermittelt' : 'Aus host.yaml'}
    >
      {children}
    </code>
  )
}

/** Abschnitt mit Linie statt Karte. */
export function Section({ title, aside, children, className }: { title: string; aside?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={cn('border-t border-border pt-3 pb-5 first:border-t-0 first:pt-0', className)}>
      <div className="mb-2 flex items-baseline gap-3">
        <h2 className="text-[13px] font-semibold">{title}</h2>
        {aside && <div className="ml-auto text-[12px] text-muted-foreground">{aside}</div>}
      </div>
      {children}
    </section>
  )
}

/** Schlüssel-Wert-Zeilen, dicht. */
export function KV({ rows, className }: { rows: [ReactNode, ReactNode][]; className?: string }) {
  return (
    <dl className={cn('grid grid-cols-[minmax(9rem,max-content)_1fr] gap-x-4 gap-y-1 text-[13px]', className)}>
      {rows.map(([k, v], i) => (
        <div key={i} className="contents">
          <dt className="text-muted-foreground">{k}</dt>
          <dd className="min-w-0">{v}</dd>
        </div>
      ))}
    </dl>
  )
}

/** Inline-Code. */
export function C({ children, className }: { children: ReactNode; className?: string }) {
  return <code className={cn('rounded-sm bg-muted px-1 font-mono text-[12px]', className)}>{children}</code>
}

/** Hinweis „lokal / ohne Netz“ bzw. „Netzwerkzugriff, opt-in“ – Prämissen sichtbar machen. */
export function Premise({ kind, children }: { kind: 'local' | 'network'; children: ReactNode }) {
  return (
    <p className="flex items-start gap-2 text-[12px] text-muted-foreground">
      <span
        aria-hidden
        className={cn('mt-[5px] size-1.5 shrink-0', kind === 'local' ? 'rounded-full bg-ok' : 'rounded-full border border-muted-foreground')}
      />
      <span>
        <span className="font-medium text-foreground">{kind === 'local' ? 'Lokal' : 'Netzwerkzugriff, nur auf Klick'}</span> · {children}
      </span>
    </p>
  )
}

/** Einfache Buttons im Stil der Referenz. */
export function Btn({
  children,
  variant = 'outline',
  className,
}: {
  children: ReactNode
  variant?: 'primary' | 'outline' | 'ghost' | 'signal' | 'danger'
  className?: string
}) {
  return (
    <button
      className={cn(
        'inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12.5px] whitespace-nowrap',
        variant === 'primary' && 'bg-foreground font-semibold text-background',
        variant === 'outline' && 'border border-border hover:bg-accent',
        variant === 'ghost' && 'text-muted-foreground hover:bg-accent hover:text-foreground',
        variant === 'signal' && 'chamfer-sm rounded-none bg-signal font-semibold text-signal-foreground',
        variant === 'danger' && 'border border-deny/50 text-deny hover:bg-deny-soft',
        className,
      )}
    >
      {children}
    </button>
  )
}
