import type { ReactNode } from 'react'
import { CalendarClock, Clock, GitFork, Webhook } from 'lucide-react'
import { AppLayout } from '@/app/app-layout'
import { cn } from '@/lib/utils'
import { Mark, type MarkKind } from '@/app/kit/harnesses'

export type RunStatus =
  | 'queued'
  | 'dispatched'
  | 'running'
  | 'paused_approval'
  | 'completed'
  | 'failed'
  | 'aborted'
  | 'timed_out'
  | 'budget_exceeded'
  | 'skipped'

const statusMeta: Record<RunStatus, { kind: MarkKind; label: string }> = {
  queued: { kind: 'queued', label: 'wartet' },
  dispatched: { kind: 'run', label: 'wird gestartet' },
  running: { kind: 'run', label: 'läuft' },
  paused_approval: { kind: 'wait', label: 'wartet auf Freigabe' },
  completed: { kind: 'ok', label: 'fertig' },
  failed: { kind: 'deny', label: 'fehlgeschlagen' },
  aborted: { kind: 'off', label: 'abgebrochen' },
  timed_out: { kind: 'deny', label: 'Zeitlimit erreicht' },
  budget_exceeded: { kind: 'deny', label: 'Budget erreicht' },
  skipped: { kind: 'skip', label: 'übersprungen' },
}

/** Lauf-Status als Form + Text. Nur „wartet auf Freigabe“ ist gelb. */
export function RunStatusMark({ status, extra }: { status: RunStatus; extra?: string }) {
  const m = statusMeta[status]
  return <Mark kind={m.kind} label={extra ? `${m.label} · ${extra}` : m.label} />
}

export type TriggerKind = 'schedule' | 'timer' | 'webhook' | 'spawn'
const triggerIcon: Record<TriggerKind, typeof Clock> = { schedule: CalendarClock, timer: Clock, webhook: Webhook, spawn: GitFork }
const triggerLabel: Record<TriggerKind, string> = { schedule: 'Schedule', timer: 'Timer', webhook: 'Webhook', spawn: 'Sub-Agent' }

export function TriggerLabel({ kind, name }: { kind: TriggerKind; name: string }) {
  const Icon = triggerIcon[kind]
  return (
    <span className="inline-flex items-center gap-1.5">
      <Icon className="size-3.5 shrink-0 text-muted-foreground" aria-label={triggerLabel[kind]} />
      <span className="truncate">{name}</span>
    </span>
  )
}

export type AutoTab = 'runs' | 'schedules' | 'triggers'

/** Rahmen der Automationen: Reiter oben, Daemon-Hinweis unten links. */
export function AutomationShell({ tab, children, actions, daemonOff }: { tab: AutoTab; children: ReactNode; actions?: ReactNode; daemonOff?: boolean }) {
  const tabs: { id: AutoTab; label: string }[] = [
    { id: 'runs', label: 'Läufe' },
    { id: 'schedules', label: 'Schedules & Timer' },
    { id: 'triggers', label: 'Webhook-Trigger' },
  ]
  return (
    <AppLayout nav="automations" connection={daemonOff ? 'offline' : 'local'}>
      <div className="flex shrink-0 items-end gap-6 border-b border-border px-6 pt-4">
        <div className="pb-3">
          <h2 className="type-wide text-lg font-[700]">Automationen</h2>
          <p className="text-[12px] text-muted-foreground">Agents, die ohne dich laufen – auf diesem Rechner, solange beton läuft.</p>
        </div>
        <div role="tablist" className="flex gap-1">
          {tabs.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={t.id === tab}
              className={cn('-mb-px border-b-2 px-2 pb-2.5 text-[13px]', t.id === tab ? 'border-foreground font-medium' : 'border-transparent text-muted-foreground hover:text-foreground')}
            >
              {t.label}
            </button>
          ))}
        </div>
        {actions && <div className="ml-auto flex gap-2 pb-3">{actions}</div>}
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">{children}</div>
    </AppLayout>
  )
}
