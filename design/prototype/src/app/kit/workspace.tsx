import type { ReactNode } from 'react'
import { Globe, HardDrive } from 'lucide-react'
import { AppLayout, type NavItem } from '@/app/app-layout'
import { Composer, SessionHeader } from '@/app/session-chrome'
import { AgentMessage, ToolCall, UserMessage } from '@/app/stream'
import { cn } from '@/lib/utils'
import type { HarnessId, SessionStatus } from '@/mock/data'

/*
 * Hilfsbausteine der Gruppen Workspace, Zusammenarbeit und GitHub & GitLab.
 * Dialoge werden im Fensterrahmen gezeigt (nicht als Portal), damit der Prototyp sie im Frame darstellt.
 */

/** Abgedunkelter Hintergrund mit Dialog im Fenster. */
export function Overlay({ children, align = 'top' }: { children: ReactNode; align?: 'top' | 'center' }) {
  return (
    <div
      className={cn(
        'absolute inset-0 z-40 flex justify-center bg-foreground/25 px-6',
        align === 'top' ? 'items-start pt-16' : 'items-center',
      )}
    >
      {children}
    </div>
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
      className={cn(
        'flex max-h-full flex-col overflow-hidden border border-border bg-popover text-popover-foreground shadow-[0_24px_60px_-28px_rgb(0_0_0/0.55)]',
        waiting ? 'chamfer border-l-4 border-l-signal' : 'rounded-lg',
        width,
      )}
    >
      <div className="border-b border-border px-5 pt-4 pb-3">
        <h3 className="text-[15px] font-semibold">{title}</h3>
        {subtitle && <p className="mt-0.5 text-[13px] text-muted-foreground">{subtitle}</p>}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
      {footer && <div className="flex items-center gap-2 border-t border-border bg-card px-5 py-3">{footer}</div>}
    </div>
  )
}

type BtnVariant = 'primary' | 'outline' | 'ghost' | 'danger' | 'signal'

/** Knopf im Stil der Referenz-Screens. `signal` nur für die eine Aktion, auf die der Screen wartet. */
export function Btn({
  children,
  variant = 'outline',
  size = 'md',
  disabled,
  className,
  title,
}: {
  children: ReactNode
  variant?: BtnVariant
  size?: 'sm' | 'md'
  disabled?: boolean
  className?: string
  title?: string
}) {
  return (
    <button
      disabled={disabled}
      title={title}
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

/** Schwebendes Menü (Picker, @-Suche, Slash-Menü, Kontextmenü). */
export function Menu({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      role="listbox"
      className={cn(
        'z-30 overflow-hidden rounded-lg border border-border bg-popover py-1 text-popover-foreground shadow-[0_16px_40px_-20px_rgb(0_0_0/0.5)]',
        className,
      )}
    >
      {children}
    </div>
  )
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return <div className="px-3 pt-2 pb-1 text-[11px] font-medium text-muted-foreground">{children}</div>
}

export function MenuItem({
  icon,
  children,
  hint,
  active,
  disabled,
  right,
  danger,
}: {
  icon?: ReactNode
  children: ReactNode
  hint?: ReactNode
  active?: boolean
  disabled?: boolean
  right?: ReactNode
  danger?: boolean
}) {
  return (
    <div
      role="option"
      aria-selected={active}
      aria-disabled={disabled}
      className={cn(
        'mx-1 flex items-start gap-2 rounded-md px-2 py-1.5 text-[13px]',
        active && 'bg-accent',
        disabled && 'opacity-55',
        danger && 'text-deny',
      )}
    >
      {icon && <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center text-muted-foreground">{icon}</span>}
      <span className="min-w-0 flex-1">
        <span className="block truncate">{children}</span>
        {hint && <span className="block text-[11px] text-muted-foreground">{hint}</span>}
      </span>
      {right && <span className="shrink-0 text-[11px] text-muted-foreground">{right}</span>}
    </div>
  )
}

export function MenuSeparator() {
  return <div className="my-1 h-px bg-border" />
}

/** Avatar eines Menschen mit Initialen. Presence über Ring und Text, nicht nur Farbe. */
export function Avatar({
  name,
  size = 'md',
  ring,
  muted,
  title,
}: {
  name: string
  size?: 'sm' | 'md'
  ring?: boolean
  muted?: boolean
  title?: string
}) {
  const initials = name
    .split(' ')
    .map((p) => p[0])
    .join('')
    .slice(0, 2)
    .toUpperCase()
  return (
    <span
      title={title ?? name}
      className={cn(
        'inline-flex shrink-0 items-center justify-center rounded-full font-semibold',
        size === 'md' ? 'size-6 text-[10px]' : 'size-5 text-[9px]',
        muted ? 'border border-dashed border-muted-foreground text-muted-foreground' : 'bg-foreground text-background',
        ring && 'ring-2 ring-ok ring-offset-1 ring-offset-background',
      )}
    >
      {initials}
    </span>
  )
}

/** Kleines Etikett mit Rahmen. */
export function Tag({ children, tone = 'neutral', className }: { children: ReactNode; tone?: 'neutral' | 'ok' | 'deny' | 'muted' | 'signal'; className?: string }) {
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-1 rounded-sm border px-1.5 py-px text-[11px] leading-4 whitespace-nowrap',
        tone === 'neutral' && 'border-border text-foreground',
        tone === 'muted' && 'border-border text-muted-foreground',
        tone === 'ok' && 'border-ok/40 bg-ok-soft text-ok',
        tone === 'deny' && 'border-deny/40 bg-deny-soft text-deny',
        tone === 'signal' && 'chamfer-sm border-signal bg-signal-soft text-foreground',
        className,
      )}
    >
      {children}
    </span>
  )
}

