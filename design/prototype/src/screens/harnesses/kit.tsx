import type { ButtonHTMLAttributes, ReactNode } from 'react'
import { Check, CircleHelp, Info, TriangleAlert } from 'lucide-react'
import { cn } from '@/lib/utils'

/**
 * Kleine Bausteine für die Gruppen „Einrichtung & Harnesses“, „Agents“ und „Automationen“.
 * Bewusst lokal gehalten (keine Änderung an src/app); die anderen beiden Gruppen importieren von hier.
 */

type BtnVariant = 'primary' | 'signal' | 'outline' | 'ghost' | 'danger'

/** Buttons: `signal` nur für „du bist dran“ (Freigabe, die eine Primäraktion, die auf dich wartet). */
export function Btn({
  variant = 'outline',
  size = 'md',
  className,
  children,
  ...rest
}: { variant?: BtnVariant; size?: 'sm' | 'md'; children: ReactNode } & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...rest}
      className={cn(
        'inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50',
        size === 'md' ? 'h-8 px-3 text-[13px]' : 'h-7 px-2 text-xs',
        variant === 'primary' && 'rounded-md bg-foreground font-semibold text-background',
        variant === 'signal' && 'chamfer-sm bg-signal font-semibold text-signal-foreground',
        variant === 'outline' && 'rounded-md border border-border hover:bg-accent',
        variant === 'ghost' && 'rounded-md text-muted-foreground hover:bg-accent hover:text-foreground',
        variant === 'danger' && 'rounded-md border border-deny/50 text-deny hover:bg-deny-soft',
        className,
      )}
    >
      {children}
    </button>
  )
}

