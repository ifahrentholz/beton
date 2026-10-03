import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/*
 * Bausteine für die ratatui-TUI. Alles in Monospace; Rahmen wie ratatui-Blocks
 * (Linie mit Titel im oberen Rand). Farben kommen aus der Terminal-Palette:
 * die Wrapper-Klasse `dark` sorgt dafür, dass Tokens (Stimmfarben, signal, ok, deny)
 * ihre Nacht-Werte haben – passend zum dunklen Terminal.
 */

export function Tui({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn('dark -m-4 flex h-[calc(100%+2rem)] min-h-[600px] flex-col bg-sunken p-1.5 font-mono text-[12.5px] leading-[1.55] text-foreground', className)}>
      {children}
    </div>
  )
}

/** Ein Block mit Rahmen und Titel im oberen Rand. */
export function Block({
  title,
  right,
  focused,
  accent,
  className,
  bodyClassName,
  children,
}: {
  title?: ReactNode
  right?: ReactNode
  focused?: boolean
  /** `signal` = wartet auf dich. */
  accent?: 'signal'
  className?: string
  bodyClassName?: string
  children?: ReactNode
}) {
  return (
    <div
      className={cn(
        'relative min-h-0 border pt-2',
        accent === 'signal' ? 'border-2 border-signal' : focused ? 'border-foreground/80' : 'border-foreground/25',
        className,
      )}
    >
      {title && (
        <span
          className={cn(
            'absolute -top-[0.8em] left-2 bg-sunken px-1 whitespace-nowrap',
            accent === 'signal' ? 'font-semibold text-signal' : focused ? 'font-semibold' : 'text-muted-foreground',
          )}
        >
          {title}
        </span>
      )}
      {right && <span className="absolute -top-[0.8em] right-2 bg-sunken px-1 whitespace-nowrap text-muted-foreground">{right}</span>}
      <div className={cn('h-full overflow-hidden px-2 pb-1', bodyClassName)}>{children}</div>
    </div>
  )
}

/** Untere Tastenleiste: Taste invertiert, Bedeutung daneben. */
export function KeyBar({ keys, className }: { keys: [string, string][]; className?: string }) {
  return (
    <div className={cn('flex shrink-0 flex-wrap gap-x-3 gap-y-0.5 px-1 pt-1', className)}>
      {keys.map(([k, label]) => (
        <span key={k + label} className="whitespace-nowrap">
          <span className="bg-foreground/85 px-1 text-background">{k}</span>
          <span className="ml-1 text-muted-foreground">{label}</span>
        </span>
      ))}
    </div>
  )
}

/** Kopfzeile der TUI. */
export function TitleBar({ left, right }: { left: ReactNode; right?: ReactNode }) {
  return (
    <div className="mb-2.5 flex shrink-0 items-center gap-3 bg-foreground/10 px-2">
      <span className="font-semibold">beton tui</span>
      <span className="text-muted-foreground">{left}</span>
      {right && <span className="ml-auto text-muted-foreground">{right}</span>}
    </div>
  )
}

export type TuiStatus = 'waiting' | 'running' | 'idle' | 'failed' | 'stopped'

/** Status als Zeichen + Farbe (nie nur Farbe). */
export function Glyph({ status }: { status: TuiStatus }) {
  const map: Record<TuiStatus, [string, string, string]> = {
    waiting: ['◆', 'text-signal', 'wartet auf dich'],
    running: ['●', 'text-ok', 'läuft'],
    idle: ['○', 'text-muted-foreground', 'bereit'],
    failed: ['✕', 'text-deny', 'Fehler'],
    stopped: ['■', 'text-muted-foreground', 'gestoppt'],
  }
  const [g, c, label] = map[status]
  return (
    <span className={c} title={label}>
      {g}
    </span>
  )
}

export function SessionListBlock({ active, waitingOther }: { active: string; waitingOther?: boolean }) {
  const items: { id: string; title: string; status: TuiStatus; who: string }[] = [
    { id: 'ses_7f3k', title: 'Rate-Limiter Login', status: 'waiting', who: 'claude' },
    { id: 'ses_7f3m', title: 'Review: Rate-Limiter', status: waitingOther ? 'waiting' : 'running', who: 'codex' },
    { id: 'ses_6q2a', title: 'Checkout a11y', status: 'idle', who: 'claude' },
    { id: 'ses_6n1c', title: 'Dependency-Update', status: 'running', who: 'claude' },
    { id: 'ses_6k8e', title: 'Flaky payment_spec', status: 'failed', who: 'claude' },
    { id: 'ses_6m4d', title: 'Terraform-Plan', status: 'stopped', who: 'gemini' },
  ]
  return (
    <Block title="Sessions" className="w-[27ch] shrink-0">
      <div className="text-muted-foreground">shop-frontend</div>
      {items.map((s) => (
        <div key={s.id} className={cn('flex gap-1 whitespace-nowrap', s.id === active && 'bg-foreground/12 font-semibold')}>
          <span className="w-[1ch]">{s.id === active ? '>' : ' '}</span>
          <span className="min-w-0 flex-1 truncate">{s.title}</span>
          <Glyph status={s.status} />
        </div>
      ))}
      <div className="mt-2 text-muted-foreground">─────────────────────</div>
      <div className="text-muted-foreground">◆ wartet  ● läuft</div>
      <div className="text-muted-foreground">○ bereit  ✕ Fehler</div>
    </Block>
  )
}

/** Harness-Name in Stimmfarbe. */
export function Voice({ v, children }: { v: 'claude' | 'codex' | 'acp' | 'direct'; children: ReactNode }) {
  return (
    <span
      className={cn(
        'font-semibold',
        v === 'claude' && 'text-voice-claude',
        v === 'codex' && 'text-voice-codex',
        v === 'acp' && 'text-voice-acp',
        v === 'direct' && 'text-voice-direct',
      )}
    >
      {children}
    </span>
  )
}

export function Cursor() {
  return <span className="inline-block w-[1ch] animate-pulse bg-foreground">&nbsp;</span>
}
