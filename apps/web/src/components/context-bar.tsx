import type { Capabilities } from '@beton/sdk'
import { Minimize2 } from 'lucide-react'
import { contextLevel, LEVEL_LABEL, type ContextState } from '@/lib/context'
import { cn } from '@/lib/utils'
import { harnessName } from './harness'

/*
 * Kontext und Compaction über der Eingabe (USE-008, USE-009; Entwurf `usage-context`): Ring mit
 * Prozentwert und Warnstufen 70/85/95 %, daneben die absoluten Tokens und rechts „Kompaktieren“.
 */

/** Ring mit Prozentwert; ohne bekanntes Fenster nur „150 k Tokens“ (USE-008 AC2). */
export function ContextRing({ used, window, source }: ContextState) {
  const { pct, level } = contextLevel(used, window)
  const r = 9
  const c = 2 * Math.PI * r
  const label = LEVEL_LABEL[level]
  const quelle = source === 'estimated' ? 'geschätzt' : 'Harness'
  return (
    <span
      className="inline-flex items-center gap-2"
      data-testid="context-ring"
      data-level={level}
      title={
        window
          ? `${used.toLocaleString('de-DE')} / ${window.toLocaleString('de-DE')} Tokens · Quelle: ${quelle}`
          : `${used.toLocaleString('de-DE')} Tokens · Fenstergröße unbekannt`
      }
    >
      {pct !== null && (
        <svg width="24" height="24" viewBox="0 0 24 24" aria-hidden className="-rotate-90">
          <circle cx="12" cy="12" r={r} fill="none" stroke="var(--muted)" strokeWidth="3" />
          <circle
            cx="12"
            cy="12"
            r={r}
            fill="none"
            stroke={level === 'full' ? 'var(--deny)' : level === 'warn' || level === 'high' ? 'var(--warn)' : 'var(--muted-foreground)'}
            strokeWidth={level === 'ok' ? 3 : 4}
            strokeDasharray={`${(c * Math.min(pct, 100)) / 100} ${c}`}
          />
        </svg>
      )}
      <span className={cn('text-[12px] tabular-nums', level === 'full' && 'font-semibold text-deny', (level === 'warn' || level === 'high') && 'font-semibold text-warn')}>
        {pct !== null ? `${pct} % Kontext` : `${Math.round(used / 1000)} k Tokens`}
      </span>
      {label && <span className={cn('text-[11px]', level === 'full' ? 'text-deny' : 'text-warn')}>· {label}</span>}
    </span>
  )
}

/** Grund, warum ein Harness nicht kompaktieren kann (USE-009 AC3, HAR-022 AC2). */
export function unsupportedReason(harness: string): string {
  return `${harnessName(harness)} kann nicht kompaktieren. Starte bei Bedarf eine neue Session.`
}

export function ContextBar({
  harness,
  capabilities,
  context,
  compacting,
  running,
  onCompact,
}: {
  harness: string
  capabilities: Capabilities | undefined
  context: ContextState | undefined
  compacting: boolean
  running: boolean
  onCompact: () => void
}) {
  // Vor dem ersten gemeldeten Kontext steht der Entwurf noch aus; bis dahin keine Zeile.
  if (!context) return null
  const unsupported = capabilities?.compaction === 'none'
  const { level } = contextLevel(context.used, context.window)
  return (
    <div className="flex items-center gap-3 border-t border-border px-4 py-1.5" data-testid="context-bar">
      <ContextRing {...context} />
      <span className="hidden text-[11px] text-muted-foreground tabular-nums sm:inline">
        {context.window
          ? `${context.used.toLocaleString('de-DE')} von ${context.window.toLocaleString('de-DE')} Tokens`
          : 'Fenstergröße nicht bekannt (weder Harness noch Katalog)'}
      </span>
      <span className="ml-auto flex items-center gap-1">
        <button
          type="button"
          onClick={onCompact}
          disabled={unsupported || compacting || running || !capabilities}
          title={unsupported ? unsupportedReason(harness) : 'Verlauf zusammenfassen, um Platz im Kontext zu schaffen (/compact)'}
          className={cn(
            'inline-flex h-7 items-center gap-1 rounded-md border px-2 text-[12px] disabled:cursor-not-allowed disabled:opacity-50',
            level === 'high' || level === 'full' ? 'border-foreground font-semibold' : 'border-border hover:bg-accent',
          )}
        >
          <Minimize2 className="size-3.5" /> {compacting ? 'Kompaktiert …' : 'Kompaktieren'}
        </button>
        {unsupported && <span className="text-[11px] text-muted-foreground">nicht verfügbar für diesen Harness</span>}
      </span>
    </div>
  )
}