/** Kopf eines Hauptbereichs: Titel, Unterzeile, Aktionen rechts. */
export function PageHeader({ title, sub, actions, className }: { title: ReactNode; sub?: ReactNode; actions?: ReactNode; className?: string }) {
  return (
    <div className={cn('flex shrink-0 items-start gap-4 border-b border-border px-6 py-4', className)}>
      <div className="min-w-0 flex-1">
        <h2 className="type-wide text-lg font-[700]">{title}</h2>
        {sub && <p className="mt-0.5 text-[13px] text-muted-foreground">{sub}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  )
}

export function SectionTitle({ children, aside, className }: { children: ReactNode; aside?: ReactNode; className?: string }) {
  return (
    <div className={cn('flex items-baseline gap-3 pb-2', className)}>
      <h3 className="type-wide text-[13px] font-[650]">{children}</h3>
      {aside && <span className="ml-auto text-[11px] text-muted-foreground">{aside}</span>}
    </div>
  )
}

export function Code({ children, className }: { children: ReactNode; className?: string }) {
  return <code className={cn('rounded-sm bg-muted px-1 font-mono text-[12px]', className)}>{children}</code>
}

export type CodeLine = { text: string; mark?: 'error' | 'warn' | 'add'; note?: string }

/** Codeblock mit Zeilennummern; markierte Zeilen für Validierungsfehler und Warnungen. */
export function CodeBlock({ lines, start = 1, className, title }: { lines: (string | CodeLine)[]; start?: number; className?: string; title?: ReactNode }) {
  return (
    <div className={cn('overflow-hidden rounded-md border border-border bg-card font-mono text-[12px] leading-[1.6]', className)}>
      {title && <div className="border-b border-border bg-sunken px-3 py-1 font-sans text-[11px] text-muted-foreground">{title}</div>}
      <div className="overflow-x-auto py-1">
        {lines.map((raw, i) => {
          const l = typeof raw === 'string' ? { text: raw } : raw
          return (
            <div key={i}>
              <div
                className={cn(
                  'flex',
                  l.mark === 'error' && 'bg-deny-soft',
                  l.mark === 'warn' && 'bg-muted',
                  l.mark === 'add' && 'bg-ok-soft',
                )}
              >
                <span className="w-9 shrink-0 pr-2 text-right text-muted-foreground select-none">{start + i}</span>
                <span className="w-4 shrink-0 select-none">
                  {l.mark === 'error' && <span className="text-deny">✕</span>}
                  {l.mark === 'warn' && <span className="text-muted-foreground">!</span>}
                </span>
                <span className="pr-3 whitespace-pre">{l.text}</span>
              </div>
              {l.note && (
                <div className={cn('flex py-0.5 font-sans text-[12px]', l.mark === 'error' ? 'bg-deny-soft text-deny' : 'bg-muted text-muted-foreground')}>
                  <span className="w-13 shrink-0" />
                  <span>{l.note}</span>
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

/** Kleines Etikett mit Rahmen (Quelle, Modus, Label). */
export function Pill({ children, className, title }: { children: ReactNode; className?: string; title?: string }) {
  return (
    <span title={title} className={cn('inline-flex items-center gap-1 rounded-sm border border-border px-1.5 py-px text-[11px] leading-4 whitespace-nowrap text-muted-foreground', className)}>
      {children}
    </span>
  )
}

export type MarkKind = 'ok' | 'run' | 'wait' | 'deny' | 'off' | 'unknown' | 'queued' | 'skip'

/** Zustand als Form + Text, nie nur Farbe. */
export function Mark({ kind, label, className }: { kind: MarkKind; label?: ReactNode; className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5', className)}>
      <span className="flex size-3.5 shrink-0 items-center justify-center" aria-hidden>
        {kind === 'ok' && <Check className="size-3.5 text-ok" strokeWidth={2.5} />}
        {kind === 'run' && <span className="size-2 animate-pulse rounded-full bg-ok" />}
        {kind === 'wait' && <span className="chamfer-sm size-2.5 bg-signal" />}
        {kind === 'deny' && <span className="size-2 rotate-45 bg-deny" />}
        {kind === 'off' && <span className="size-2 rounded-full border border-muted-foreground" />}
        {kind === 'unknown' && <CircleHelp className="size-3.5 text-muted-foreground" />}
        {kind === 'queued' && <span className="size-2.5 rounded-full border border-dashed border-foreground/60" />}
        {kind === 'skip' && <span className="size-2.5 rounded-[1px] border border-dashed border-muted-foreground" />}
      </span>
      {label && <span className="text-[12px]">{label}</span>}
    </span>
  )
}

/** Hinweiszeile mit Icon. `deny` für Fehler mit Ursache und nächstem Schritt. */
export function Note({ tone = 'neutral', title, children, className, action }: { tone?: 'neutral' | 'deny' | 'warn'; title?: ReactNode; children?: ReactNode; className?: string; action?: ReactNode }) {
  const Icon = tone === 'neutral' ? Info : TriangleAlert
  return (
    <div
      className={cn(
        'flex gap-2.5 rounded-md border px-3 py-2.5 text-[13px]',
        tone === 'neutral' && 'border-border bg-card',
        tone === 'deny' && 'border-deny/40 bg-deny-soft',
        tone === 'warn' && 'border-foreground/25 bg-muted',
        className,
      )}
    >
      <Icon className={cn('mt-0.5 size-4 shrink-0', tone === 'deny' ? 'text-deny' : 'text-muted-foreground')} />
      <div className="min-w-0 flex-1">
        {title && <p className="font-semibold">{title}</p>}
        {children && <div className={cn('text-muted-foreground', title && 'mt-0.5')}>{children}</div>}
      </div>
      {action && <div className="flex shrink-0 items-start gap-2">{action}</div>}
    </div>
  )
}

/** Beschriftete Zeile in einer Definitionsliste. */
export function Row({ label, children, className }: { label: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={cn('flex gap-4 py-1.5 text-[13px]', className)}>
      <dt className="w-40 shrink-0 text-muted-foreground">{label}</dt>
      <dd className="min-w-0 flex-1">{children}</dd>
    </div>
  )
}

/** Fortschrittsbalken für Budgets: Text steht immer daneben, die Farbe ist nur Zusatz. */
export function Meter({ value, max, label, detail, tone }: { value: number; max: number; label: ReactNode; detail: ReactNode; tone?: 'deny' }) {
  const pct = Math.min(100, Math.round((value / max) * 100))
  return (
    <div className="text-[12px]">
      <div className="flex items-baseline gap-2">
        <span>{label}</span>
        <span className="ml-auto text-muted-foreground tabular-nums">{detail}</span>
      </div>
      <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
        <div className={cn('h-full', tone === 'deny' ? 'bg-deny' : 'bg-foreground/70')} style={{ width: `${pct}%` }} />
      </div>
    </div>
  )
}

/** Imitiertes Eingabefeld (Prototyp, nicht editierbar). */
export function FakeInput({ value, placeholder, mono, invalid, className }: { value?: ReactNode; placeholder?: string; mono?: boolean; invalid?: boolean; className?: string }) {
  return (
    <div
      className={cn(
        'flex h-8 items-center rounded-md border bg-card px-2.5 text-[13px]',
        invalid ? 'border-deny' : 'border-input',
        mono && 'font-mono text-[12px]',
        !value && 'text-muted-foreground',
        className,
      )}
    >
      {value ?? placeholder}
    </div>
  )
}

export function FieldLabel({ children, hint }: { children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="mb-1 flex items-baseline gap-2 text-[12px]">
      <span className="font-medium">{children}</span>
      {hint && <span className="text-muted-foreground">{hint}</span>}
    </div>
  )
}

/** Segmentierte Auswahl (nur Darstellung). */
export function Segmented({ items, active }: { items: string[]; active: string }) {
  return (
    <div className="inline-flex rounded-md border border-border p-0.5 text-xs">
      {items.map((i) => (
        <span key={i} className={cn('rounded-[3px] px-2 py-1', i === active ? 'bg-foreground text-background' : 'text-muted-foreground')}>
          {i}
        </span>
      ))}
    </div>
  )
}

/** Klassen für eine Seitenleiste, die über dem Inhalt liegt (Eltern-Element braucht `relative`). */
export const drawerClass =
  'absolute inset-y-0 right-0 z-10 flex flex-col border-l border-border bg-background shadow-[-16px_0_32px_-24px_rgb(0_0_0/0.45)]'
