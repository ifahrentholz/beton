import type { ButtonHTMLAttributes, ReactNode } from 'react'
import type { ChangeStatus } from '@beton/sdk'
import { cn } from '@/lib/utils'

/*
 * Bausteine der Workspace-Screens, übernommen aus dem Prototyp (`design/prototype/src/app/kit/workspace.tsx`,
 * `screens/workspace/bits.tsx`), hier bedienbar.
 */

type BtnVariant = 'primary' | 'outline' | 'ghost' | 'danger' | 'signal'

/** Knopf im Stil der Referenz-Screens. `signal` nur für die eine Aktion, auf die der Screen wartet. */
export function Btn({
  variant = 'outline',
  size = 'md',
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: BtnVariant; size?: 'sm' | 'md'; children: ReactNode }) {
  return (
    <button
      {...rest}
      className={cn(
        'inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap disabled:cursor-not-allowed disabled:opacity-50',
        size === 'md' ? 'h-8 px-3 text-[13px]' : 'h-7 px-2 text-xs',
        variant === 'primary' && 'rounded-md bg-foreground font-semibold text-background',
        variant === 'signal' && 'chamfer-sm bg-foreground font-semibold text-background',
        variant === 'outline' && 'rounded-md border border-border hover:bg-accent',
        variant === 'ghost' && 'rounded-md text-muted-foreground hover:bg-accent hover:text-foreground',
        variant === 'danger' && 'rounded-md border border-deny/50 font-medium text-deny hover:bg-deny-soft',
        className,
      )}
    >
      {children}
    </button>
  )
}

/** Segmentierte Umschaltung (Sichten). */
export function Segmented<T extends string>({
  items,
  value,
  onChange,
  label,
  className,
}: {
  items: { id: T; label: ReactNode; disabled?: boolean }[]
  value: T
  onChange: (id: T) => void
  label: string
  className?: string
}) {
  return (
    <div role="tablist" aria-label={label} className={cn('inline-flex rounded-md border border-border p-0.5 text-xs', className)}>
      {items.map((i) => (
        <button
          key={i.id}
          role="tab"
          aria-selected={i.id === value}
          disabled={i.disabled}
          onClick={() => onChange(i.id)}
          className={cn(
            'rounded-[3px] px-2 py-0.5 whitespace-nowrap disabled:cursor-not-allowed',
            i.id === value ? 'bg-foreground text-background' : 'text-muted-foreground hover:text-foreground disabled:hover:text-muted-foreground',
          )}
        >
          {i.label}
        </button>
      ))}
    </div>
  )
}

/** Hinweisblock mit Linie links. */
export function Callout({ children, tone = 'neutral', className }: { children: ReactNode; tone?: 'neutral' | 'deny' | 'ok' | 'signal'; className?: string }) {
  return (
    <div
      className={cn(
        'border-l-2 py-1.5 pr-2 pl-3 text-[13px]',
        tone === 'neutral' && 'border-border bg-card',
        tone === 'deny' && 'border-deny bg-deny-soft',
        tone === 'ok' && 'border-ok bg-ok-soft',
        tone === 'signal' && 'chamfer-sm border-signal bg-signal-soft',
        className,
      )}
    >
      {children}
    </div>
  )
}

const statusLetter: Record<ChangeStatus, 'A' | 'M' | 'D' | 'R'> = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R' }
const statusWord: Record<ChangeStatus, string> = { added: 'neu', modified: 'geändert', deleted: 'gelöscht', renamed: 'umbenannt' }

/** Git-Status als Buchstabe mit Text (nicht nur Farbe). */
export function FileStatus({ status }: { status: ChangeStatus }) {
  return (
    <span
      title={statusWord[status]}
      className={cn(
        'inline-flex size-4 shrink-0 items-center justify-center rounded-[2px] border font-mono text-[10px] font-semibold',
        status === 'added' && 'border-ok/50 text-ok',
        (status === 'modified' || status === 'renamed') && 'border-foreground/40 text-foreground',
        status === 'deleted' && 'border-deny/50 text-deny',
      )}
    >
      <span aria-hidden>{statusLetter[status]}</span>
      <span className="sr-only">{statusWord[status]}</span>
    </span>
  )
}

/** Buchstabe im Dateibaum (A grün, sonst Vordergrund). */
export function TreeStatus({ status }: { status: ChangeStatus }) {
  return (
    <span className={cn('ml-auto font-mono text-[10px] font-semibold', status === 'added' ? 'text-ok' : status === 'deleted' ? 'text-deny' : 'text-foreground')} title={statusWord[status]}>
      {statusLetter[status]}
    </span>
  )
}

export function AddDel({ add, del }: { add?: number | undefined; del?: number | undefined }) {
  if (add === undefined && del === undefined) return <span className="shrink-0 font-mono text-[11px] text-muted-foreground">binär</span>
  return (
    <span className="shrink-0 font-mono text-[11px] tabular-nums">
      <span className="text-ok">+{add ?? 0}</span> <span className="text-deny">−{del ?? 0}</span>
    </span>
  )
}

/** Dialogfläche. `waiting` markiert Dialoge, die auf eine Entscheidung warten (Fase + Schalungsgelb). */
export function DialogPanel({
  title,
  subtitle,
  children,
  footer,
  waiting,
  width = 'w-[540px]',
  role = 'dialog',
}: {
  title: ReactNode
  subtitle?: ReactNode
  children: ReactNode
  footer?: ReactNode
  waiting?: boolean
  width?: string
  role?: 'dialog' | 'alertdialog'
}) {
  return (
    <div
      role={role}
      aria-modal
      aria-label={typeof title === 'string' ? title : undefined}
      className={cn(
        'flex max-h-full max-w-full flex-col overflow-hidden border border-border bg-popover text-popover-foreground shadow-[0_24px_60px_-28px_rgb(0_0_0/0.55)]',
        waiting ? 'chamfer border-l-4 border-l-signal' : 'rounded-lg',
        width,
      )}
    >
      <div className="border-b border-border px-5 pt-4 pb-3">
        <h3 className="text-[15px] font-semibold">{title}</h3>
        {subtitle && <p className="mt-0.5 text-[13px] text-muted-foreground">{subtitle}</p>}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
      {footer && <div className="flex flex-wrap items-center gap-2 border-t border-border bg-card px-5 py-3">{footer}</div>}
    </div>
  )
}

/** Abgedunkelter Hintergrund mit Dialog oben im Fenster. */
export function Overlay({ children }: { children: ReactNode }) {
  return <div className="fixed inset-0 z-50 flex items-start justify-center bg-foreground/25 px-4 pt-16 sm:px-6">{children}</div>
}
