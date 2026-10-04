import { cn } from '@/lib/utils'

/** Label-Chip; automatische `beton.`-Labels gestrichelt und gedämpft. */
export function LabelChip({ children }: { children: string }) {
  const auto = children.startsWith('beton.')
  return (
    <code
      className={cn(
        'inline-block rounded-sm border px-1 font-mono text-[11px] leading-[18px] whitespace-nowrap',
        auto ? 'border-dashed border-border text-muted-foreground' : 'border-border bg-card',
      )}
      title={auto ? 'Automatisch ermittelt' : 'Aus host.yaml'}
    >
      {children}
    </code>
  )
}