/** Formularzeile mit Beschriftung links. */
export function Field({ label, hint, children, className }: { label: ReactNode; hint?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={cn('grid grid-cols-[160px_1fr] items-start gap-x-4 gap-y-1 py-2', className)}>
      <div className="pt-1.5 text-[13px] font-medium">{label}</div>
      <div className="min-w-0">
        {children}
        {hint && <p className="mt-1 text-[12px] text-muted-foreground">{hint}</p>}
      </div>
    </div>
  )
}

/** Statische Eingabe (der Prototyp tippt nicht). */
export function FakeInput({ value, placeholder, mono, className, focus }: { value?: string; placeholder?: string; mono?: boolean; className?: string; focus?: boolean }) {
  return (
    <div
      className={cn(
        'flex h-8 items-center rounded-md border border-input bg-card px-2.5 text-[13px]',
        mono && 'font-mono text-[12px]',
        !value && 'text-muted-foreground',
        focus && 'outline-2 outline-ring',
        className,
      )}
    >
      <span className="truncate">{value || placeholder}</span>
      {focus && <span className="ml-px inline-block h-4 w-px animate-pulse bg-foreground" />}
    </div>
  )
}

/** Auswahl mit Pfeil (statisch). */
export function FakeSelect({ value, className }: { value: ReactNode; className?: string }) {
  return (
    <div className={cn('flex h-8 items-center justify-between gap-2 rounded-md border border-input bg-card px-2.5 text-[13px]', className)}>
      <span className="min-w-0 truncate">{value}</span>
      <span aria-hidden className="text-muted-foreground">▾</span>
    </div>
  )
}

/** Segmentierte Umschaltung (z. B. Sichten). */
export function Segmented({ items, value, className }: { items: { id: string; label: ReactNode }[]; value: string; className?: string }) {
  return (
    <div role="tablist" className={cn('inline-flex rounded-md border border-border p-0.5 text-xs', className)}>
      {items.map((i) => (
        <span
          key={i.id}
          role="tab"
          aria-selected={i.id === value}
          className={cn('rounded-[3px] px-2 py-0.5 whitespace-nowrap', i.id === value ? 'bg-foreground text-background' : 'text-muted-foreground')}
        >
          {i.label}
        </span>
      ))}
    </div>
  )
}

