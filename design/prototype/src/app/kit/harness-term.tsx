import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/**
 * Terminal-Bausteine der Gruppen „Einrichtung & Harnesses“, „Agents“ und „Automationen“. Der Inhalt läuft unter `.dark`, damit Tokens (text-ok, text-deny …) auf dem
 * dunklen Terminal-Hintergrund die Nacht-Werte bekommen; keine Hex-Farben nötig.
 */
export function Term({ children }: { children: ReactNode }) {
  return <div className="dark space-y-0.5 whitespace-pre-wrap text-foreground">{children}</div>
}

export function Cmd({ children }: { children: ReactNode }) {
  return (
    <div className="mt-3 first:mt-0">
      <span className="text-muted-foreground">~/code/shop-frontend </span>
      <span className="text-voice-codex">$</span> {children}
    </div>
  )
}

export function Ln({ children, tone, className }: { children?: ReactNode; tone?: 'ok' | 'deny' | 'muted' | 'signal' | 'add' | 'del'; className?: string }) {
  return (
    <div
      className={cn(
        tone === 'ok' && 'text-ok',
        tone === 'deny' && 'text-deny',
        tone === 'muted' && 'text-muted-foreground',
        tone === 'signal' && 'text-signal',
        tone === 'add' && 'text-ok',
        tone === 'del' && 'text-deny',
        className,
      )}
    >
      {children ?? ' '}
    </div>
  )
}
