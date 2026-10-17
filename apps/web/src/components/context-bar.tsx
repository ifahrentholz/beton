import type { Capabilities } from '@beton/sdk'
import { Minimize2 } from 'lucide-react'
import { contextLevel, LEVEL_LABEL, type ContextState } from '@/lib/context'
import { useT, type T } from '@/lib/i18n'
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

/** Warum „Kompaktieren“ gerade nicht geht: Tooltip und, wo nötig, kurzer sichtbarer Hinweis (USE-009 AC3). */
function blockedReason(
  t: T,
  { harness, unsupported, running, stopped, fresh }: { harness: string; unsupported: boolean; running: boolean; stopped: boolean; fresh: boolean },
): { title: string; hint?: string } | undefined {
  if (unsupported) return { title: unsupportedReason(harness), hint: 'nicht verfügbar für diesen Harness' }
  // Ein Tooltip an einem deaktivierten Button erreicht die Tastatur nicht: daher der Hinweis daneben.
  if (running) return { title: t('context.turnRunning'), hint: t('context.turnRunningHint') }
  if (stopped) return { title: t('context.stopped'), hint: t('context.stoppedHint') }
  // Links steht schon „Kontext: noch kein Turn“.
  if (fresh) return { title: t('context.nothing') }
  return undefined
}

export function ContextBar({
  harness,
  capabilities,
  context,
  compacting,
  running,
  stopped = false,
  fresh = false,
  error,
  onCompact,
}: {
  harness: string
  capabilities: Capabilities | undefined
  context: ContextState | undefined
  compacting: boolean
  running: boolean
  /** Session läuft nicht (gestoppt oder fehlgeschlagen): erst fortsetzen. */
  stopped?: boolean
  /** Noch kein Turn: Es gibt keinen Kontext und nichts zu kompaktieren. */
  fresh?: boolean
  /** Grund, warum das letzte Kompaktieren fehlschlug. */
  error?: string | undefined
  onCompact: () => void
}) {
  const t = useT()
  // Nach Turns ohne gemeldeten Kontext (Harness ohne Usage-Reporting) bleibt die Zeile weg.
  if (!context && !fresh) return null
  const unsupported = capabilities?.compaction === 'none'
  const level = context ? contextLevel(context.used, context.window).level : 'plain'
  const blocked = blockedReason(t, { harness, unsupported, running, stopped, fresh: !context })
  return (
    <>
      {error && (
        <p role="alert" className="mx-4 mb-2 flex items-center gap-2 rounded-md border border-deny/40 bg-deny-soft px-3 py-2 text-[13px]" data-testid="compact-error">
          <span aria-hidden className="size-2 shrink-0 rotate-45 bg-deny" />
          {t('context.failed', { reason: error.replace(/\.$/, '') })}
        </p>
      )}
      <div className="flex items-center gap-3 border-t border-border px-4 py-1.5" data-testid="context-bar">
        {context ? (
          <>
            <ContextRing {...context} />
            <span className="hidden text-[11px] text-muted-foreground tabular-nums sm:inline">
              {context.window
                ? `${context.used.toLocaleString('de-DE')} von ${context.window.toLocaleString('de-DE')} Tokens`
                : 'Fenstergröße nicht bekannt (weder Harness noch Katalog)'}
            </span>
          </>
        ) : (
          <span className="text-[12px] text-muted-foreground" data-testid="context-empty">
            {t('context.noTurn')}
          </span>
        )}
        <span className="ml-auto flex items-center gap-1">
          <button
            type="button"
            onClick={onCompact}
            disabled={!!blocked || compacting || !capabilities}
            title={blocked ? blocked.title : 'Verlauf zusammenfassen, um Platz im Kontext zu schaffen (/compact)'}
            className={cn(
              'inline-flex h-7 items-center gap-1 rounded-md border px-2 text-[12px] disabled:cursor-not-allowed disabled:opacity-50',
              level === 'high' || level === 'full' ? 'border-foreground font-semibold' : 'border-border hover:bg-accent',
            )}
          >
            <Minimize2 className="size-3.5" /> {compacting ? 'Kompaktiert …' : 'Kompaktieren'}
          </button>
          {blocked?.hint && !compacting && <span className="text-[11px] text-muted-foreground">{blocked.hint}</span>}
        </span>
      </div>
    </>
  )
}
