import type { ReactNode } from 'react'
import { Ban, Check, Circle, Minus, X } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Tag } from '../workspace/parts'

export type CheckStatus = 'queued' | 'running' | 'success' | 'failure' | 'cancelled' | 'skipped'

const label: Record<CheckStatus, string> = {
  queued: 'wartet',
  running: 'läuft',
  success: 'erfolgreich',
  failure: 'fehlgeschlagen',
  cancelled: 'abgebrochen',
  skipped: 'übersprungen',
}

/** Status eines Checks: Form + Text, nicht nur Farbe. */
export function CheckMark({ status }: { status: CheckStatus }) {
  return (
    <span className="inline-flex size-4 shrink-0 items-center justify-center" title={label[status]}>
      {status === 'success' && <Check className="size-3.5 text-ok" />}
      {status === 'failure' && <X className="size-3.5 text-deny" />}
      {status === 'running' && <span className="size-2 animate-pulse rounded-full bg-ok" />}
      {status === 'queued' && <Circle className="size-3 text-muted-foreground" />}
      {status === 'cancelled' && <Ban className="size-3 text-muted-foreground" />}
      {status === 'skipped' && <Minus className="size-3 text-muted-foreground" />}
      <span className="sr-only">{label[status]}</span>
    </span>
  )
}

export function CheckRow({ name, status, detail, action, children }: { name: string; status: CheckStatus; detail?: string; action?: ReactNode; children?: ReactNode }) {
  return (
    <div className={cn('border-b border-border', status === 'failure' && 'bg-deny-soft/50')}>
      <div className="flex items-center gap-2 px-3 py-1.5 text-[13px]">
        <CheckMark status={status} />
        <span className="min-w-0 flex-1 truncate font-mono text-[12px]">{name}</span>
        <span className="shrink-0 text-[11px] text-muted-foreground">
          {label[status]}
          {detail && ` · ${detail}`}
        </span>
        {action}
      </div>
      {children}
    </div>
  )
}

export type CrState = 'draft' | 'open' | 'merged' | 'closed'
const crLabel: Record<CrState, string> = { draft: 'Entwurf', open: 'Offen', merged: 'Gemergt', closed: 'Geschlossen' }

export function CrStateTag({ state }: { state: CrState }) {
  return <Tag tone={state === 'merged' ? 'ok' : state === 'closed' ? 'muted' : 'neutral'}>{crLabel[state]}</Tag>
}

export type Origin = 'created' | 'attached' | 'inferred'
const originLabel: Record<Origin, string> = {
  created: 'in dieser Session erstellt',
  attached: 'von dir verknüpft',
  inferred: 'erkannt: gleicher Branch',
}

export function OriginTag({ origin }: { origin: Origin }) {
  return <span className="text-[11px] text-muted-foreground">{originLabel[origin]}</span>
}
