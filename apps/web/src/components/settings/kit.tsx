import type { ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { Check } from 'lucide-react'
import { cn } from '@/lib/utils'

/**
 * Bausteine der Einstellungsseiten wie im Entwurf (`design/prototype`, `app/kit/settings` und
 * `app/settings-shell`). Die Bereichsliste zeigt nur Bereiche, die es schon gibt.
 */

const SECTIONS: { title: string; items: { to: string; label: string }[] }[] = [
  { title: 'App', items: [{ to: '/settings/notifications', label: 'Benachrichtigungen' }] },
]

export function SettingsShell({ active, children }: { active: string; children: ReactNode }) {
  return (
    <div className="relative flex min-h-0 flex-1">
      <div className="relative hidden w-52 shrink-0 border-r border-border bg-sidebar md:block">
        <nav aria-label="Einstellungsbereiche" className="absolute inset-0 flex flex-col overflow-y-auto py-3">
          {SECTIONS.map((g) => (
            <div key={g.title} className="mb-4">
              <div className="px-4 pb-1 text-[11px] font-medium text-muted-foreground">{g.title}</div>
              {g.items.map((it) => (
                <Link
                  key={it.to}
                  to={it.to}
                  aria-current={it.to === active ? 'page' : undefined}
                  className={cn(
                    'block w-full border-l-2 px-4 py-1 text-left text-[13px]',
                    it.to === active ? 'border-foreground bg-accent font-medium' : 'border-transparent text-muted-foreground hover:bg-accent/60 hover:text-foreground',
                  )}
                >
                  {it.label}
                </Link>
              ))}
            </div>
          ))}
          <div className="mt-auto px-4 text-[11px] leading-relaxed text-muted-foreground">Läuft lokal auf diesem Rechner</div>
        </nav>
      </div>
      <div className="relative min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="max-w-3xl px-4 py-6 sm:px-8">{children}</div>
      </div>
    </div>
  )
}

export function PageHeader({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="mb-5 flex items-start gap-6 border-b border-border pb-4">
      <div className="min-w-0 flex-1">
        <h1 className="type-wide text-xl font-[700]">{title}</h1>
        {children && <p className="mt-1 max-w-2xl text-[13px] text-muted-foreground">{children}</p>}
      </div>
    </div>
  )
}

export function SectionTitle({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="mt-6 mb-2 flex flex-wrap items-baseline gap-3">
      <h2 className="text-[14px] font-semibold">{children}</h2>
      {aside && <div className="ml-auto text-[12px] text-muted-foreground">{aside}</div>}
    </div>
  )
}

/** Eine Einstellungszeile: Beschriftung + Erklärung links, Bedienelement rechts. */
export function Row({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex items-start gap-6 border-b border-border py-3 last:border-b-0">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium">{label}</div>
        {hint && <div className="mt-0.5 text-[12px] text-muted-foreground">{hint}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  )
}

/** Hinweis „bleibt auf diesem Rechner“ bzw. „verlässt den Rechner“. */
export function NetNote({ kind, children }: { kind: 'local' | 'network'; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[12px] text-muted-foreground">
      {kind === 'local' ? <span aria-hidden className="size-2 rounded-full bg-ok" /> : <span aria-hidden className="size-2 rotate-45 border border-foreground" />}
      {children}
    </span>
  )
}

export function Checkbox({
  checked,
  disabled,
  onChange,
  label,
  title,
}: {
  checked: boolean
  disabled?: boolean
  onChange?: (checked: boolean) => void
  label: string
  title?: string
}) {
  return (
    <span className="relative inline-flex size-4 items-center justify-center align-middle">
      <input
        type="checkbox"
        aria-label={label}
        title={title}
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange?.(e.target.checked)}
        className="peer size-4 cursor-pointer appearance-none rounded-[4px] border border-input checked:border-primary checked:bg-primary disabled:cursor-not-allowed disabled:opacity-50"
      />
      <Check aria-hidden className="pointer-events-none absolute size-3.5 text-primary-foreground opacity-0 peer-checked:opacity-100" />
    </span>
  )
}

export function Switch({ checked, onChange, label }: { checked: boolean; onChange: (checked: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={cn('relative inline-flex h-[18.4px] w-8 shrink-0 items-center rounded-full border border-transparent transition-colors', checked ? 'bg-primary' : 'bg-input')}
    >
      <span className={cn('block size-4 rounded-full bg-background transition-transform', checked ? 'translate-x-[calc(100%-2px)]' : 'translate-x-0')} />
    </button>
  )
}
