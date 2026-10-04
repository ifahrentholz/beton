import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/** Code mit Zeilennummern (Editor-ähnlich, ohne Monaco). */
export function CodeLines({
  lines,
  start = 1,
  selected,
  marks,
  className,
}: {
  lines: string[]
  start?: number
  /** Markierter Zeilenbereich [von, bis] (inklusive). */
  selected?: [number, number]
  /** Zusätzliche Inhalte unter einer Zeile, z. B. Kommentar-Threads. */
  marks?: Record<number, ReactNode>
  className?: string
}) {
  return (
    <div className={cn('font-mono text-[12px] leading-[1.6]', className)}>
      {lines.map((text, i) => {
        const n = start + i
        const sel = selected && n >= selected[0] && n <= selected[1]
        return (
          <div key={n}>
            <div className={cn('flex', sel && 'bg-accent')}>
              <span className={cn('w-10 shrink-0 pr-3 text-right text-muted-foreground select-none', sel && 'text-foreground')}>{n}</span>
              <span className={cn('w-0.5 shrink-0', sel && 'bg-foreground/60')} />
              <span className="pl-3 whitespace-pre">{highlight(text)}</span>
            </div>
            {marks?.[n] && <div className="py-1.5 pr-3 pl-14">{marks[n]}</div>}
          </div>
        )
      })}
    </div>
  )
}

/** Sehr einfache Hervorhebung: Kommentare und Strings gedämpft, Schlüsselwörter fett. */
function highlight(line: string): ReactNode {
  if (/^\s*(\/\/|#)/.test(line)) return <span className="text-muted-foreground italic">{line}</span>
  const parts = line.split(/('[^']*'|"[^"]*"|`[^`]*`)/g)
  return parts.map((p, i) =>
    /^['"`]/.test(p) ? (
      <span key={i} className="text-voice-codex">
        {p}
      </span>
    ) : (
      <span key={i}>
        {p.split(/\b(import|from|export|function|const|return|type|if|await|async|let|new)\b/g).map((w, j) =>
          /^(import|from|export|function|const|return|type|if|await|async|let|new)$/.test(w) ? (
            <span key={j} className="font-semibold text-voice-claude">
              {w}
            </span>
          ) : (
            w
          ),
        )}
      </span>
    ),
  )
}

/**
 * Terminal (xterm-ähnlich). Nutzt die Dunkel-Tokens über die Klasse `dark`, damit Terminals
 * in beiden Themes dunkel sind, ohne Hex-Werte.
 */
export function TermView({ children, className, cursor = true }: { children: ReactNode; className?: string; cursor?: boolean }) {
  return (
    <div className={cn('dark min-h-0 flex-1 overflow-auto bg-sunken p-3 font-mono text-[12px] leading-[1.55] text-foreground', className)}>
      <pre className="whitespace-pre-wrap">{children}</pre>
      {cursor && <span className="inline-block h-[14px] w-[7px] animate-pulse bg-foreground align-text-bottom" />}
    </div>
  )
}