/** Kennzeichnet einen optionalen Netzwerkzugriff (Prämisse: lokal-first). */
export function NetNote({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-[12px] text-muted-foreground', className)}>
      <Globe className="size-3.5 shrink-0" />
      <span>{children}</span>
    </span>
  )
}

/** Kennzeichnet, dass etwas den Rechner nicht verlässt. */
export function LocalNote({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-[12px] text-muted-foreground', className)}>
      <HardDrive className="size-3.5 shrink-0" />
      <span>{children}</span>
    </span>
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

/** Seitenkopf für Seiten ohne Session (Projects, Worktrees, Einstellungen). */
export function PageHeader({ title, subtitle, actions }: { title: ReactNode; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="flex shrink-0 items-end gap-4 border-b border-border px-6 pt-5 pb-4">
      <div className="min-w-0">
        <h2 className="type-wide text-lg font-[700]">{title}</h2>
        {subtitle && <p className="mt-0.5 text-[13px] text-muted-foreground">{subtitle}</p>}
      </div>
      {actions && <div className="ml-auto flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  )
}

/**
 * Session-Ansicht mit Kopf, Verlauf, Eingabe und optional Rail und Dialog.
 * Für Screens, die einen Teil der Session-Ansicht zeigen.
 */
export function SessionShell({
  title = 'Rate-Limiter für die Login-API',
  harness = 'claude',
  model,
  status = 'idle',
  branch = 'beton/rate-limiter-7f3k',
  headerExtra,
  header,
  rail,
  composer,
  overlay,
  sessionList,
  activeSession = 'ses_7f3k',
  nav = 'sessions',
  connection,
  children,
  streamClassName,
}: {
  title?: string
  harness?: HarnessId
  model?: string
  status?: SessionStatus
  branch?: string
  headerExtra?: ReactNode
  /** Ersetzt den Standard-Kopf vollständig. */
  header?: ReactNode
  rail?: ReactNode
  composer?: ReactNode
  overlay?: ReactNode
  sessionList?: boolean
  activeSession?: string
  nav?: NavItem
  connection?: 'local' | 'server' | 'offline'
  children: ReactNode
  streamClassName?: string
}) {
  return (
    <div className="relative h-full">
      <AppLayout nav={nav} sessionList={sessionList} activeSession={activeSession} rail={rail} connection={connection}>
        {header ?? <SessionHeader title={title} harness={harness} model={model} status={status} branch={branch} extra={headerExtra} />}
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className={cn('mx-auto flex max-w-3xl flex-col gap-4 px-6 py-6', streamClassName)}>{children}</div>
        </div>
        {composer === undefined ? <Composer harness={harness} running={status === 'running'} /> : composer}
      </AppLayout>
      {overlay}
    </div>
  )
}

/** Kurzer Verlauf der Beispiel-Session als Kontext neben Panels. */
export function ShortStream({ compact }: { compact?: boolean }) {
  return (
    <>
      <UserMessage>Die Login-Route braucht einen Rate-Limiter: höchstens 5 Versuche pro Minute und IP. Bitte mit Tests.</UserMessage>
      {!compact && <ToolCall kind="read" name="Lesen" target="src/routes/auth.ts" duration="0,1 s" />}
      <ToolCall kind="edit" name="Bearbeiten" target="src/middleware/rate-limit.ts" duration="0,3 s" />
      <ToolCall kind="shell" name="Shell" target="pnpm vitest run auth" duration="4,8 s" />
      <AgentMessage harness="claude">
        <p>
          Die Login-Route ist jetzt auf 5 Versuche pro Minute und IP begrenzt. Danach antwortet sie mit{' '}
          <code className="rounded-sm bg-muted px-1 text-[13px]">429</code>. Alle 10 Tests laufen.
        </p>
      </AgentMessage>
    </>
  )
}
