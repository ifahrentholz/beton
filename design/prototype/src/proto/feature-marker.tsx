import { createContext, useContext, type ReactNode } from 'react'
import { featureById, specUrl } from '@/catalog/types'
import { cn } from '@/lib/utils'

/** Steuert, ob Feature-Markierungen im Prototyp sichtbar sind. */
export const FeatureOverlayContext = createContext(false)

type Props = {
  /** Eine oder mehrere Feature-IDs, z. B. `"WEB-002"` oder `["SES-004", "WEB-005"]`. */
  id: string | string[]
  children: ReactNode
  className?: string
  /** Badge-Position, falls oben links schon etwas liegt. */
  badge?: 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right'
  as?: 'div' | 'span' | 'section'
}

/**
 * Markiert den Bereich, der ein Feature zeigt. Mit eingeschaltetem Overlay
 * („Feature-IDs“ in der Kopfzeile) erscheinen Umrandung und ID-Badges mit Link zur Spec.
 */
export function F({ id, children, className, badge = 'top-left', as = 'div' }: Props) {
  const show = useContext(FeatureOverlayContext)
  const ids = Array.isArray(id) ? id : [id]
  const Tag = as
  return (
    <Tag
      data-features={ids.join(' ')}
      className={cn(
        'relative',
        show && 'outline-2 outline-dashed outline-offset-2 outline-signal/80',
        className,
      )}
    >
      {children}
      {show && (
        <span
          className={cn(
            'pointer-events-auto absolute z-50 flex flex-wrap gap-0.5',
            badge === 'top-left' && '-top-2.5 left-1',
            badge === 'top-right' && '-top-2.5 right-1',
            badge === 'bottom-left' && '-bottom-2.5 left-1',
            badge === 'bottom-right' && '-bottom-2.5 right-1',
          )}
        >
          {ids.map((fid) => {
            const f = featureById.get(fid)
            return (
              <a
                key={fid}
                href={f ? specUrl(f) : undefined}
                target="_blank"
                rel="noreferrer"
                title={f ? `${fid} — ${f.title}\n\n${f.summary}` : `${fid} (nicht in der Spec!)`}
                className={cn(
                  'rounded-sm px-1 py-px font-mono text-[10px] leading-tight font-medium shadow-sm',
                  f ? 'bg-signal text-signal-foreground' : 'bg-deny text-white',
                )}
              >
                {fid}
              </a>
            )
          })}
        </span>
      )}
    </Tag>
  )
}
