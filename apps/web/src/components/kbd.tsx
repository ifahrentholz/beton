import { Fragment, type ReactNode } from 'react'
import { formatChord } from '@/lib/keys'

/** Tastenkürzel als Tasten (Entwurf `inbox-shortcuts`): je Anschlag ein `kbd`, `/` trennt Alternativen. */
export function Keys({ chord, className }: { chord: string; className?: string }) {
  return (
    <span className={className ?? 'inline-flex gap-0.5'}>
      {formatChord(chord).map((p, i) =>
        p === '/' ? (
          <span key={i} aria-hidden className="px-0.5 text-muted-foreground">
            /
          </span>
        ) : (
          <kbd key={i} className="min-w-5 rounded-sm border border-border bg-card px-1 text-center font-mono text-[11px]">
            {p}
          </kbd>
        ),
      )}
    </span>
  )
}

/** Text mit Platzhaltern `{name}`, die durch Elemente ersetzt werden (z. B. Tasten im Satz). */
export function rich(text: string, parts: Record<string, ReactNode>): ReactNode {
  return text.split(/(\{\w+\})/g).map((piece, i) => {
    const m = /^\{(\w+)\}$/.exec(piece)
    return <Fragment key={i}>{m && m[1]! in parts ? parts[m[1]!] : piece}</Fragment>
  })
}
