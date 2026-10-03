import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/**
 * Bausteine für Terminal-Sitzungen (Gruppen cli, hosts, plugins, install).
 *
 * Das Terminal ist immer dunkel. `Term` setzt deshalb die Klasse `dark`: Alle Tokens
 * (`text-ok`, `text-deny`, `text-signal`, `text-voice-*` …) nehmen ihre Nachtwerte an,
 * Hex-Werte sind nicht nötig.
 */
export function Term({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('dark break-words whitespace-pre-wrap text-foreground', className)}>{children}</div>
}

/** Terminal-Ausschnitt innerhalb eines Desktop-Screens. */
export function TermBlock({ children, title, className }: { children: ReactNode; title?: string; className?: string }) {
  return (
    <div className={cn('dark overflow-hidden rounded-md border border-border bg-sunken text-foreground', className)}>
      {title && <div className="border-b border-border px-3 py-1 font-mono text-[11px] text-muted-foreground">{title}</div>}
      <div className="overflow-x-auto p-3 font-mono text-[12px] leading-[1.55] break-words whitespace-pre-wrap">{children}</div>
    </div>
  )
}

export type Tone = 'plain' | 'dim' | 'ok' | 'deny' | 'bold' | 'signal' | 'claude' | 'codex' | 'acp' | 'direct' | 'italic'

const toneClass: Record<Tone, string> = {
  plain: '',
  dim: 'text-muted-foreground',
  ok: 'text-ok',
  deny: 'text-deny',
  bold: 'font-semibold',
  signal: 'text-signal',
  claude: 'text-voice-claude',
  codex: 'text-voice-codex',
  acp: 'text-voice-acp',
  direct: 'text-voice-direct',
  italic: 'italic text-muted-foreground',
}

/** Eine Ausgabezeile. */
export function L({ children, tone = 'plain', className }: { children?: ReactNode; tone?: Tone; className?: string }) {
  return <div className={cn('min-h-[1.55em]', toneClass[tone], className)}>{children}</div>
}

/** Eingefärbter Teil einer Zeile. */
export function S({ children, tone = 'plain', className }: { children: ReactNode; tone?: Tone; className?: string }) {
  return <span className={cn(toneClass[tone], className)}>{children}</span>
}

/** Shell-Prompt mit Befehl. */
export function Prompt({ cwd = '~/code/shop-frontend', children, host = 'ingo@mbp' }: { cwd?: string; children?: ReactNode; host?: string }) {
  return (
    <div className="min-h-[1.55em]">
      <span className="text-ok">{host}</span> <span className="text-voice-direct">{cwd}</span> <span className="text-muted-foreground">%</span>{' '}
      <span className="font-medium">{children}</span>
    </div>
  )
}

export function Cursor() {
  return <span aria-hidden className="inline-block h-[1.15em] w-[0.6em] translate-y-[3px] animate-pulse bg-foreground" />
}

export function Blank() {
  return <div className="h-[1.55em]" />
}

/** Kommentarzeile, wie man sie in Shell-Mitschnitten schreibt. */
export function Comment({ children }: { children: ReactNode }) {
  return <div className="min-h-[1.55em] text-muted-foreground italic"># {children}</div>
}

/** `echo $?` mit Exit-Code; ungleich 0 rot. */
export function ExitCode({ code, cwd }: { code: number; cwd?: string }) {
  return (
    <>
      <Prompt cwd={cwd}>echo $?</Prompt>
      <L tone={code === 0 ? 'plain' : 'deny'}>{code}</L>
    </>
  )
}

/**
 * Rückfrage, die auf dich wartet (Freigabe, Berechtigungen, Installation):
 * Schalungsgelb und Fase, weil hier der Mensch dran ist.
 */
export function Ask({
  title,
  children,
  question,
  choice = '[y/N]',
  answered,
}: {
  title: string
  children?: ReactNode
  question: string
  choice?: string
  /** Bereits beantwortet: dann neutral statt gelb. */
  answered?: string
}) {
  return (
    <div className={cn('chamfer my-1 border-l-2 py-1.5 pr-3 pl-3', answered ? 'border-border bg-card' : 'border-signal bg-signal-soft')}>
      <div className={cn('font-semibold', !answered && 'text-signal')}>{title}</div>
      {children}
      <div className="mt-0.5">
        {question}{' '}
        {answered ? (
          <span className="text-muted-foreground">
            {choice} {answered}
          </span>
        ) : (
          <>
            <span className="bg-signal px-1 font-semibold text-signal-foreground">{choice}</span> <Cursor />
          </>
        )}
      </div>
    </div>
  )
}

/** Kleiner Fortschrittsbalken in Textform. */
export function Bar({ pct, width = 24 }: { pct: number; width?: number }) {
  const full = Math.round((pct / 100) * width)
  return (
    <span>
      <span className="text-foreground">{'█'.repeat(full)}</span>
      <span className="text-muted-foreground">{'░'.repeat(width - full)}</span>
    </span>
  )
}
